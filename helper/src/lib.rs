//! The CAM package's native helper: what it does, apart from reading and writing
//! (`main.rs`).
//!
//! printCAD runs it from a job of the package with a [`Request`] on its standard
//! input, and kills it when the job is stopped. It reads each model's solid from
//! ogeom's native format, runs the setup through camcraft's one engine path
//! (`camcraft_session::run_setup`, which the camcraft app and command line run
//! too) on every core, and writes a [`Response`] on its standard output: the
//! toolpaths for drawing, the checks, and a program for each operation.
//!
//! A setup that cannot run is answered with [`Response::Refused`] and a reason a
//! person can read; only a broken request or a crash ends it with a failure.

use std::sync::Arc;
use std::time::Instant;

use cam_protocol::{
    Cutting, Cylinder, Finding, HeightPlane, IDENTITY, Layout, Level, Machining, Mark, Material,
    OperationRun, Program, Request, Run, Stroke, ToolUse, VERSION,
};
use camcraft_core::job::{HeightContext, Operation, Outline, Programs};
use camcraft_core::path::toolpath::{MoveRole, Toolpath};
use camcraft_core::run::Made;
use camcraft_core::run::{PLUNGE_RATIO, Stage};
use camcraft_core::setup::frame::Frame;
use camcraft_core::setup::{Bodies, Prepared, bounds_all, features, prepare};
use camcraft_geom::mesh::Aabb;
use camcraft_import::native::load_native;
use camcraft_post::op::{Op, lower_counted, tilt_notes};
use camcraft_post::profile::{Profile, Rapid};
use camcraft_post::render::{Title, render_program};
use camcraft_session::checks::Level as CheckLevel;
use camcraft_session::{
    Model, Models, Ran, custom_gcode, load_profile, next_tools, parse_job, program_spec,
    run_setup_planned, setup_program, setup_program_name, travel_check,
};
use camcraft_sim::stock::{Axis, Block};
use glam::{DAffine3, DMat4, DVec2, DVec3};

/// How many columns a side the drawn material has, at most.
const MATERIAL_COLUMNS: usize = 100;

/// The largest file [`read`] reads: a tool library or a machine profile is
/// far smaller.
const MOST_READ: u64 = 16 << 20;

/// Whether `request` was written for this helper.
fn speaks(request: &Request) -> Result<(), String> {
    if request.version != VERSION {
        return Err(format!(
            "the package speaks version {} to its helper, which speaks {VERSION}: \
             reinstall the package",
            request.version
        ));
    }
    Ok(())
}

