//! Posting: a setup's programs named, rendered and written.
//!
//! A setup is posted as one program, every operation in it, or as one
//! program an operation, as it says ([`Programs`]) unless the posting says
//! otherwise.
//!
//! A [`Posting`] is made first, with every program rendered, then written in one
//! go. It says what stands in the way — setups that did not pass their checks, and
//! files that would be replaced — so a front end can ask before anything is
//! written. Several setups are all posted or none are: half a job's programs are a
//! trap on the machine. The command line's `--out` and the app's Post both come
//! through here, so they write the same files under the same names.

use std::path::{Path, PathBuf};

use camcraft_core::job::{Programs, Setup};
use camcraft_core::run::{Made, OperationResult, SetupResult, StrategyDetail};
use camcraft_core::setup::bounds_all;
use camcraft_core::setup::frame::Frame;
use camcraft_post::op::{CustomGcode, DrillCycles, LowerError, ProgramSpec, extents, lower};
use camcraft_post::profile::{Placement, Profile, TapStyle, TravelCheck};
use camcraft_post::render::{RenderReport, Title, render, render_program};
use camcraft_post::setup::{Joining, Section, lower_setup};
use glam::DVec3;

use crate::{Ran, SessionError, fs};

/// A builtin machine profile by name, or a TOML profile by path. A relative path
/// is found from `job_dir`, when there is one.
pub fn load_profile(name: &str, job_dir: Option<&Path>) -> Result<Profile, SessionError> {
    Ok(match Profile::builtin(name) {
        Some(p) => p?,
        None => match job_dir {
            Some(dir) => Profile::load(fs::resolve(dir, name))?,
            None => Profile::load(name)?,
        },
    })
}

/// The machine profile `setup` is posted with: `profile` for every setup when
/// given, or its own, named from `job_dir`.
pub fn setup_profile(
    setup: &Setup,
    job_dir: &Path,
    profile: Option<&str>,
) -> Result<Profile, SessionError> {
    load_profile(profile.unwrap_or(&setup.post), Some(job_dir)).map_err(|source| {
        SessionError::InSetup {
            setup: setup.name.clone(),
            source: Box::new(source),
        }
    })
}

/// The machine profiles built in, by name.
pub fn builtin_posts() -> impl Iterator<Item = &'static str> {
    Profile::builtin_names()
}