/// The text of the file at `path`, for `request`: a tool library or a machine
/// profile the package was pointed at.
pub fn read(request: &Request, path: &str) -> Result<String, String> {
    speaks(request)?;
    let path = std::path::Path::new(path.trim());
    let size = std::fs::metadata(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?
        .len();
    if size > MOST_READ {
        return Err(format!(
            "{} is {} MB, too big for a tool library or a profile",
            path.display(),
            size >> 20
        ));
    }
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// Run `request`'s setup, or say why it cannot run.
pub fn answer(request: &Request) -> Result<Run, String> {
    answer_reporting(request, &mut |_, _| {})
}

/// [`answer`], telling `progress` how far it is as `(done, total)` steps: the
/// solids read, laying the setup out, each operation, verifying, checking,
/// and posting.
pub fn answer_reporting(
    request: &Request,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Run, String> {
    execute(request, progress).map(|done| done.run)
}

/// A setup run, with what the answer was made from: the job, camcraft's
/// run, and each operation as drawn, which a playback session keeps to
/// sweep the run again.
pub(crate) struct Executed {
    pub run: Run,
    pub job: camcraft_core::job::Job,
    pub ran: Ran,
    pub draws: Vec<Drawn>,
}

/// [`answer_reporting`], keeping what the answer was made from.
pub(crate) fn execute(
    request: &Request,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Executed, String> {
    speaks(request)?;
    let started = Instant::now();
    let job = parse_job(&request.job, "the setup").map_err(|e| e.to_string())?;
    let setup = job.setups.first().ok_or("the job has no setup")?;
    let ops = setup.operations.len() as u64;
    // Reading the solids, laying out, each operation, verifying, checking,
    // posting.
    let total = ops + 5;
    progress(0, total);

    // A STEP file is tessellated to a quarter of the machining tolerance; a
    // native shape is the same B-rep, so it gets the same.
    let tessellation = job.tolerance / 4.0;
    let mut loaded = Vec::new();
    for def in &job.models {
        let shape = request
            .shapes
            .get(&def.name)
            .ok_or_else(|| format!("no shape came for '{}'", def.name))?;
        let (body, report) =
            load_native(&shape.text, tessellation).map_err(|e| format!("{}: {e}", def.name))?;
        let body = if shape.placement == IDENTITY {
            body
        } else {
            body.placed(affine(&shape.placement))
                .map_err(|e| format!("{}: {e}", def.name))?
        };
        let bounds = body
            .mesh()
            .bounds()
            .ok_or_else(|| format!("{} is empty", def.name))?;
        let mut warnings = Vec::new();
        if report.degenerate > 0 {
            warnings.push(format!(
                "{} degenerate triangle(s) from tessellation",
                report.degenerate
            ));
        }
        loaded.push((
            def.name.as_str(),
            Model {
                file: Arc::new(body.clone()),
                body,
                bounds,
                path: def.name.clone(),
                units: "mm".into(),
                notes: vec![format!(
                    "{}: {} face(s), {} triangles at {} mm",
                    def.name, report.faces, report.triangles, tessellation
                )],
                warnings,
            },
        ));
    }
    let models: Models<'_> = loaded.iter().map(|(n, m)| (*n, m)).collect();
    let ran = run_setup_planned(&job, 0, &models, &[], &mut |stage| {
        let done = match stage {
            Stage::Preparing => 1,
            Stage::Operation(i) => 2 + i as u64,
            Stage::Verifying => ops + 2,
            Stage::Checking => ops + 3,
        };
        progress(done, total);
    })
    .map_err(|e| e.to_string())?;
    progress(ops + 4, total);
    let profile = match &request.post {
        Some(text) => {
            Profile::from_toml(text).map_err(|e| format!("the post {}: {e}", setup.post))?
        }
        None => load_profile(&setup.post, None).map_err(|e| e.to_string())?,
    };

    let frame = ran.result.frame;
    let tolerance = job.tolerance.max(0.02);
    let stock_box = bounds_all(&ran.result.stocks);
    let bodies: Bodies<'_> = loaded.iter().map(|(n, m)| (*n, &m.body)).collect();
    // Laid out again only for a height measured from the picks.
    let mut prepared: Option<Option<Prepared>> = None;
    let mut operations = Vec::new();
    let mut draws = Vec::new();
    // Programs the post cannot write, as a tap for a controller without the
    // tapping cycle: each a failed check, so the setup is not posted, while
    // its run is still shown.
    let mut refused = Vec::new();
    // What the travel could not be checked for, and the operations it
    // could not be for.
    let mut unchecked: std::collections::BTreeMap<String, Vec<usize>> = Default::default();
    let mut with_paths = 0;
    // Machining seconds before the operation under way.
    let mut elapsed = 0.0;
    let custom = custom_gcode(setup, &ran.result.operations);
    let next = next_tools(&ran.result.operations);
    let date = today();
    // One program for the setup, or one an operation, each numbered on
    // from the setup's.
    let one_each = setup.programs == Programs::Operation;
    let mut numbered = 0u32;
    // How many times the programs change the tool.
    let mut tool_changes = 0usize;
    let rapid = profile.rapids();
    // What the programs' headers say of a tilt, once.
    let mut tilt: Option<Vec<String>> = None;
    for (k, op) in ran.result.operations.iter().enumerate() {
        let heights = match (setup.operations.get(k), stock_box) {
            _ if op.made == Made::Text => Vec::new(),
            (Some(spec), Some(stock)) => {
                let mut ctx = ran.result.heights;
                if spec.heights.measured_from_selection() || down_to_picks(spec) {
                    let laid = prepared.get_or_insert_with(|| prepare(&job, 0, &bodies).ok());
                    ctx.picked = laid.as_ref().and_then(|p| {
                        features::picks(&spec.selection, p, &bodies, job.tolerance)
                            .ok()
                            .flatten()
                    });
                }
                planes(spec, &ctx, stock, &frame)
            }
            _ => Vec::new(),
        };
        let made = match op.made {
            Made::Generated => "generated",
            Made::Kept => "kept",
            Made::Skipped => "skipped",
            Made::Disabled => "disabled",
            Made::Text => "custom-gcode",
        };
        let mut drawn = strokes(&op.path, &frame, tolerance, request.max_points, &rapid);
        if tilt.is_none() && op.made.has_paths() {
            tilt = program_spec(op, setup.wcs, &frame, &profile)
                .ok()
                .map(|spec| tilt_notes(&spec));
        }
        let program = if one_each && op.made.has_paths() {
            let name = camcraft_session::program_name(&setup.name, k, &op.name);
            // As `camcraft_session::program` posts it, keeping which move
            // each line is of.
            let posted = program_spec(op, setup.wcs, &frame, &profile).and_then(|mut spec| {
                if let Some((before, after)) = custom.get(k) {
                    spec.before.clone_from(before);
                    spec.after.clone_from(after);
                }
                spec.next_tool = next.get(k).copied().flatten();
                spec.stock = stock_box.map(|b| (b.min, b.max));
                lower_counted(&op.path, &spec)
            });
            match posted {
                Ok((ops, made)) => {
                    tool_changes += changes(&ops);
                    let title = Title {
                        name: &name,
                        number: setup.number.saturating_add(numbered),
                        date: Some(&date),
                    };
                    numbered += 1;
                    let (text, report) = render_program(&ops, &profile, &title);
                    Some(Program {
                        name,
                        extension: profile.extension.clone(),
                        post: profile.name.clone(),
                        lines: usize::try_from(report.lines).unwrap_or(usize::MAX),
                        moves: line_moves(&made, &report.op_lines, report.lines),
                        operations: Vec::new(),
                        text,
                    })
                }
                Err(e) => {
                    refused.push(Finding {
                        level: Level::Fail,
                        code: "post".into(),
                        text: format!("post {} {}: {e}", profile.name, op.name),
                        operation: Some(k),
                        at_move: None,
                    });
                    None
                }
            }
        } else {
            None
        };
        // Past the machine's travel, the program stops at its limits part
        // way through, or crashes into them: a failed check, as the
        // simulator's are, which Save anyway still gets past. What could
        // not be checked is said, not passed.
        if op.made.has_paths() {
            let travel = travel_check(op, setup, &profile);
            refused.extend(travel.past.into_iter().map(|why| Finding {
                level: Level::Fail,
                code: "travel".into(),
                text: format!("travel {}: {why}", op.name),
                operation: Some(k),
                at_move: None,
            }));
            for why in travel.unchecked {
                unchecked.entry(why).or_default().push(k);
            }
            with_paths += 1;
        }
        let summary = if op.made == Made::Text {
            let lines = setup
                .operations
                .get(k)
                .map_or(0, |o| o.gcode.lines().count());
            format!("{lines} line(s) of custom G-code")
        } else {
            format!(
                "{} passes, {} links stayed down, removed {:.0} mm³",
                op.passes, op.links.stayed_down, op.sim.removed_mm3
            )
        };
        operations.push(OperationRun {
            name: op.name.clone(),
            made: made.into(),
            summary,
            warnings: op
                .warnings
                .iter()
                .cloned()
                .chain(
                    op.made
                        .has_paths()
                        .then(|| profile.spindle_problem(op.rpm))
                        .flatten(),
                )
                .collect(),
            strokes: std::mem::take(&mut drawn.strokes),
            points: drawn.points,
            cut_mm: drawn.cut,
            rapid_mm: drawn.rapid,
            program,
            heights,
            seconds: drawn.times.last().copied().unwrap_or(0.0),
            cutting: setup
                .operations
                .get(k)
                .filter(|_| op.made.has_paths())
                .and_then(|spec| cutting(spec, op, &drawn)),
            marks: marks(&ran, k, op, &drawn, elapsed),
            outlines: setup.operations.get(k).and_then(|spec| {
                let uses = spec.strategy.uses();
                if !(uses.boundary || uses.outline || uses.traces || uses.carves) {
                    return None;
                }
                let laid = prepared
                    .get_or_insert_with(|| prepare(&job, 0, &bodies).ok())
                    .as_ref()?;
                outlines::Ground {
                    job: &job,
                    prepared: laid,
                    bodies: &bodies,
                    frame: &frame,
                    top: ran.result.heights.stock_top,
                }
                .outlines(spec)
            }),
        });
        elapsed += drawn.times.last().copied().unwrap_or(0.0);
        draws.push(drawn);
    }

    // The setup's program, every operation in it.
    let program = if one_each || !ran.result.operations.iter().any(|op| op.made.has_paths()) {
        None
    } else {
        let name = setup_program_name(&setup.name);
        let title = Title {
            name: &name,
            number: setup.number,
            date: Some(&date),
        };
        match setup_program(setup, &ran.result, &profile, &title) {
            Ok(p) => {
                tool_changes = p.tool_changes;
                let (operations, moves) = line_moments(&p.made, &p.report.op_lines, p.report.lines);
                Some(Program {
                    name,
                    extension: profile.extension.clone(),
                    post: profile.name.clone(),
                    lines: usize::try_from(p.report.lines).unwrap_or(usize::MAX),
                    moves,
                    operations,
                    text: p.text,
                })
            }
            Err((k, e)) => {
                let op = ran.result.operations.get(k).map_or("", |o| o.name.as_str());
                refused.push(Finding {
                    level: Level::Fail,
                    code: "post".into(),
                    text: format!("post {} {op}: {e}", profile.name),
                    operation: Some(k),
                    at_move: None,
                });
                None
            }
        }
    };

    // A program that could not be posted changes the tool, at least,
    // wherever the next operation's is another.
    if tool_changes == 0 {
        tool_changes = changes_by_tool(&ran.result.operations);
    }
    let machining = Machining {
        moves: elapsed,
        tool_changes: u32::try_from(tool_changes).unwrap_or(u32::MAX),
        tool_change: profile.tool_change_seconds,
        rapid: [rapid.x, rapid.y, rapid.z],
        rapid_given: profile.rapid.is_some(),
    };
    let tools = tool_uses(&job, setup, &ran.result.operations, &draws);
    let layout = stock_box.map(|b| Layout {
        stock: [to_array(b.min), to_array(b.max)],
        part: part_box(&loaded, &setup.models, &frame).unwrap_or([[0.0; 3]; 2]),
        tool_axis: to_array(frame.turn.tool_axis()),
        tilt: tilt.unwrap_or_default(),
        post: profile.name.clone(),
        date: date.clone(),
    });
    let stock = stock_box
        .map(|b| corners(&frame, b))
        .unwrap_or([[0.0; 3]; 8]);
    progress(total, total);
    let run = Run {
        version: VERSION,
        passed: ran.passed() && refused.is_empty(),
        findings: ranked(
            refused
                .into_iter()
                .chain(travel_notes(unchecked, with_paths, &ran))
                .chain(findings(&ran))
                .collect(),
        ),
        notes: loaded
            .iter()
            .flat_map(|(_, m)| m.notes.iter().cloned())
            .chain(ran.notes.iter().cloned())
            .collect(),
        warnings: ran.warnings.clone(),
        stock,
        cylinder: cylinder(&frame, &ran.result.stocks),
        origin: to_array(frame.to_model(DVec3::ZERO)),
        operations,
        material: material(&ran),
        playback: playback::playback(&job, &ran, &draws),
        program,
        seconds: started.elapsed().as_secs_f64(),
        timing: cam_protocol::timing(draws.iter().map(|d| d.times.as_slice())),
        machining,
        tools,
        layout,
    };
    Ok(Executed {
        run,
        job,
        ran,
        draws,
    })
}

/// How many times `ops`, a program lowered, changes the tool.
fn changes(ops: &[Op]) -> usize {
    ops.iter()
        .filter(|op| matches!(op, Op::ToolChange { .. }))
        .count()
}

/// How many times a setup's operations change the tool, the first
/// included, going by their tools alone: for a setup whose program was not
/// posted.
pub(crate) fn changes_by_tool(ops: &[camcraft_core::run::OperationResult]) -> usize {
    let mut tools: Vec<u32> = ops
        .iter()
        .filter(|op| op.made.has_paths())
        .map(|op| op.tool_number)
        .collect();
    tools.dedup();
    tools.len()
}

/// What the setup sheet says of how operation `spec` cut, as run (`op`).
fn cutting(
    spec: &Operation,
    op: &camcraft_core::run::OperationResult,
    drawn: &Drawn,
) -> Option<Cutting> {
    let uses = spec.strategy.uses();
    let leave = spec.stock_to_leave();
    Some(Cutting {
        strategy: op.strategy.to_string(),
        tool: spec.tool.clone()?,
        tool_number: op.tool_number,
        rpm: op.rpm,
        feed: spec.passes.feed,
        plunge_feed: spec.linking.plunge_feed,
        fastest_feed: drawn.fastest,
        stepover: uses.stepover.then_some(spec.passes.stepover),
        stepdown: spec.stepdown(),
        stock_to_leave: (uses.stock_to_leave != camcraft_core::job::Leave::None)
            .then_some([leave.radial, leave.axial]),
        z: drawn.z,
        notes: strategy_notes(spec),
    })
}

/// What the setup sheet says of an operation's own settings, beyond its
/// feeds and steps: a bore's pitch, a thread's pitch and hand, a tap, a
/// peck, rest machining and a pattern's copies.
fn strategy_notes(spec: &Operation) -> Vec<String> {
    use camcraft_core::job::{StrategySpec, ThreadHand};
    let mm = |v: f64| format!("{}", (v * 1000.0).round() / 1000.0);
    let hand = |h: ThreadHand| match h {
        ThreadHand::Right => "right-hand",
        ThreadHand::Left => "left-hand",
    };
    let mut out = Vec::new();
    match &spec.strategy {
        StrategySpec::Bore { pitch, .. } => out.push(format!("helix {} mm a turn", mm(*pitch))),
        StrategySpec::Thread {
            pitch,
            hand: h,
            diameter,
            ..
        } => out.push(match diameter {
            Some(d) => format!("thread {} mm pitch, {}, Ø{}", mm(*pitch), hand(*h), mm(*d)),
            None => format!("thread {} mm pitch, {}", mm(*pitch), hand(*h)),
        }),
        StrategySpec::Circular { boss_width, .. } if *boss_width > 0.0 => {
            out.push(format!("boss material {} mm wide", mm(*boss_width)));
        }
        StrategySpec::Drill {
            pitch: Some(pitch),
            hand: h,
            ..
        } => out.push(format!("tapped {} mm pitch, {}", mm(*pitch), hand(*h))),
        StrategySpec::Drill {
            peck: Some(peck),
            chip_break,
            ..
        } => out.push(match chip_break {
            Some(b) => format!("pecks of {} mm, backing off {}", mm(*peck), mm(*b)),
            None => format!("pecks of {} mm", mm(*peck)),
        }),
        StrategySpec::Adaptive {
            rest: Some(rest), ..
        } => out.push(if rest.adjustment > 0.0 {
            format!(
                "rest machining: what the operations before left, past {} mm",
                mm(rest.adjustment)
            )
        } else {
            "rest machining: what the operations before left".into()
        }),
        _ => {}
    }
    if let Some(relief) = spec.relief() {
        use camcraft_core::job::ReliefStyle;
        let style = match relief.style {
            ReliefStyle::Dogbone => "dogbones",
            ReliefStyle::TBoneX => "T-bones along X",
            ReliefStyle::TBoneY => "T-bones along Y",
            ReliefStyle::TBoneLong => "T-bones along the longer wall",
            ReliefStyle::TBoneShort => "T-bones along the shorter wall",
        };
        out.push(match relief.sharper_than {
            Some(a) => format!("corners relieved with {style}, sharper than {}°", mm(a)),
            None => format!("corners relieved with {style}"),
        });
    }
    let copies = spec.transform.copies();
    if copies > 1 {
        out.push(format!("{copies} copies, by its pattern"));
    }
    out
}

/// The tools a setup's operations cut with, in the order they first cut,
/// each with what it does.
fn tool_uses(
    job: &camcraft_core::job::Job,
    setup: &camcraft_core::job::Setup,
    ops: &[camcraft_core::run::OperationResult],
    draws: &[Drawn],
) -> Vec<ToolUse> {
    let mut out: Vec<ToolUse> = Vec::new();
    for (k, (op, drawn)) in ops.iter().zip(draws).enumerate() {
        if !op.made.has_paths() {
            continue;
        }
        let Some(spec) = setup.operations.get(k) else {
            continue;
        };
        let (Some(name), Some(tool)) = (spec.tool.as_deref(), job.tool_for(spec)) else {
            continue;
        };
        let at = match out.iter().position(|t| t.name == name) {
            Some(at) => at,
            None => {
                out.push(ToolUse {
                    name: name.to_string(),
                    number: tool.number,
                    kind: tool_kind(tool.kind).into(),
                    diameter: tool.diameter,
                    corner: tool.corner,
                    angle: tool.angle.or(match tool.kind {
                        camcraft_core::job::ToolKind::Drill => {
                            Some(camcraft_core::job::DRILL_POINT)
                        }
                        _ => None,
                    }),
                    pitch_range: tool.pitch_range,
                    flutes: tool.flutes,
                    flute_length: tool.flute_length,
                    stickout: tool.stickout,
                    overall_length: tool.overall_length,
                    holder: tool.holder.as_ref().map(holder_words),
                    comment: tool.comment.clone(),
                    ..ToolUse::default()
                });
                out.len() - 1
            }
        };
        let Some(t) = out.get_mut(at) else {
            continue;
        };
        t.operations.push(k);
        if let Some([low, _]) = drawn.z {
            t.lowest = Some(t.lowest.map_or(low, |l| l.min(low)));
        }
        t.fastest_feed = t.fastest_feed.max(drawn.fastest);
        t.fastest_rpm = t.fastest_rpm.max(op.rpm);
        t.seconds += drawn.times.last().copied().unwrap_or(0.0);
    }
    out
}

/// A tool's kind, as a job file names it.
fn tool_kind(kind: camcraft_core::job::ToolKind) -> &'static str {
    use camcraft_core::job::ToolKind;
    match kind {
        ToolKind::Flat => "flat",
        ToolKind::Ball => "ball",
        ToolKind::Bullnose => "bullnose",
        ToolKind::Cone => "cone",
        ToolKind::Drill => "drill",
        ToolKind::ThreadMill => "thread-mill",
    }
}

/// The box round the bodies `names`, as the setup sees them: their lowest
/// and highest corners from the work zero along its axes.
fn part_box(loaded: &[(&str, Model)], names: &[String], frame: &Frame) -> Option<[[f64; 3]; 2]> {
    let mut low = DVec3::splat(f64::INFINITY);
    let mut high = DVec3::splat(f64::NEG_INFINITY);
    for (_, model) in loaded.iter().filter(|(n, _)| names.iter().any(|m| m == n)) {
        for &v in model.body.mesh().vertices() {
            let p = frame.to_setup(v);
            low = low.min(p);
            high = high.max(p);
        }
    }
    low.cmple(high)
        .all()
        .then(|| [to_array(low), to_array(high)])
}

/// Where operation `k`'s checks found something at a move, and its steep
/// descents deeper than it may plunge, as camcraft's app marks its timeline:
/// in order, a failure kept over a warning at the same place. `start` is
/// when the operation starts, in the setup's machining seconds.
fn marks(
    ran: &Ran,
    k: usize,
    op: &camcraft_core::run::OperationResult,
    drawn: &Drawn,
    start: f64,
) -> Vec<Mark> {
    let at = |moves: usize, fail: bool| Mark {
        moves,
        seconds: start
            + moves
                .checked_sub(1)
                .and_then(|i| drawn.times.get(i))
                .copied()
                .unwrap_or(0.0),
        fail,
    };
    let mut out: Vec<Mark> = ran
        .checks
        .groups
        .iter()
        .filter(|g| g.op == Some(k))
        .flat_map(|g| g.findings.iter())
        .filter_map(|f| {
            let fail = match f.level {
                CheckLevel::Fail => true,
                CheckLevel::Warn => false,
                CheckLevel::Note | CheckLevel::Pass => return None,
            };
            Some(at(f.at_move?, fail))
        })
        .collect();
    let moves = op.path.moves();
    let first = moves.first().map_or(DVec3::ZERO, |m| m.to);
    out.extend(
        op.path
            .steep_descents(first, PLUNGE_RATIO)
            .into_iter()
            .filter(|d| d.depth > op.max_cut_step)
            .map(|d| at(d.last, false)),
    );
    out.sort_by_key(|m| (m.moves, !m.fail));
    out.dedup_by_key(|m| m.moves);
    out
}

/// A holder as the setup sheet lists it: its name, and its segments from
/// the tool up, each its diameter (foot to top, for a cone) by its length,
/// as the shank and holder checks take it.
fn holder_words(h: &camcraft_core::job::Holder) -> String {
    let mm = |v: f64| {
        let s = format!("{v:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    let segments: Vec<String> = h
        .segments
        .iter()
        .map(|s| {
            if (s.top() - s.diameter).abs() > 1e-9 {
                format!("Ø{}–{} × {}", mm(s.diameter), mm(s.top()), mm(s.length))
            } else {
                format!("Ø{} × {}", mm(s.diameter), mm(s.length))
            }
        })
        .collect();
    let shape = format!("{} mm", segments.join(", "));
    if h.name.is_empty() {
        shape
    } else if segments.is_empty() {
        h.name.clone()
    } else {
        format!("{}: {shape}", h.name)
    }
}

/// The checks' findings, failures first, then warnings, notes and passes.
fn findings(ran: &Ran) -> Vec<Finding> {
    let out: Vec<Finding> = ran
        .checks
        .groups
        .iter()
        .flat_map(|group| {
            group.findings.iter().map(move |f| Finding {
                level: match f.level {
                    CheckLevel::Fail => Level::Fail,
                    CheckLevel::Warn => Level::Warn,
                    CheckLevel::Note => Level::Note,
                    CheckLevel::Pass => Level::Pass,
                },
                code: f.code.to_string(),
                text: f.line(group),
                operation: group.op,
                at_move: f.at_move,
            })
        })
        .collect();
    ranked(out)
}

/// `findings`, failures first, then warnings, notes and passes, each kept in
/// its order.
fn ranked(mut findings: Vec<Finding>) -> Vec<Finding> {
    let rank = |l: Level| match l {
        Level::Fail => 0,
        Level::Warn => 1,
        Level::Note => 2,
        Level::Pass => 3,
    };
    findings.sort_by_key(|f| rank(f.level));
    findings
}

/// A note for each thing the travel could not be checked for: once for the
/// setup when it holds for all `with_paths` of its operations that move,
/// else once for each it holds for.
fn travel_notes(
    unchecked: std::collections::BTreeMap<String, Vec<usize>>,
    with_paths: usize,
    ran: &Ran,
) -> Vec<Finding> {
    let note = |text: String, operation| Finding {
        level: Level::Note,
        code: "travel".into(),
        text,
        operation,
        at_move: None,
    };
    let mut out = Vec::new();
    for (why, ops) in unchecked {
        if ops.len() == with_paths {
            out.push(note(format!("travel: {why}"), None));
            continue;
        }
        for k in ops {
            let name = ran.result.operations.get(k).map_or("", |o| o.name.as_str());
            out.push(note(format!("travel {name}: {why}"), Some(k)));
        }
    }
    out
}

/// For each of a program's `lines`, how many moves are made once it has run:
/// `made` says it of each instruction, and `op_lines` how many lines are
/// written once each is. The header's lines make none.
pub(crate) fn line_moves(made: &[usize], op_lines: &[u64], lines: u64) -> Vec<u32> {
    let mut out = Vec::with_capacity(usize::try_from(lines).unwrap_or(0));
    let mut now = 0;
    for (n, upto) in made.iter().zip(op_lines) {
        now = u32::try_from(*n).unwrap_or(u32::MAX);
        while (out.len() as u64) < *upto {
            out.push(now);
        }
    }
    // The footer's.
    while (out.len() as u64) < lines {
        out.push(now);
    }
    out
}

/// For each of a setup's program's `lines`, the operation it is of and how
/// many of that operation's moves are made once it has run: `made` says it
/// of each instruction, and `op_lines` how many lines are written once each
/// is. The header's lines are the first operation's, making none.
pub(crate) fn line_moments(
    made: &[(usize, usize)],
    op_lines: &[u64],
    lines: u64,
) -> (Vec<u32>, Vec<u32>) {
    let size = usize::try_from(lines).unwrap_or(0);
    let (mut ops, mut moves) = (Vec::with_capacity(size), Vec::with_capacity(size));
    let whole = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    let mut now = made.first().map_or((0, 0), |&(k, _)| (whole(k), 0));
    for (&(k, n), upto) in made.iter().zip(op_lines) {
        now = (whole(k), whole(n));
        while (ops.len() as u64) < *upto {
            ops.push(now.0);
            moves.push(now.1);
        }
    }
    // The footer's.
    while (ops.len() as u64) < lines {
        ops.push(now.0);
        moves.push(now.1);
    }
    (ops, moves)
}

/// Today's date, as `2026-11-03`, in UTC: what a program's `@date@` says.
fn today() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400);
    let (y, m, d) = civil(i64::try_from(days).unwrap_or(0));
    format!("{y:04}-{m:02}-{d:02}")
}

/// The year, month and day `days` after 1970-01-01, in the proleptic
/// Gregorian calendar (Howard Hinnant's `civil_from_days`).
pub(crate) fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (
        y,
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

/// Whether a 2D operation goes down to its picks without a bottom: it has some,
/// and they are what it cuts round.
fn down_to_picks(op: &Operation) -> bool {
    op.strategy.uses().outline && !op.selection.is_empty() && op.outline == Outline::Selection
}

/// `op`'s heights as planes across `stock`, as camcraft's app draws them: the
/// clearance, the retract where it stays down, and the top and bottom where
/// the strategy has them.
fn planes(op: &Operation, ctx: &HeightContext, stock: Aabb, frame: &Frame) -> Vec<HeightPlane> {
    let uses = op.strategy.uses();
    let (top, bottom) = if uses.outline {
        op.levels_2d(ctx, ctx.picked)
    } else {
        op.levels_3d(ctx)
    };
    // A little past the stock, so a plane on its top does not hide its edges.
    let grow = 0.04 * (stock.max.x - stock.min.x).max(stock.max.y - stock.min.y);
    let (lo, hi) = (
        stock.min.truncate() - DVec2::splat(grow),
        stock.max.truncate() + DVec2::splat(grow),
    );
    [
        ("Clearance", Some(op.heights.clearance.resolve(ctx))),
        (
            "Retract",
            uses.stay_down.then(|| op.heights.retract.resolve(ctx)),
        ),
        ("Top", uses.top.map(|_| top)),
        ("Bottom", uses.bottom.map(|_| bottom)),
    ]
    .into_iter()
    .filter_map(|(name, z)| {
        let z = z?;
        let at = |x: f64, y: f64| to_f32(frame.to_model(DVec3::new(x, y, z)));
        Some(HeightPlane {
            name: name.into(),
            z,
            corners: [
                at(lo.x, lo.y),
                at(hi.x, lo.y),
                at(hi.x, hi.y),
                at(lo.x, hi.y),
            ],
        })
    })
    .collect()
}

/// The stock as a round bar, in the model's coordinates, when it is one.
fn cylinder(frame: &Frame, blocks: &[Block]) -> Option<Cylinder> {
    let [Block::Bar(bar)] = blocks else {
        return None;
    };
    let along = match bar.axis {
        Axis::X => DVec3::X,
        Axis::Y => DVec3::Y,
        Axis::Z => DVec3::Z,
    } * (bar.length * 0.5);
    Some(Cylinder {
        ends: [
            to_array(frame.to_model(bar.centre - along)),
            to_array(frame.to_model(bar.centre + along)),
        ],
        radius: bar.radius,
    })
}

/// Whether there is stock at the start, or within `grow` of it, under each
/// corner of a square of a grid from `lo` to `lo + step`: a square of a grid
/// drawn over the stock's box is drawn only where there is, so round stock
/// leaves its box's corners empty rather than flat at its foot, and its edge
/// is not drawn falling to it.
pub(crate) fn stocked(blocks: &[Block], lo: DVec2, step: DVec2, grow: f64) -> bool {
    [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)]
        .into_iter()
        .all(|(i, j)| {
            let p = lo + step * DVec2::new(i, j);
            blocks.iter().any(|b| b.top_near(p, grow).is_some())
        })
}

/// The triangles of a grid of `nx` by `ny` points, row by row from
/// `origin`, `step` apart: two a square, of each square over the stock.
pub(crate) fn grid_indices(
    nx: usize,
    ny: usize,
    origin: DVec2,
    step: DVec2,
    blocks: &[Block],
) -> Vec<u32> {
    let mut indices = Vec::with_capacity((nx - 1) * (ny - 1) * 6);
    for j in 0..ny - 1 {
        for i in 0..nx - 1 {
            let corner = origin + step * DVec2::new(i as f64, j as f64);
            // A rounding error past the box's edge is on it.
            if !stocked(blocks, corner, step, 1e-6 * step.max_element()) {
                continue;
            }
            let a = (j * nx + i) as u32;
            let b = a + 1;
            let c = a + nx as u32;
            let d = c + 1;
            indices.extend([a, b, d, a, d, c]);
        }
    }
    indices
}

/// A box in the setup's frame as its eight corners in the model's, as
/// [`cam_protocol::Run::stock`] has them: a tilted setup's is turned.
fn corners(frame: &Frame, b: Aabb) -> [[f64; 3]; 8] {
    std::array::from_fn(|i| {
        let pick = |bit: usize, lo: f64, hi: f64| if i & bit == 0 { lo } else { hi };
        to_array(frame.to_model(DVec3::new(
            pick(1, b.min.x, b.max.x),
            pick(2, b.min.y, b.max.y),
            pick(4, b.min.z, b.max.z),
        )))
    })
}

/// A row-major rigid 4x4 matrix as an affine map.
fn affine(rows: &[f64; 16]) -> DAffine3 {
    DAffine3::from_mat4(DMat4::from_cols_array(rows).transpose())
}

fn to_array(p: DVec3) -> [f64; 3] {
    [p.x, p.y, p.z]
}

fn to_f32(p: DVec3) -> [f32; 3] {
    [p.x as f32, p.y as f32, p.z as f32]
}

/// An operation's toolpath as drawn, and what playing it back needs to know
/// of it.
pub(crate) struct Drawn {
    /// Its strokes in the model's coordinates, thinned to the budget.
    pub strokes: Vec<Stroke>,
    /// How many points the whole toolpath has, before thinning.
    pub points: usize,
    /// How far it cuts, and how far it moves at rapid, in millimetres.
    pub cut: f64,
    pub rapid: f64,
    /// Where each move ends, in the setup's frame.
    pub ends: Vec<DVec3>,
    /// The machining seconds after each move, from the operation's start.
    pub times: Vec<f64>,
    /// How many points, before thinning, the strokes have after each move.
    pub after_move: Vec<usize>,
    /// Which points, before thinning, were kept, in order; `None` for all.
    pub kept: Option<Vec<usize>>,
    /// The lowest and highest the tool's tip goes, in the setup's frame;
    /// `None` for no moves.
    pub z: Option<[f64; 2]>,
    /// The fastest any move is fed, mm/min.
    pub fastest: f64,
}

impl Drawn {
    /// How many of the drawn points come before the tool, after `moves` moves.
    pub fn drawn_after(&self, moves: usize) -> usize {
        let raw = match moves {
            0 => 0,
            m => self.after_move.get(m - 1).copied().unwrap_or(self.points),
        };
        match &self.kept {
            Some(kept) => kept.partition_point(|&i| i < raw),
            None => raw,
        }
    }
}

/// How long a move `length` long from `from` to `to`, in the setup's frame,
/// takes, in seconds: at its feed, or at rapid, as the machine's axes go
/// (`rapid`). The program's axes are the setup's.
pub(crate) fn duration(
    from: DVec3,
    to: DVec3,
    length: f64,
    feed: Option<f64>,
    rapid: &Rapid,
) -> f64 {
    match feed.filter(|f| *f > 0.0) {
        Some(feed) => rapid.feed_seconds(length, to - from, feed),
        None => rapid.seconds(to - from),
    }
}

/// `path` as strokes in the model's coordinates, thinned to `budget` points,
/// with how many points it has, how far it cuts and how far it rapids, each
/// move timed at the machine's rates (`rates`).
pub(crate) fn strokes(
    path: &Toolpath,
    frame: &Frame,
    tolerance: f64,
    budget: usize,
    rates: &Rapid,
) -> Drawn {
    let mut out: Vec<Stroke> = Vec::new();
    let (mut cut, mut rapid) = (0.0, 0.0);
    let mut at: Option<DVec3> = None;
    let mut ends = Vec::new();
    let mut after_move = Vec::new();
    let mut times = Vec::new();
    let mut clock = 0.0;
    let mut raw = 0;
    let mut z: Option<[f64; 2]> = None;
    let mut fastest: f64 = 0.0;
    for m in path.moves() {
        let fast = m.role == MoveRole::Rapid;
        ends.push(m.to);
        z = Some(z.map_or([m.to.z, m.to.z], |[lo, hi]| {
            [lo.min(m.to.z), hi.max(m.to.z)]
        }));
        if let Some(f) = m.feed {
            fastest = fastest.max(f);
        }
        let Some(from) = at else {
            times.push(clock);
            at = Some(m.to);
            out.push(Stroke {
                rapid: fast,
                points: vec![to_f32(frame.to_model(m.to))],
            });
            raw += 1;
            after_move.push(raw);
            continue;
        };
        let length = m.length(from);
        clock += duration(from, m.to, length, m.feed, rates);
        times.push(clock);
        if fast {
            rapid += length;
        } else {
            cut += length;
        }
        if out.last().is_none_or(|s| s.rapid != fast) {
            out.push(Stroke {
                rapid: fast,
                points: vec![to_f32(frame.to_model(from))],
            });
            raw += 1;
        }
        if let Some(stroke) = out.last_mut() {
            let chords = m.chords(from, tolerance);
            raw += chords.len();
            stroke
                .points
                .extend(chords.into_iter().map(|p| to_f32(frame.to_model(p))));
        }
        after_move.push(raw);
        at = Some(m.to);
    }
    let points: usize = out.iter().map(|s| s.points.len()).sum();
    let mut drawn = Drawn {
        strokes: Vec::new(),
        points,
        cut,
        rapid,
        ends,
        times,
        after_move,
        kept: None,
        z,
        fastest,
    };
    if budget == 0 {
        return drawn;
    }
    if points > budget {
        let every = points.div_ceil(budget);
        let mut kept_all = Vec::with_capacity(budget + out.len());
        let mut offset = 0;
        for stroke in &mut out {
            let last = stroke.points.len().saturating_sub(1);
            let mut kept = Vec::new();
            for (i, p) in stroke.points.iter().enumerate() {
                if i % every == 0 || i == last {
                    kept.push(*p);
                    kept_all.push(offset + i);
                }
            }
            offset += stroke.points.len();
            stroke.points = kept;
        }
        drawn.kept = Some(kept_all);
    }
    drawn.strokes = out;
    drawn
}

/// The finished material's top over the stock, as triangles in the model's
/// coordinates.
fn material(ran: &Ran) -> Option<Material> {
    let r = &ran.result;
    let stock = r.keyframes.last()?.unpack().ok()?;
    let (lo, hi) = (r.stock_min, r.stock_max);
    let span = hi - lo;
    if !(span.x > 0.0 && span.y > 0.0) {
        return None;
    }
    let pitch = span.x.max(span.y) / MATERIAL_COLUMNS as f64;
    let nx = ((span.x / pitch).ceil() as usize).max(1) + 1;
    let ny = ((span.y / pitch).ceil() as usize).max(1) + 1;
    let step = DVec2::new(span.x / (nx - 1) as f64, span.y / (ny - 1) as f64);
    let floor = stock.floor();
    let mut positions = Vec::with_capacity(nx * ny);
    for j in 0..ny {
        for i in 0..nx {
            let p = lo + step * DVec2::new(i as f64, j as f64);
            let z = stock.height_at(p.x, p.y).unwrap_or(floor);
            positions.push(to_f32(r.frame.to_model(p.extend(z))));
        }
    }
    let indices = grid_indices(nx, ny, lo, step, &r.stocks);
    Some(Material { positions, indices })
}

pub mod cache;
mod outlines;
mod playback;
pub mod session;
#[cfg(test)]
mod tests;