fn file_safe(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// The program name for the `index`-th operation (from zero) of a setup:
/// `top-01-rough`.
///
/// The name is kept usable as a file name.
pub fn program_name(setup: &str, index: usize, op_name: &str) -> String {
    format!(
        "{}-{:02}-{}",
        file_safe(setup),
        index + 1,
        file_safe(op_name)
    )
}

/// The program name for the whole of a setup posted as one: `top`.
pub fn setup_program_name(setup: &str) -> String {
    file_safe(setup)
}

/// Everything one operation's program needs besides its path, for `profile`,
/// in a setup whose frame is `frame`: tilted, where the machine's rotaries go
/// to stand its tool axis up the spindle, which they may not reach. A tap
/// for a machine that cannot tap is refused, saying which built-in machines
/// can.
pub fn program_spec(
    op: &OperationResult,
    wcs: u8,
    frame: &Frame,
    profile: &Profile,
) -> Result<ProgramSpec, LowerError> {
    let [x, _, z] = frame.turn.axes();
    let tap = match &op.detail {
        StrategyDetail::Drill(r) => r.tap,
        _ => None,
    };
    let tapping = profile.tapping();
    if tap.is_some() && tapping == TapStyle::Cannot {
        return Err(LowerError::CannotTap {
            machine: profile.name.clone(),
            can: Profile::builtin_tapping().join(", "),
        });
    }
    let rotary = match &profile.rotary {
        Some(r) => r.position(z, x)?,
        None => Vec::new(),
    };
    Ok(ProgramSpec {
        op_name: op.name.clone(),
        strategy: op.strategy.to_string(),
        tool: op.tool_number,
        tool_comment: op.tool_comment.clone(),
        wcs,
        extended_offsets: u8::try_from(profile.extended_offsets.len()).unwrap_or(u8::MAX),
        rpm: op.rpm,
        spin: op.spindle,
        coolant: op.coolant,
        units: profile.units,
        // Holes as cycles where the machine takes them; a tap however it
        // taps, cycles or not.
        drill: match &op.detail {
            StrategyDetail::Drill(r) if profile.drill_cycles || r.tap.is_some() => {
                Some(DrillCycles {
                    holes: r.holes.clone(),
                    peck: r.peck,
                    chip_break: r.chip_break,
                    dwell: r.dwell,
                    tap: r.tap,
                    tap_style: tapping,
                    feed: r.feed,
                    clearance: op.clearance_z,
                })
            }
            _ => None,
        },
        taps: tap.is_some(),
        // Said in the header when the part is tilted, or turned by the
        // rotaries; a part turned square by hand is written as it always was.
        tilt: (z != glam::DVec3::Z && (frame.turn.square().is_none() || !rotary.is_empty()))
            .then_some(z),
        rotary,
        next_tool: None,
        stock: None,
        before: Vec::new(),
        after: Vec::new(),
    })
}

/// For each of a setup's operations, as run, the tool the program after its
/// own changes to: the next operation's with a program, if one has.
pub fn next_tools(ops: &[OperationResult]) -> Vec<Option<u32>> {
    (0..ops.len())
        .map(|k| {
            ops.iter()
                .skip(k + 1)
                .find(|op| op.made.has_paths())
                .map(|op| op.tool_number)
        })
        .collect()
}

/// A setup's stock as a box, its lowest and highest corners in the setup's
/// frame, the programs' coordinates: for a control that draws it.
pub fn stock_box(result: &SetupResult) -> Option<(DVec3, DVec3)> {
    bounds_all(&result.stocks).map(|b| (b.min, b.max))
}

/// For each of a setup's operations, as run (`ops`), the custom G-code
/// written before its own start and after its end. Each Custom G-code
/// operation of `setup` goes before the next operation with a program, or,
/// with none after it, after the last; in a setup with no program at all it
/// is written nowhere.
pub fn custom_gcode(
    setup: &Setup,
    ops: &[OperationResult],
) -> Vec<(Vec<CustomGcode>, Vec<CustomGcode>)> {
    let mut out = vec![(Vec::new(), Vec::new()); ops.len()];
    let mut waiting = Vec::new();
    let mut last = None;
    for (k, op) in ops.iter().enumerate() {
        if op.made == Made::Text {
            let text = setup.operations.get(k).map_or("", |o| o.gcode.as_str());
            waiting.push(CustomGcode::new(&op.name, text));
        } else if op.made.has_paths() {
            if let Some((before, _)) = out.get_mut(k) {
                before.append(&mut waiting);
            }
            last = Some(k);
        }
    }
    if let Some((_, after)) = last.and_then(|k| out.get_mut(k)) {
        after.append(&mut waiting);
    }
    out
}

/// How operation `op`'s moves, in `setup`, keep to `profile`'s travel: each
/// axis they take past it, and what could not be checked. In machine
/// coordinates, the setup's work offset places them on the machine, and the
/// tool's length from the gauge line raises Z for a profile that takes length
/// offsets. The moves are checked along the setup's own axes: tilted, as the
/// machine moves once its rotaries have stood the part up (the setup's X on
/// the machine's, as positioning them requires), or as it is fixtured tilted.
pub fn travel_check(op: &OperationResult, setup: &Setup, profile: &Profile) -> TravelCheck {
    let place = Placement {
        zero: setup.work_offset.map(DVec3::from_array),
        tool_length: op.tool_length,
    };
    match extents(&op.path) {
        Some((low, high)) => profile.travel_check(low, high, &place),
        None => TravelCheck::default(),
    }
}

/// Render one operation, of a setup whose frame is `frame`, as a program for
/// `profile`.
pub fn program(
    op: &OperationResult,
    wcs: u8,
    frame: &Frame,
    profile: &Profile,
    name: &str,
) -> Result<(String, RenderReport), SessionError> {
    program_with(op, &program_spec(op, wcs, frame, profile)?, profile, name)
}

/// Render one operation, with `spec` (from [`program_spec`], and its custom
/// G-code), as a program for `profile`.
pub fn program_with(
    op: &OperationResult,
    spec: &ProgramSpec,
    profile: &Profile,
    name: &str,
) -> Result<(String, RenderReport), SessionError> {
    let ops = lower(&op.path, spec)?;
    Ok(render(&ops, profile, name))
}

/// Each of a setup's operations that has a program, by its index among the
/// setup's operations as run (`result`), with everything its part of a
/// program needs besides its path, its custom G-code and the stock among
/// it; or the first
/// that cannot be posted, by its index, and why.
pub fn setup_specs(
    setup: &Setup,
    result: &SetupResult,
    profile: &Profile,
) -> Result<Vec<(usize, ProgramSpec)>, (usize, LowerError)> {
    let custom = custom_gcode(setup, &result.operations);
    let stock = stock_box(result);
    let mut out = Vec::new();
    for (k, op) in result.operations.iter().enumerate() {
        if !op.made.has_paths() {
            continue;
        }
        let mut spec = program_spec(op, setup.wcs, &result.frame, profile).map_err(|e| (k, e))?;
        if let Some((before, after)) = custom.get(k) {
            spec.before.clone_from(before);
            spec.after.clone_from(after);
        }
        // The next tool is the joined program's to call up; the stock is
        // the setup's.
        spec.stock = stock;
        out.push((k, spec));
    }
    Ok(out)
}

/// A setup posted as one program.
#[derive(Debug, Clone)]
pub struct SetupProgram {
    /// What it says.
    pub text: String,
    /// What rendering wrote.
    pub report: RenderReport,
    /// For each instruction, the operation it is of, by its index among the
    /// setup's as run, and how many of that operation's moves are made once
    /// it has run: with `report.op_lines`, which operation and move each
    /// line is of.
    pub made: Vec<(usize, usize)>,
    /// How many times it changes the tool, the first included: for its
    /// machining time.
    pub tool_changes: usize,
}

/// Every operation of `setup` that has a program, as run (`result`),
/// posted as one program for `profile`, called as `title` says; or the
/// first operation that cannot be posted, by its index, and why.
pub fn setup_program(
    setup: &Setup,
    result: &SetupResult,
    profile: &Profile,
    title: &Title,
) -> Result<SetupProgram, (usize, LowerError)> {
    let specs = setup_specs(setup, result, profile)?;
    let sections: Vec<Section> = specs
        .iter()
        .filter_map(|(k, spec)| {
            let path = &result.operations.get(*k)?.path;
            Some(Section { path, spec })
        })
        .collect();
    let (ops, made) = lower_setup(&sections, Joining::of(profile)).map_err(|e| {
        let k = specs.get(e.section).map_or(0, |(k, _)| *k);
        (k, e.error)
    })?;
    let (text, report) = render_program(&ops, profile, title);
    let tool_changes = ops
        .iter()
        .filter(|op| matches!(op, camcraft_post::op::Op::ToolChange { .. }))
        .count();
    let made = made
        .into_iter()
        .map(|(section, n)| (specs.get(section).map_or(0, |(k, _)| *k), n))
        .collect();
    Ok(SetupProgram {
        text,
        report,
        made,
        tool_changes,
    })
}

/// One program, rendered.
#[derive(Debug, Clone)]
pub struct Program {
    /// Where it goes.
    pub path: PathBuf,
    /// What it says.
    pub text: String,
    /// The operation it cuts, or the setup, posted as one program.
    pub op: String,
    /// The machine profile it was rendered for.
    pub profile: String,
    /// What rendering it wrote.
    pub report: RenderReport,
}

/// What posting would write, and what stands in the way.
#[derive(Debug, Clone)]
pub struct Posting {
    /// The folder the programs go into.
    pub dir: PathBuf,
    /// Every program, in setup and operation order.
    pub programs: Vec<Program>,
    /// Setups that did not pass their checks, each with what failed.
    pub refused: Vec<(String, Vec<String>)>,
    /// Programs that would replace a file already there.
    pub overwrites: Vec<PathBuf>,
}

impl Posting {
    /// Whether anything must be confirmed before writing.
    pub fn needs_confirming(&self) -> bool {
        !self.refused.is_empty() || !self.overwrites.is_empty()
    }
}

/// Render every program of `setups` (each with its run) into `dir`, writing
/// nothing.
///
/// A setup's `post` names a builtin profile or a file, found from `job_dir`;
/// `profile`, when given, is used for every setup instead. Each setup is
/// posted as its `programs` says, or as `programs` says when it is given.
pub fn plan_posting(
    setups: &[(&Setup, &Ran)],
    job_dir: &Path,
    dir: &Path,
    profile: Option<&str>,
    programs: Option<Programs>,
) -> Result<Posting, SessionError> {
    let mut out = Posting {
        dir: dir.to_path_buf(),
        programs: Vec::new(),
        refused: Vec::new(),
        overwrites: Vec::new(),
    };
    // Each program's file, by what a file system that ignores case sees, and
    // whose it is: two names can come to one file.
    let mut taken: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let programs_given = programs;
    for (setup, ran) in setups {
        let in_setup = |source| SessionError::InSetup {
            setup: setup.name.clone(),
            source: Box::new(source),
        };
        let loaded = setup_profile(setup, job_dir, profile)?;
        // Past the machine's travel, a program stops at its limits part-way
        // through, or crashes into them.
        let past: Vec<String> = ran
            .result
            .operations
            .iter()
            .filter(|op| op.made.has_paths())
            .flat_map(|op| {
                travel_check(op, setup, &loaded)
                    .past
                    .into_iter()
                    .map(|why| format!("travel {}: {why}", op.name))
            })
            .collect();
        if !ran.passed() || !past.is_empty() {
            let mut why = ran.checks.failure_lines();
            if why.is_empty() && !ran.passed() {
                why.push("the simulator did not verify it".into());
            }
            why.extend(past);
            out.refused.push((setup.name.clone(), why));
        }
        // Each program's name, whose it is, the operation it cuts (the
        // setup, for one of them all), and what it says.
        let mut programs = Vec::new();
        let one_each = programs_given.unwrap_or(setup.programs) == Programs::Operation;
        if !one_each && ran.result.operations.iter().any(|op| op.made.has_paths()) {
            let name = setup_program_name(&setup.name);
            let title = Title {
                name: &name,
                number: setup.number,
                date: None,
            };
            let p = setup_program(setup, &ran.result, &loaded, &title)
                .map_err(|(_, e)| in_setup(e.into()))?;
            let whose = format!("setup '{}'", setup.name);
            programs.push((name, whose, setup.name.clone(), p.text, p.report));
        }
        let custom = custom_gcode(setup, &ran.result.operations);
        let next = next_tools(&ran.result.operations);
        let stock = stock_box(&ran.result);
        // An operation turned off or left out of the run has no program.
        let made = ran.result.operations.iter().enumerate();
        for (k, op) in made.filter(|(_, op)| one_each && op.made.has_paths()) {
            let name = program_name(&setup.name, k, &op.name);
            let mut spec = program_spec(op, setup.wcs, &ran.result.frame, &loaded)
                .map_err(|e| in_setup(e.into()))?;
            if let Some((before, after)) = custom.get(k) {
                spec.before.clone_from(before);
                spec.after.clone_from(after);
            }
            spec.next_tool = next.get(k).copied().flatten();
            spec.stock = stock;
            // Numbered on from the setup's, one a program.
            let title = Title {
                name: &name,
                number: setup
                    .number
                    .saturating_add(u32::try_from(programs.len()).unwrap_or(u32::MAX)),
                date: None,
            };
            let ops = lower(&op.path, &spec).map_err(|e| in_setup(e.into()))?;
            let (text, report) = render_program(&ops, &loaded, &title);
            let whose = format!("setup '{}' operation '{}'", setup.name, op.name);
            programs.push((name, whose, op.name.clone(), text, report));
        }
        for (name, whose, op, text, report) in programs {
            let path = dir.join(format!("{name}.{}", loaded.extension));
            let key = path.display().to_string().to_lowercase();
            if let Some(first) = taken.insert(key, whose.clone()) {
                return Err(SessionError::SameProgram {
                    path: path.display().to_string(),
                    first,
                    second: whose,
                });
            }
            if path.exists() {
                out.overwrites.push(path.clone());
            }
            out.programs.push(Program {
                path,
                text,
                op,
                profile: loaded.name.clone(),
                report,
            });
        }
    }
    Ok(out)
}

/// Write every program in `posting`: each beside its place first, and only once
/// all are written is each moved into it, so a failure writing any of them
/// leaves every program as it was, not some of this posting's and some of the
/// last's. A move that fails part-way through the moves can still leave some
/// of each. Returns how many were written.
pub fn write_posting(posting: &Posting) -> Result<usize, SessionError> {
    let failed = |path: &Path| {
        let path = path.display().to_string();
        move |source| SessionError::ProgramWrite { path, source }
    };
    std::fs::create_dir_all(&posting.dir).map_err(failed(&posting.dir))?;
    let temp = |path: &Path| {
        let mut t = path.as_os_str().to_owned();
        t.push(".camcraft-posting");
        std::path::PathBuf::from(t)
    };
    let clean = || {
        for p in &posting.programs {
            let _ = std::fs::remove_file(temp(&p.path));
        }
    };
    for p in &posting.programs {
        if let Err(e) = std::fs::write(temp(&p.path), &p.text) {
            clean();
            return Err(failed(&p.path)(e));
        }
    }
    for p in &posting.programs {
        if let Err(e) = std::fs::rename(temp(&p.path), &p.path) {
            clean();
            return Err(failed(&p.path)(e));
        }
    }
    Ok(posting.programs.len())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests;
