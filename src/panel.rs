//! The task panels of a setup and an operation, the preferences page, and what
//! their widgets do. They are made of printCAD's panel blocks, so the app
//! draws them as it draws its own panels: folds with a count and a summary,
//! item lists with menus, pick chips, a status pill in the task's header.

use cam_protocol::{Level, Run};
use camcraft_core::job::{
    Boundary, Containment, CutDirection, Depths, HeightRef, HoleOrder, Leave, Outline,
    PlacementOrder, ProfileSide, Programs, StrategySpec, ToolKind, ToolSpec,
};
use camcraft_core::setup::frame::{Axis, Orientation};
use camcraft_post::profile::Profile;
use printcad_bench_sdk::api::*;
use printcad_bench_sdk::host;
use printcad_bench_sdk::panel as pb;

use crate::data::{
    AxisPick, CylinderAxis, MachineX, OperationData, Pick, STRATEGIES, SetupData, StockKind,
    ToolAxis, Zero,
};
use crate::linking::{linking_event, linking_group};
use crate::passes::{finishing_group, passes_event, relief_group, strategy_rows, tabs_group};
use crate::pattern::{pattern_event, pattern_group};
use crate::picking::Slot;
use crate::runs::State;
use crate::tools::{open_post, open_tools, posts_group, tool_editor, tool_event};
use crate::{Cam, Lit, operation_node, operations_of, setup_node, setups};

/// The most findings a panel lists.
const FINDINGS: usize = 12;

/// A label kept short: what it says in brackets ("Flutes (0: not given)")
/// taken off to be said on hover, and its rate unit after a comma ("Feed,
/// mm/min") to stand after the value. The label, the unit and the words.
fn short(label: &str) -> (&str, Option<String>, Option<&str>) {
    let (label, aside) = match label.strip_suffix(')').and_then(|l| l.split_once(" (")) {
        Some((l, aside)) => (l, Some(aside)),
        None => (label, None),
    };
    for unit in ["mm/min", "rev/min", "s"] {
        if let Some(l) = label.strip_suffix(&format!(", {unit}")) {
            return (l, Some(unit.into()), aside);
        }
    }
    (label, None, aside)
}

/// `widget` with `aside` said on hover, when there is one.
fn aside(widget: Widget, aside: Option<&str>) -> Widget {
    match aside {
        Some(hint) if !matches!(widget, Widget::HintedFigure { .. }) => Widget::Hinted {
            hint: hint.into(),
            widget: Box::new(widget),
        },
        _ => widget,
    }
}

/// A number of feature `feature`, which takes formulas: its parameter is
/// its field's id.
pub(crate) fn number(
    id: &str,
    label: &str,
    value: f64,
    dim: Dim,
    decimals: usize,
    feature: &str,
) -> Widget {
    let (label, unit, words) = short(label);
    let field = crate::hints::with_hint(
        Quantity {
            id: id.into(),
            label: label.into(),
            value,
            dim,
            unit,
            bind: Some(Bind {
                feature: feature.to_string(),
                key: format!("/{id}"),
                ..Default::default()
            }),
            decimals,
            ..Default::default()
        }
        .into(),
    );
    aside(field, words)
}

/// A number kept outside the document (the library's, a part's place),
/// with no formula.
pub(crate) fn unbound(id: &str, label: &str, value: f64, dim: Dim, decimals: usize) -> Widget {
    let (label, unit, words) = short(label);
    aside(
        Quantity {
            id: id.into(),
            label: label.into(),
            unit,
            value,
            dim,
            decimals,
            ..Default::default()
        }
        .into(),
        words,
    )
}

pub(crate) fn choice(id: &str, label: &str, options: Vec<String>, selected: usize) -> Widget {
    Select {
        id: id.into(),
        label: label.into(),
        options,
        selected,
        ..Default::default()
    }
    .into()
}

/// A check box with `text` beside it, told as a toggle.
pub(crate) fn toggle(id: impl Into<String>, text: impl Into<String>, on: bool) -> Widget {
    Check {
        id: id.into(),
        text: text.into(),
        on,
        ..Default::default()
    }
    .into()
}

pub(crate) fn button(id: &str, label: &str, style: ButtonStyle, enabled: bool) -> Widget {
    Widget::Button {
        id: id.into(),
        label: label.into(),
        style,
        enabled,
    }
}

pub(crate) fn note(kind: NoteKind, text: impl Into<String>) -> Widget {
    pb::alert(None, kind, None, &text.into())
}

/// What a fold's fields mean, in the panel's quieter words.
pub(crate) fn about(text: impl Into<String>) -> Widget {
    pb::caption(&text.into(), Tone::Neutral, true)
}

/// A fold of what is less often changed: closed at first, `summary` saying
/// what it holds.
pub(crate) fn fold(title: &str, summary: &str, children: Vec<Widget>) -> Widget {
    pb::section(title)
        .summary(summary)
        .closed()
        .children(children)
        .into()
}

/// A read-only line: words, and a value at its end.
pub(crate) fn stat(label: &str, value: &str) -> Item {
    pb::item(label).plain_value(value).into()
}

/// Read-only lines, one under another.
pub(crate) fn stats(id: &str, items: Vec<Item>) -> Widget {
    pb::items(id).items(items).into()
}

/// What is picked in the view for slot `id`, as a chip a click arms and
/// whose cross clears it.
pub(crate) fn chip(id: &str, label: &str, value: Option<String>, armed: bool) -> Widget {
    PickChip {
        placeholder: "Pick in the view".into(),
        ..pb::pick_chip(id, label, value.as_deref(), armed)
    }
    .into()
}

/// One of the package's own icons, as a panel's rows name it: the app
/// keeps them under the package's id, and draws a row's icon by that name.
pub(crate) fn own_icon(name: &str) -> String {
    format!("{}/{name}", crate::PACKAGE)
}

/// Buttons side by side.
pub(crate) fn buttons(children: Vec<Widget>) -> Widget {
    pb::flow(children)
}

fn axis_label(a: Axis) -> String {
    a.label().to_string()
}

/// The X axes that are square to `z`.
fn x_axes(z: Axis) -> Vec<Axis> {
    Axis::ALL
        .into_iter()
        .filter(|&x| Orientation::square(z, x).check().is_ok())
        .collect()
}

/// The X axes machine X may be taken from: those square to the tool axis
/// when it is one of the document's, and when it is tilted, those not
/// nearly along it, which are made square to it.
fn machine_xs(setup: &SetupData) -> Vec<Axis> {
    if setup.tool_axis == ToolAxis::Square {
        return x_axes(setup.z_axis);
    }
    let up = setup.up();
    Axis::ALL
        .into_iter()
        .filter(|a| {
            let v = a.vector();
            (v.x * up[0] + v.y * up[1] + v.z * up[2]).abs() < 0.999
        })
        .collect()
}

/// Where a face or an edge was picked for an axis, as its chip says it.
fn axis_pick_text(pick: Option<&AxisPick>) -> Option<String> {
    pick.map(|p| format!("{:.2}, {:.2}, {:.2}", p.at[0], p.at[1], p.at[2]))
}

/// The rows of a setup's orientation: where its tool axis comes from, and
/// its machine X, with what each takes.
fn orientation_rows(cam: &Cam, id: &str, setup: &SetupData) -> Vec<Widget> {
    let armed = |slot: Slot| {
        cam.armed
            .as_ref()
            .is_some_and(|a| a.feature == id && a.slot == slot)
    };
    let mut options: Vec<String> = Axis::ALL.into_iter().map(axis_label).collect();
    options.extend(ToolAxis::TILTED.iter().map(|t| t.label().to_string()));
    let selected = match setup.tool_axis {
        ToolAxis::Square => Axis::ALL
            .iter()
            .position(|&a| a == setup.z_axis)
            .unwrap_or(4),
        t => Axis::ALL.len() + ToolAxis::TILTED.iter().position(|&o| o == t).unwrap_or(0),
    };
    let mut rows = vec![choice("z_axis", "Tool axis (Z)", options, selected)];
    match setup.tool_axis {
        ToolAxis::Square => {}
        ToolAxis::Face | ToolAxis::Edge => {
            let (slot, what) = if setup.tool_axis == ToolAxis::Face {
                ("axis_face", "Face")
            } else {
                ("axis_edge", "Edge")
            };
            rows.push(chip(
                slot,
                what,
                axis_pick_text(setup.axis_pick.as_ref()),
                Slot::named(slot).is_some_and(armed),
            ));
            rows.push(toggle(
                "axis_reverse",
                "The other way",
                setup.axis_pick.as_ref().is_some_and(|p| p.reverse),
            ));
        }
        ToolAxis::Tilt => {
            rows.push(number(
                "tilt_x",
                "Tilt about X",
                setup.tilt_x,
                Dim::Angle,
                2,
                id,
            ));
            rows.push(number(
                "tilt_y",
                "Tilt about Y",
                setup.tilt_y,
                Dim::Angle,
                2,
                id,
            ));
        }
        ToolAxis::Direction => {
            rows.push(number(
                "axis_dx",
                "Direction X",
                setup.axis_dx,
                Dim::Number,
                4,
                id,
            ));
            rows.push(number(
                "axis_dy",
                "Direction Y",
                setup.axis_dy,
                Dim::Number,
                4,
                id,
            ));
            rows.push(number(
                "axis_dz",
                "Direction Z",
                setup.axis_dz,
                Dim::Number,
                4,
                id,
            ));
        }
    }
    let xs = machine_xs(setup);
    let mut x_options: Vec<String> = xs.iter().copied().map(axis_label).collect();
    x_options.push("Along an edge picked".into());
    let x_selected = match setup.machine_x {
        MachineX::Axis => xs.iter().position(|&a| a == setup.x_axis).unwrap_or(0),
        MachineX::Edge => xs.len(),
    };
    rows.push(choice("x_axis", "Machine X", x_options, x_selected));
    if setup.machine_x == MachineX::Edge {
        rows.push(chip(
            "x_edge",
            "Edge",
            axis_pick_text(setup.x_pick.as_ref()),
            armed(Slot::XEdge),
        ));
        rows.push(toggle(
            "x_reverse",
            "The other way",
            setup.x_pick.as_ref().is_some_and(|p| p.reverse),
        ));
    }
    let unpicked =
        matches!(setup.tool_axis, ToolAxis::Face | ToolAxis::Edge) && setup.axis_pick.is_none();
    if !setup.square() && !unpicked {
        let up = setup.up();
        let degrees = up[2].clamp(-1.0, 1.0).acos().to_degrees();
        rows.push(note(
            NoteKind::Info,
            format!(
                "Tilted for 3+2 machining: the tool axis points along ({:.3}, {:.3}, {:.3}) \
                 of the document, {degrees:.1}° from its Z, and machine X is made square to \
                 it. The stock is laid out square to the tool. A post with rotary axes \
                 turns the part up; without, fixture it tilted, and set the work offset \
                 in the tilted frame either way.",
                up[0], up[1], up[2]
            ),
        ));
    }
    rows
}

/// The posts to choose from: camcraft's built-in ones, then the user's.
fn posts(cam: &Cam) -> Vec<String> {
    Profile::builtin_names()
        .map(String::from)
        .chain(cam.settings.posts.keys().cloned())
        .collect()
}

pub(crate) fn kind_label(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Flat => "flat",
        ToolKind::Ball => "ball",
        ToolKind::Bullnose => "bull-nose",
        ToolKind::Tapered => "tapered",
        ToolKind::FaceMill => "face mill",
        ToolKind::Lollipop => "lollipop",
        ToolKind::SlotCutter => "slot cutter",
        ToolKind::Cone => "V-bit",
        ToolKind::SpotDrill => "spot drill",
        ToolKind::Countersink => "countersink",
        ToolKind::Drill => "drill",
        ToolKind::Tap => "tap",
        ToolKind::ThreadMill => "thread mill",
        ToolKind::Probe => "touch probe",
    }
}

fn tool_line(tool: &ToolSpec) -> String {
    format!(
        "T{} · {} Ø{} mm{}",
        tool.number,
        kind_label(tool.kind),
        tool.diameter,
        if tool.corner > 0.0 {
            format!(" r{}", tool.corner)
        } else {
            String::new()
        }
    )
}

/// The failed checks of `run` that are operation `only`'s (or the
/// setup's), or all of them.
fn failed(run: &Run, only: Option<usize>) -> usize {
    run.findings
        .iter()
        .filter(|f| f.level == Level::Fail)
        .filter(|f| only.is_none() || f.operation.is_none() || f.operation == only)
        .count()
}

/// Where setup `setup`'s run stands, as a task header's pill says it:
/// for operation `only` of it when given. `runs` is whether it is on.
fn run_status(cam: &Cam, setup: &str, runs: bool, only: Option<usize>) -> StatusLine {
    if !runs {
        return pb::status(
            "Off",
            Tone::Muted,
            "Turned off: it is not run, simulated, checked or posted.",
        );
    }
    let Some(c) = cam.runs.get(setup) else {
        return pb::status(
            "Not run",
            Tone::Warning,
            "Generate to work out its toolpaths and check them.",
        );
    };
    match &c.state {
        State::Queued => pb::status(
            "Waiting",
            Tone::Info,
            "Waiting for another setup to finish.",
        ),
        State::Running(_) => pb::status(
            "Running",
            Tone::Info,
            "Working out the toolpaths and simulating them.",
        ),
        State::Stopped => pb::status(
            "Stopped",
            Tone::Warning,
            "The run was stopped before it was done.",
        ),
        State::Failed(e) => pb::status("Failed", Tone::Bad, e),
        State::Ready(run) if !run.passed => {
            let n = failed(run, only).max(1);
            pb::status(
                &format!("{n} failed"),
                Tone::Bad,
                "Checks failed: camcraft will not post this setup. Do not run it.",
            )
        }
        State::Ready(run) => {
            let warned = run
                .findings
                .iter()
                .filter(|f| f.level == Level::Warn)
                .any(|f| only.is_none() || f.operation.is_none() || f.operation == only);
            let tone = if warned { Tone::Warning } else { Tone::Good };
            let detail = if warned {
                "Every check passed, with warnings: the checks say which."
            } else {
                "Verified against the material: every check passed."
            };
            let text = match only {
                Some(i) => match run.operations.get(i) {
                    Some(o) if o.seconds > 0.0 => crate::sheet::duration(o.seconds),
                    Some(o) if o.made == "custom-gcode" => "G-code".into(),
                    Some(o) => o.made.clone(),
                    None => "Not run".into(),
                },
                None => {
                    let ops = run.operations.len();
                    format!(
                        "{ops} operation{} · {}",
                        if ops == 1 { "" } else { "s" },
                        crate::sheet::duration(run.machining.total())
                    )
                }
            };
            pb::status(&text, tone, detail)
        }
    }
}

/// What a setup's run says: where it stands, its checks (those of operation
/// `only`, when given) and the way to save its programs.
fn run_widgets(cam: &Cam, setup: &str, only: Option<usize>) -> Vec<Widget> {
    let mut out = Vec::new();
    let Some(c) = cam.runs.get(setup) else {
        out.push(button("generate", "Generate", ButtonStyle::Primary, true));
        return out;
    };
    match &c.state {
        State::Queued => {
            out.push(Widget::Progress {
                label: "Waiting for another setup to finish".into(),
                fraction: None,
                job: None,
            });
            out.push(button("stop", "Stop", ButtonStyle::Secondary, true));
        }
        State::Running(job) => {
            out.push(Widget::Progress {
                label: "Working out the toolpaths and simulating them".into(),
                fraction: None,
                job: Some(*job),
            });
            out.push(button("stop", "Stop", ButtonStyle::Secondary, true));
        }
        State::Stopped => {
            out.push(note(
                NoteKind::Warning,
                "The run was stopped before it was done.",
            ));
            out.push(button("generate", "Generate", ButtonStyle::Primary, true));
        }
        State::Failed(e) => {
            out.push(note(NoteKind::Error, e.clone()));
            out.push(button(
                "generate",
                "Try again",
                ButtonStyle::Secondary,
                true,
            ));
        }
        State::Ready(_) => {}
    }
    let Some(run) = c.shown() else {
        return out;
    };
    if c.stale() {
        out.push(pb::caption(
            "What follows is from the last run.",
            Tone::Muted,
            true,
        ));
    }
    out.extend(verdict(run, only));
    // The program being played: the operation's own, or in a setup's panel
    // the one under way.
    let here = cam.under_way(setup);
    let under = here.and_then(|(s, _)| crate::playback::section_of(run, s));
    if let Some(op) = only.or(under.map(|s| s.op)) {
        let mut kept = cam.listing.borrow_mut();
        out.extend(crate::listing::widgets(run, op, here, &mut kept));
    }
    out
}

/// A finding's row: its icon and tone by how bad it is.
fn finding_item(f: &cam_protocol::Finding) -> Item {
    let (icon, tone) = match f.level {
        Level::Fail => ("error", Tone::Bad),
        Level::Warn => ("warning", Tone::Warning),
        _ => ("info", Tone::Muted),
    };
    pb::item(&f.text).icon(icon).tone(tone).hint(&f.text).into()
}

fn verdict(run: &Run, only: Option<usize>) -> Vec<Widget> {
    let mut out = Vec::new();
    let mine =
        |f: &&cam_protocol::Finding| only.is_none() || f.operation.is_none() || f.operation == only;
    out.push(if run.passed {
        note(
            NoteKind::Success,
            format!(
                "Verified against the material: every check passed ({:.1} s).",
                run.seconds
            ),
        )
    } else {
        note(
            NoteKind::Error,
            format!(
                "{} check(s) failed: camcraft will not post this setup. Do not run it.",
                failed(run, None)
            ),
        )
    });
    let found: Vec<&cam_protocol::Finding> = run
        .findings
        .iter()
        .filter(mine)
        .filter(|f| f.level != Level::Pass)
        .collect();
    if !found.is_empty() {
        let fails = found.iter().filter(|f| f.level == Level::Fail).count();
        let warns = found.len() - fails;
        let mut summary = Vec::new();
        if fails > 0 {
            summary.push(format!("{fails} failed"));
        }
        if warns > 0 {
            summary.push(format!("{warns} to note"));
        }
        let mut checks = pb::section("Checks")
            .count(found.len())
            .summary(&summary.join(" · "))
            .child(
                pb::items("findings").items(found.iter().take(FINDINGS).map(|f| finding_item(f))),
            );
        if run.passed {
            checks = checks.closed();
        }
        out.push(checks.into());
    }
    match only {
        Some(i) => {
            if let Some(op) = run.operations.get(i) {
                let mut lines = Vec::new();
                let mut about_it = None;
                // Short lines: a long value widens the panel over the view.
                if op.made == "generated" || op.made == "kept" {
                    lines.push(stat(
                        "Toolpath",
                        &format!("{:.0} mm cut, {:.0} mm rapid", op.cut_mm, op.rapid_mm),
                    ));
                    lines.push(stat("Machining time", &crate::sheet::duration(op.seconds)));
                    about_it = Some(op.summary.clone());
                } else if op.made == "custom-gcode" {
                    lines.push(stat("G-code", &op.summary));
                } else {
                    lines.push(stat("Toolpath", &op.made));
                }
                // Its own, or, posted as one, its setup's.
                let program = op.program.as_ref().or(run.program.as_ref());
                if let Some(p) = program {
                    lines.push(
                        pb::item(if p.is_setups() {
                            "Setup's program"
                        } else {
                            "Program"
                        })
                        .value(&format!("{} lines, {}", p.lines, p.post))
                        .into(),
                    );
                }
                out.push(stats("op_stats", lines));
                if let Some(text) = about_it {
                    out.push(about(text));
                }
                for w in &op.warnings {
                    out.push(note(NoteKind::Warning, w.clone()));
                }
                let mut saves = Vec::new();
                if program.is_some() {
                    saves.push(button(
                        "save",
                        "Save G-code…",
                        ButtonStyle::Primary,
                        run.passed,
                    ));
                    if !run.passed {
                        saves.push(button(
                            "save_anyway",
                            "Save anyway, unchecked…",
                            ButtonStyle::Destructive,
                            true,
                        ));
                    }
                }
                saves.push(sheet_button());
                out.push(buttons(saves));
            }
        }
        None if run.program.is_some() => {
            out.push(machining(run));
            out.push(buttons(vec![
                button(
                    "save_setup",
                    "Save G-code…",
                    ButtonStyle::Primary,
                    run.passed,
                ),
                sheet_button(),
            ]));
        }
        None => {
            out.push(machining(run));
            let programs = run
                .operations
                .iter()
                .filter(|o| o.program.is_some())
                .count();
            let mut saves = Vec::new();
            if programs > 1 {
                saves.push(button(
                    "save_all",
                    "Save every program…",
                    ButtonStyle::Primary,
                    run.passed,
                ));
            }
            for (i, op) in run.operations.iter().enumerate() {
                if op.program.is_some() {
                    saves.push(button(
                        &format!("save:{i}"),
                        &format!("Save {}…", op.name),
                        ButtonStyle::Secondary,
                        run.passed,
                    ));
                }
            }
            saves.push(sheet_button());
            out.push(buttons(saves));
        }
    }
    out
}

/// The button that saves the setup's sheet, beside its G-code's: a setup
/// that failed has one too, saying so first.
fn sheet_button() -> Widget {
    button(
        "save_sheet",
        "Save setup sheet…",
        ButtonStyle::Secondary,
        true,
    )
}

/// How long a setup machines: each operation, the tool changes and the
/// whole, and what that leaves out.
fn machining(run: &Run) -> Widget {
    let m = &run.machining;
    // With several parts, each operation at each, and the tool's way
    // between them.
    let parts = run.placements.len().max(1);
    let mut rows: Vec<Item> = run
        .operations
        .iter()
        .filter(|o| o.seconds > 0.0)
        .map(|o| {
            pb::item(&o.name)
                .icon(&own_icon("operation"))
                .tone(Tone::Muted)
                .plain_value(&if parts > 1 {
                    format!("{} × {parts}", crate::sheet::duration(o.seconds))
                } else {
                    crate::sheet::duration(o.seconds)
                })
                .into()
        })
        .collect();
    let between = crate::sheet::between_parts(run);
    if between > 0.0 {
        rows.push(stat("Between parts", &crate::sheet::duration(between)));
    }
    rows.push(stat(
        "Tool changes",
        &match m.tool_change {
            Some(each) => format!("{} × {}", m.tool_changes, crate::sheet::duration(each)),
            None => format!("{}, not timed", m.tool_changes),
        },
    ));
    rows.push(
        pb::item("Total")
            .plain_value(&crate::sheet::duration(m.total()))
            .value_tone(Tone::Accent)
            .into(),
    );
    let [x, y, z] = m.rapid.map(crate::sheet::num);
    pb::section("Machining time")
        .summary(&crate::sheet::duration(m.total()))
        .closed()
        .child(stats("machining", rows))
        .child(about(format!(
            "Rapids at X {x}, Y {y}, Z {z} mm/min{}. Nothing speeds up or slows down \
             here, so a machine takes longer.",
            if m.rapid_given {
                ", as the post says"
            } else {
                ", assumed"
            }
        )))
        .into()
}

/// Several parts, each at a work offset of its own: the setup's own first,
/// the others listed, the chosen one's place to type, and which comes first.
fn part_rows(cam: &Cam, setup: &SetupData) -> Vec<Widget> {
    let word = |w: f64| {
        setup
            .work_offsets()
            .into_iter()
            .find(|(n, _)| f64::from(*n) == w)
            .map_or_else(|| format!("G{w}"), |(_, s)| s)
    };
    let mut items = vec![
        pb::item(&setup.wcs_word())
            .icon("body")
            .secondary("where the part is")
            .muted(true)
            .into(),
    ];
    items.extend(setup.parts.iter().enumerate().map(|(i, p)| {
        pb::item(&word(p.wcs))
            .icon("body")
            .value(&format!(
                "{}, {}, {}",
                crate::sheet::num(p.x),
                crate::sheet::num(p.y),
                crate::sheet::num(p.z)
            ))
            .selected(cam.chosen_part == Some(i))
            .menu("remove", "Remove the part")
            .into()
    }));
    let mut rows = vec![pb::items("parts").items(items).into()];
    if let Some((i, p)) = cam.chosen_part.and_then(|i| Some((i, setup.parts.get(i)?))) {
        let offsets = setup.work_offsets();
        let mut words: Vec<String> = offsets.iter().map(|(_, w)| w.clone()).collect();
        let at = offsets
            .iter()
            .position(|(n, _)| f64::from(*n) == p.wcs)
            .unwrap_or_else(|| {
                words.push(word(p.wcs));
                words.len() - 1
            });
        rows.push(pb::overline(&format!("Part {}", i + 2)));
        rows.push(pb::fields([
            choice("part_wcs", "Work offset", words, at),
            unbound("part_x", "X from the first", p.x, Dim::Length, 2),
            unbound("part_y", "Y from the first", p.y, Dim::Length, 2),
            unbound("part_z", "Z from the first", p.z, Dim::Length, 2),
        ]));
    }
    if !setup.parts.is_empty() {
        rows.push(choice(
            "part_order",
            "Order",
            vec![
                "By tool: each tool at every part".into(),
                "By part: every operation at one part".into(),
            ],
            usize::from(setup.part_order == PlacementOrder::ByPart),
        ));
    }
    rows.push(about(format!(
        "The same operations cut at more parts, each at a work offset of its own, as \
         in a second vise: the first at {}, where the part is, and each other as far \
         from it, along the setup's axes, as it says. Each is checked as the first, \
         against the fixtures and the travel where its offset puts it.",
        setup.wcs_word()
    )));
    rows
}

/// The document's bodies `setup` does not machine, by id and name: those
/// its stock may be.
fn other_bodies(setup: &SetupData) -> Vec<(String, String)> {
    host::bodies()
        .into_iter()
        .filter(|b| !setup.bodies.contains(&b.id))
        .map(|b| (b.id, b.name))
        .collect()
}

/// The setups before setup `id` in the document, by id and name: those its
/// stock may carry on from.
fn earlier_setups(id: &str) -> Vec<(String, String)> {
    setups()
        .into_iter()
        .take_while(|(n, _)| n.id != id)
        .map(|(n, _)| (n.id, n.name))
        .collect()
}

/// Bodies as a list's rows, each to be removed from its menu; one no
/// longer in the document is said to be gone.
fn body_items(ids: &[String], bodies: &[Body], lit: Option<&str>) -> Vec<Item> {
    ids.iter()
        .map(|b| {
            let row = match bodies.iter().find(|x| &x.id == b) {
                Some(x) => pb::item(&x.name).icon("body"),
                None => pb::item("A body no longer there")
                    .icon("body")
                    .problem(true),
            };
            row.lit(lit == Some(b.as_str()))
                .menu("remove", "Remove from the setup")
                .into()
        })
        .collect()
}

/// Bodies' names, for a closed fold's summary.
fn body_names(ids: &[String], bodies: &[Body]) -> String {
    let names: Vec<&str> = ids
        .iter()
        .filter_map(|b| bodies.iter().find(|x| &x.id == b).map(|x| x.name.as_str()))
        .collect();
    if names.is_empty() {
        "None".into()
    } else {
        names.join(", ")
    }
}

/// A stock's kind, as the setup's Stock choice lists them.
const STOCKS: [(StockKind, &str); 5] = [
    (StockKind::Around, "A box round the bodies"),
    (StockKind::Fixed, "A box of a fixed size"),
    (StockKind::Cylinder, "Cylinder"),
    (StockKind::Earlier, "From an earlier setup"),
    (StockKind::Solid, "A body (from solid)"),
];

pub fn setup_panel(cam: &Cam, node: &Node) -> Vec<Widget> {
    let Some(setup) = SetupData::from(&node.data) else {
        return vec![note(NoteKind::Error, "The setup's data does not read.")];
    };
    let id = node.id.as_str();
    let bodies = host::bodies();
    let ops = operations_of(id);
    let lit_body = match &cam.lit {
        Some(Lit::Body(b, _)) => Some(b.as_str()),
        _ => None,
    };
    let mut panel = vec![pb::task_header(
        &format!("{} · {}", node.name, setup.post),
        Some(run_status(cam, id, setup.enabled, None)),
    )];
    panel.push(toggle("enabled", "Runs", setup.enabled));
    if !setup.enabled {
        panel.push(note(
            NoteKind::Info,
            "Turned off: none of its operations is run, simulated, checked or posted.",
        ));
    }
    // Made to apply a template to: its fold first, open.
    if cam.template_ui.first {
        panel.push(crate::templates::group(cam, &node.name, ops.len()));
    }
    panel.push(
        pb::section("Bodies")
            .count(setup.bodies.len())
            .summary(&body_names(&setup.bodies, &bodies))
            .action("add_body", "plus", "Add the body selected in the view")
            .child(
                pb::items("bodies")
                    .items(body_items(&setup.bodies, &bodies, lit_body))
                    .empty("None yet: select a body, then add it with +."),
            )
            .into(),
    );

    let mut stock = vec![choice(
        "stock",
        "Stock",
        STOCKS.iter().map(|(_, l)| l.to_string()).collect(),
        STOCKS
            .iter()
            .position(|(k, _)| *k == setup.stock)
            .unwrap_or(0),
    )];
    match setup.stock {
        StockKind::Solid => {
            let others = other_bodies(&setup);
            let mut names = vec!["Choose a body".to_string()];
            names.extend(others.iter().map(|(_, name)| name.clone()));
            let chosen = setup
                .stock_body
                .as_ref()
                .and_then(|b| others.iter().position(|(o, _)| o == b))
                .map_or(0, |i| i + 1);
            stock.push(choice("stock_body", "Body", names, chosen));
            stock.push(about(
                "A body of the document that is the stock: a casting, a blank cut \
                 before, a printed part. Its material is what the setup starts from, \
                 each column of it from its lowest to its highest along the tool; it \
                 is not machined, and a work zero on the stock is on its box.",
            ));
        }
        StockKind::Earlier => {
            let earlier = earlier_setups(id);
            let mut names = vec!["The setup before".to_string()];
            names.extend(earlier.iter().map(|(_, name)| name.clone()));
            let chosen = setup
                .stock_from
                .as_ref()
                .and_then(|from| earlier.iter().position(|(e, _)| e == from))
                .map_or(0, |i| i + 1);
            stock.push(choice("stock_from", "From", names, chosen));
            stock.push(about(
                "What that setup leaves after its last operation, the same material \
                 turned as this setup sees it: the checks, the simulation, playback \
                 and the times start from it, and it runs again when that one changes. \
                 Seen down this setup's tool, a pocket hidden under material is taken \
                 as material.",
            ));
            if earlier.is_empty() {
                stock.push(note(
                    NoteKind::Warning,
                    "There is no setup before this one in the document.",
                ));
            }
        }
        StockKind::Around => {
            stock.push(number(
                "margin_side",
                "Margin, sides",
                setup.margin_side,
                Dim::Length,
                2,
                id,
            ));
            stock.push(number(
                "margin_top",
                "Margin, top",
                setup.margin_top,
                Dim::Length,
                2,
                id,
            ));
            stock.push(number(
                "margin_below",
                "Margin, below",
                setup.margin_below,
                Dim::Length,
                2,
                id,
            ));
        }
        StockKind::Fixed => {
            stock.push(number("size_x", "Size X", setup.size_x, Dim::Length, 2, id));
            stock.push(number("size_y", "Size Y", setup.size_y, Dim::Length, 2, id));
            stock.push(number("size_z", "Size Z", setup.size_z, Dim::Length, 2, id));
            stock.push(about("Centred on the bodies, its top flush with theirs."));
        }
        StockKind::Cylinder => {
            stock.push(choice(
                "cylinder_axis",
                "Axis",
                CylinderAxis::ALL.iter().map(|a| a.label().into()).collect(),
                CylinderAxis::ALL
                    .iter()
                    .position(|&a| a == setup.cylinder_axis)
                    .unwrap_or(0),
            ));
            stock.push(choice(
                "cylinder_size",
                "Size",
                vec!["Round the bodies".into(), "Fixed".into()],
                usize::from(setup.cylinder_fixed),
            ));
            if setup.cylinder_fixed {
                stock.push(number(
                    "diameter",
                    "Diameter",
                    setup.diameter,
                    Dim::Length,
                    2,
                    id,
                ));
                stock.push(number("length", "Length", setup.length, Dim::Length, 2, id));
                stock.push(about(if setup.cylinder_axis == CylinderAxis::Tool {
                    "Centred on the bodies, its top flush with theirs."
                } else {
                    "Centred on the bodies."
                }));
            } else {
                stock.push(number(
                    "margin_radial",
                    "Margin, round",
                    setup.margin_radial,
                    Dim::Length,
                    2,
                    id,
                ));
                stock.push(number(
                    "margin_ends",
                    "Margin, each end",
                    setup.margin_ends,
                    Dim::Length,
                    2,
                    id,
                ));
            }
            if setup.cylinder_axis != CylinderAxis::Tool {
                stock.extend(rotary_rows(id, &setup));
            }
        }
    }
    stock.push(pb::entry(
        "material",
        "Material",
        &setup.material,
        "Aluminium 6061, MDF…",
    ));
    let stock_kind = STOCKS
        .iter()
        .find(|(k, _)| *k == setup.stock)
        .map_or("", |(_, l)| l);
    let stock_summary = if setup.material.is_empty() {
        stock_kind.to_string()
    } else {
        format!("{stock_kind} · {}", setup.material)
    };
    panel.push(
        pb::section("Stock")
            .summary(&stock_summary)
            .children(stock)
            .into(),
    );

    panel.push(
        pb::section("Fixtures")
            .count(setup.fixtures.len())
            .summary(&body_names(&setup.fixtures, &bodies))
            .closed()
            .action(
                "add_fixture",
                "plus",
                "Add the body selected in the view as a fixture",
            )
            .child(
                pb::items("fixtures")
                    .items(body_items(&setup.fixtures, &bodies, lit_body))
                    .empty("None: select a vise jaw or a clamp, then add it with +."),
            )
            .child(about(
                "Bodies that hold the part: vise jaws, clamps, a board. They are never \
                 cut: the tool, its shank or its holder meeting one on any move, cutting \
                 or rapid, is a failed check that names it and the move.",
            ))
            .into(),
    );

    let mut orientation = orientation_rows(cam, id, &setup);
    orientation.push(choice(
        "zero",
        "Work zero",
        Zero::ALL.iter().map(|z| z.label().to_string()).collect(),
        Zero::ALL.iter().position(|&z| z == setup.zero).unwrap_or(0),
    ));
    if setup.zero == Zero::Picked {
        orientation.push(chip(
            "zero_pick",
            "On the part",
            setup.zero_point.as_ref().map(|z| {
                format!(
                    "{} at {:.2}, {:.2}, {:.2}",
                    if z.snap == camcraft_core::job::Snap::Vertex {
                        "Corner"
                    } else {
                        "Centre"
                    },
                    z.at[0],
                    z.at[1],
                    z.at[2]
                )
            }),
            cam.armed
                .as_ref()
                .is_some_and(|a| a.feature == id && a.slot == Slot::Zero),
        ));
    }
    orientation.extend([
        number("offset_x", "Offset X", setup.offset_x, Dim::Length, 2, id),
        number("offset_y", "Offset Y", setup.offset_y, Dim::Length, 2, id),
        number("offset_z", "Offset Z", setup.offset_z, Dim::Length, 2, id),
    ]);
    let axis = match setup.tool_axis {
        ToolAxis::Square => format!("Z {}", setup.z_axis.label()),
        t => t.label().to_string(),
    };
    panel.push(
        pb::section("Orientation & work zero")
            .id("orientation")
            .summary(&format!("{axis} · {}", setup.zero.label()))
            .children(orientation)
            .into(),
    );

    let mut posts = posts(cam);
    // A post of someone else's the setup carries, though this library has
    // not got it.
    if !posts.contains(&setup.post) {
        posts.push(setup.post.clone());
    }
    panel.push(
        pb::section("Post")
            .summary(&format!(
                "{} · {}",
                setup.post,
                if setup.programs == Programs::Operation {
                    "a program each"
                } else {
                    "one program"
                }
            ))
            .children([
                choice(
                    "post",
                    "Machine",
                    posts.clone(),
                    posts.iter().position(|p| *p == setup.post).unwrap_or(0),
                ),
                choice(
                    "programs",
                    "Programs",
                    vec!["One for the setup".into(), "One per operation".into()],
                    usize::from(setup.programs == Programs::Operation),
                ),
                number(
                    "program_number",
                    "Program number",
                    setup.program_number,
                    Dim::Number,
                    0,
                    id,
                ),
                number(
                    "tolerance",
                    "Tolerance",
                    setup.tolerance,
                    Dim::Length,
                    3,
                    id,
                ),
            ])
            .into(),
    );
    panel.push(
        pb::section("Work offset")
            .summary(&if setup.wcs_known {
                format!("{} · placed on the machine", setup.wcs_word())
            } else {
                setup.wcs_word()
            })
            .closed()
            .children(work_offset_rows(id, &setup))
            .into(),
    );
    let parts = setup.parts.len() + 1;
    panel.push(
        pb::section("Parts")
            .count(parts)
            .summary(&if parts == 1 {
                "One part".to_string()
            } else {
                format!(
                    "{parts} parts · {}",
                    if setup.part_order == PlacementOrder::ByPart {
                        "by part"
                    } else {
                        "by tool"
                    }
                )
            })
            .closed()
            .action("add_part", "plus", "Add a part at a work offset of its own")
            .children(part_rows(cam, &setup))
            .into(),
    );
    panel.push(
        pb::section("Probed surface")
            .id("probe")
            .summary(if setup.probe_correct {
                if setup.probe_file.is_empty() {
                    "On · no results"
                } else {
                    &setup.probe_file
                }
            } else {
                "Off"
            })
            .closed()
            .children(probe_rows(id, &setup))
            .into(),
    );

    let run = cam.runs.get(id).and_then(|c| c.shown());
    let lit_op = match &cam.lit {
        Some(Lit::Operation(s, i)) if s == id => Some(*i),
        _ => None,
    };
    let items = ops.iter().enumerate().map(|(i, (n, d))| {
        let on = d.operation.enabled;
        let strategy = crate::data::strategy_word(&d.strategy());
        let mut row = pb::item(&n.name)
            .icon(&own_icon(if on { "operation" } else { "operation-off" }))
            .muted(!on)
            .secondary(strategy)
            .lit(lit_op == Some(i))
            .menu("open", "Open")
            .menu("runs", if on { "Turn off" } else { "Turn on" });
        row = match run.and_then(|r| r.operations.get(i)) {
            Some(o) if o.made == "generated" => row
                .plain_value(&crate::sheet::duration(o.seconds))
                .value_hint(&format!("{:.0} mm cut", o.cut_mm)),
            Some(o) if o.made == "custom-gcode" => row.plain_value("G-code").value_hint(&o.summary),
            Some(o) => row.plain_value(&o.made).value_tone(Tone::Muted),
            None => row,
        };
        if run.is_some_and(|r| {
            r.findings
                .iter()
                .any(|f| f.level == Level::Fail && f.operation == Some(i))
        }) {
            row = row.badge("error", Tone::Bad, "A check of it failed");
        }
        row.into()
    });
    panel.push(
        pb::section("Operations")
            .count(ops.len())
            .child(
                pb::items("operations")
                    .items(items)
                    .empty("None yet: choose a strategy and add one."),
            )
            .child(pb::flow_no_wrap([
                pb::fill(choice(
                    "new_strategy",
                    "",
                    STRATEGIES.iter().map(|(_, l)| l.to_string()).collect(),
                    cam.new_strategy,
                )),
                button("add_operation", "Add", ButtonStyle::Secondary, true),
            ]))
            .into(),
    );
    if !cam.template_ui.first {
        panel.push(crate::templates::group(cam, &node.name, ops.len()));
    }
    if !setup.tools.is_empty() {
        let tools = setup.tools.iter().map(|(name, t)| {
            let row = pb::item(name)
                .icon(&own_icon("tool"))
                .secondary(&format!("{} Ø{}", kind_label(t.kind), t.diameter))
                .value(&format!("T{}", t.number));
            match run.and_then(|r| r.tools.iter().find(|u| &u.name == name)) {
                Some(used) => row
                    .hint(&format!(
                        "{} · {}",
                        tool_line(t),
                        used.operations
                            .iter()
                            .filter_map(|&k| ops.get(k).map(|(n, _)| n.name.as_str()))
                            .collect::<Vec<_>>()
                            .join(", "),
                    ))
                    .flag(pb::status(
                        &format!(
                            "Z{}",
                            used.lowest.map_or_else(|| "?".into(), crate::sheet::num)
                        ),
                        Tone::Muted,
                        "The lowest it goes",
                    ))
                    .into(),
                None => row.hint(&tool_line(t)).into(),
            }
        });
        panel.push(
            pb::section("Tools")
                .count(setup.tools.len())
                .summary(
                    &setup
                        .tools
                        .values()
                        .map(|t| format!("T{}", t.number))
                        .collect::<Vec<_>>()
                        .join(" "),
                )
                .closed()
                .child(pb::items("tools").items(tools))
                .into(),
        );
    }
    if let Some(hint) = &cam.hint {
        panel.push(note(NoteKind::Info, hint.clone()));
    }
    let mut results = vec![toggle(
        "show_material",
        "Show the material left",
        cam.show_material,
    )];
    results.extend(run_widgets(cam, id, None));
    panel.push(pb::section("Run").children(results).into());
    panel
}

/// Whether a setup's lying cylinder is turned on a rotary, and the diameter
/// its flat paths are wrapped round at.
fn rotary_rows(id: &str, setup: &SetupData) -> Vec<Widget> {
    let mut rows = vec![toggle("wrap", "On a rotary (4th axis)", setup.wrap)];
    if !setup.wrap {
        return rows;
    }
    let (along, word) = if setup.cylinder_axis == CylinderAxis::Y {
        ("Y", "B")
    } else {
        ("X", "A")
    };
    rows.push(number(
        "wrap_diameter",
        "Wrap diameter",
        setup.wrap_diameter,
        Dim::Length,
        2,
        id,
    ));
    rows.push(about(format!(
        "Turned by {word} about {along}. Sketches are wrapped round it as drawn, flat over \
         its top: across {along} is the distance round at the wrap diameter (0: the \
         stock's). Parallel finishing goes along it (angle 0) or round it in a helix \
         (90). The work zero is on the axis."
    )));
    rows
}

/// Whether a setup's cut depths are corrected to probe results, and from
/// which.
fn probe_rows(id: &str, setup: &SetupData) -> Vec<Widget> {
    let mut rows = vec![toggle(
        "probe_correct",
        "Correct Z from probe results",
        setup.probe_correct,
    )];
    if !setup.probe_correct {
        return rows;
    }
    let units = match setup.profile().map(|p| p.units) {
        Some(camcraft_post::profile::ProgramUnits::Inch) => "in",
        _ => "mm",
    };
    let (results, tone) = if !setup.probe_points.is_empty() {
        match camcraft_core::probe::ProbeMap::from_points(&setup.probe_points, 1.0) {
            Ok(map) => {
                let (columns, rows) = map.size();
                let (low, high) = map.heights();
                (
                    format!("{columns} x {rows} points, Z {low:.3} to {high:.3} {units}"),
                    Tone::Neutral,
                )
            }
            Err(e) => (e.to_string(), Tone::Bad),
        }
    } else if !setup.probe_file.is_empty() {
        ("read each run".into(), Tone::Neutral)
    } else {
        ("none loaded".into(), Tone::Muted)
    };
    let file = if setup.probe_file.is_empty() {
        "Results"
    } else {
        &setup.probe_file
    };
    rows.push(
        pb::item(file)
            .icon("file-document")
            .plain_value(&results)
            .value_tone(tone)
            .id("probe_results")
            .into(),
    );
    rows.push(button(
        "load_probe",
        "Load probe results…",
        ButtonStyle::Secondary,
        true,
    ));
    rows.push(number(
        "probe_segment",
        "Segment length",
        setup.probe_segment,
        Dim::Length,
        2,
        id,
    ));
    rows.push(about(format!(
        "Every move of every operation but a probe's goes up or down by the probed \
         surface's height there, from the work zero, bilinearly between the points; \
         a cut is split into pieces no longer than the segment length, and an arc \
         into straight pieces where the surface is not level under it. Outside the \
         probed area the surface is taken as at its nearest edge, and the operation \
         says how far it went. The stock's top and the part follow it in the \
         simulation and the checks. Results are a LinuxCNC probe log (x y z ...) or \
         a CSV of x, y, z, in the post's units ({units}) and the setup's work \
         coordinates."
    )));
    rows
}

/// The work offset a setup's programs select, from those its post has, and
/// where it is on the machine, for checking the post's travel there.
fn work_offset_rows(id: &str, setup: &SetupData) -> Vec<Widget> {
    let mut offsets: Vec<String> = setup.work_offsets().into_iter().map(|(_, w)| w).collect();
    let at = setup
        .work_offsets()
        .iter()
        .position(|(n, _)| f64::from(*n) == setup.wcs);
    // One its post has not got, kept so it shows; the setup says so.
    let selected = at.unwrap_or_else(|| {
        offsets.push(format!("{} (not on this post)", setup.wcs_word()));
        offsets.len() - 1
    });
    let word = setup.wcs_word();
    let mut rows = vec![
        choice("wcs", "Work offset", offsets, selected),
        toggle(
            "wcs_known",
            format!("Say where {word} is on the machine"),
            setup.wcs_known,
        ),
    ];
    let travel = setup.profile().and_then(|p| p.travel);
    let machine =
        travel.is_some_and(|t| t.coordinates == camcraft_post::profile::Coordinates::Machine);
    if setup.wcs_known {
        rows.extend([
            number("wcs_x", "Machine X", setup.wcs_x, Dim::Length, 3, id),
            number("wcs_y", "Machine Y", setup.wcs_y, Dim::Length, 3, id),
            number("wcs_z", "Machine Z", setup.wcs_z, Dim::Length, 3, id),
        ]);
        let z = if setup.profile().is_some_and(|p| p.length_offset) {
            "Z is the spindle's gauge line at the work zero (G43 adds each tool's length)"
        } else {
            "Z is the tool's tip at the work zero, touched off"
        };
        rows.push(about(format!(
            "The work zero in machine coordinates, as {word} holds it, along the \
             setup's axes (for a tilted setup, with the part turned). {z}."
        )));
    }
    rows.push(about(match (travel, machine, setup.wcs_known) {
        (None, ..) => "Its post gives no travel: nothing is checked against it.".to_string(),
        (Some(_), false, _) => {
            "Its post's travel is from the work zero, not in machine coordinates.".into()
        }
        (Some(_), true, true) => "Its programs are checked against its post's travel there.".into(),
        (Some(_), true, false) => format!(
            "Its post's travel is in machine coordinates: say where {word} is to check \
             it."
        ),
    }));
    rows
}

pub fn setup_event(cam: &mut Cam, id: &str, event: PanelEvent) {
    let Some((node, mut setup)) = setup_node(id) else {
        return;
    };
    if crate::templates::event(cam, id, &node.name, &event) {
        return;
    }
    match event {
        PanelEvent::Number { id: field, value } => {
            let slot = match field.as_str() {
                "margin_side" => &mut setup.margin_side,
                "margin_top" => &mut setup.margin_top,
                "margin_below" => &mut setup.margin_below,
                "size_x" => &mut setup.size_x,
                "size_y" => &mut setup.size_y,
                "size_z" => &mut setup.size_z,
                "margin_radial" => &mut setup.margin_radial,
                "margin_ends" => &mut setup.margin_ends,
                "diameter" => &mut setup.diameter,
                "length" => &mut setup.length,
                "offset_x" => &mut setup.offset_x,
                "offset_y" => &mut setup.offset_y,
                "offset_z" => &mut setup.offset_z,
                "tilt_x" => &mut setup.tilt_x,
                "tilt_y" => &mut setup.tilt_y,
                "axis_dx" => &mut setup.axis_dx,
                "axis_dy" => &mut setup.axis_dy,
                "axis_dz" => &mut setup.axis_dz,
                "wcs_x" => &mut setup.wcs_x,
                "wcs_y" => &mut setup.wcs_y,
                "wcs_z" => &mut setup.wcs_z,
                "program_number" => &mut setup.program_number,
                "tolerance" => &mut setup.tolerance,
                "probe_segment" => &mut setup.probe_segment,
                "wrap_diameter" => &mut setup.wrap_diameter,
                part => {
                    // The chosen part's place, from the first.
                    let Some(p) = cam.chosen_part.and_then(|i| setup.parts.get_mut(i)) else {
                        return;
                    };
                    match part {
                        "part_x" => &mut p.x,
                        "part_y" => &mut p.y,
                        "part_z" => &mut p.z,
                        _ => return,
                    }
                }
            };
            *slot = value;
            cam.write_setup(id, &setup);
        }
        PanelEvent::Choice { id: field, index } => {
            match field.as_str() {
                "stock" => {
                    setup.stock = match index {
                        1 => StockKind::Fixed,
                        2 => StockKind::Cylinder,
                        3 => StockKind::Earlier,
                        4 => StockKind::Solid,
                        _ => StockKind::Around,
                    }
                }
                "stock_body" => {
                    setup.stock_body = index
                        .checked_sub(1)
                        .and_then(|i| other_bodies(&setup).into_iter().nth(i))
                        .map(|(b, _)| b);
                }
                "stock_from" => {
                    setup.stock_from = index
                        .checked_sub(1)
                        .and_then(|i| earlier_setups(id).into_iter().nth(i))
                        .map(|(e, _)| e);
                }
                "cylinder_axis" => {
                    setup.cylinder_axis = CylinderAxis::ALL.get(index).copied().unwrap_or_default();
                }
                "cylinder_size" => setup.cylinder_fixed = index == 1,
                "z_axis" => match Axis::ALL.get(index) {
                    Some(&z) => {
                        setup.tool_axis = ToolAxis::Square;
                        setup.z_axis = z;
                        setup.square_axes();
                    }
                    None => {
                        let t = ToolAxis::TILTED
                            .get(index - Axis::ALL.len())
                            .copied()
                            .unwrap_or(ToolAxis::Tilt);
                        if t != setup.tool_axis {
                            setup.axis_pick = None;
                        }
                        setup.tool_axis = t;
                        // A face or an edge to pick: pick it now.
                        match t {
                            ToolAxis::Face => cam.arm(id, Some(Slot::AxisFace)),
                            ToolAxis::Edge => cam.arm(id, Some(Slot::AxisEdge)),
                            _ => {}
                        }
                    }
                },
                "x_axis" => match machine_xs(&setup).get(index) {
                    Some(&x) => {
                        setup.machine_x = MachineX::Axis;
                        setup.x_axis = x;
                    }
                    None => {
                        setup.machine_x = MachineX::Edge;
                        if setup.x_pick.is_none() {
                            cam.arm(id, Some(Slot::XEdge));
                        }
                    }
                },
                "zero" => {
                    setup.zero = Zero::ALL.get(index).copied().unwrap_or_default();
                    // A zero to pick: pick it now.
                    if setup.zero == Zero::Picked && setup.zero_point.is_none() {
                        cam.arm(id, Some(Slot::Zero));
                    }
                }
                "post" => {
                    let mut posts = posts(cam);
                    if !posts.contains(&setup.post) {
                        posts.push(setup.post.clone());
                    }
                    let Some(p) = posts.get(index).cloned() else {
                        return;
                    };
                    if p != setup.post {
                        setup.post_profile = cam.settings.posts.get(&p).cloned();
                        setup.post = p;
                    }
                }
                "wcs" => match setup.work_offsets().get(index) {
                    Some(&(n, _)) => setup.wcs = f64::from(n),
                    // The one it had, which its post has not got.
                    None => return,
                },
                "part_wcs" => {
                    let Some(&(n, _)) = setup.work_offsets().get(index) else {
                        return;
                    };
                    let Some(p) = cam.chosen_part.and_then(|i| setup.parts.get_mut(i)) else {
                        return;
                    };
                    p.wcs = f64::from(n);
                }
                "part_order" => {
                    setup.part_order = if index == 1 {
                        PlacementOrder::ByPart
                    } else {
                        PlacementOrder::ByTool
                    };
                }
                "programs" => {
                    setup.programs = if index == 1 {
                        Programs::Operation
                    } else {
                        Programs::Setup
                    };
                }
                "new_strategy" => {
                    cam.new_strategy = index.min(STRATEGIES.len() - 1);
                    return;
                }
                _ => return,
            }
            cam.write_setup(id, &setup);
        }
        // The setup's own part is the list's first.
        PanelEvent::Select { id: list, index } if list == "parts" => {
            cam.chosen_part = index.checked_sub(1);
        }
        PanelEvent::Button { id: b } if b == "add_part" => match setup.next_part(100.0) {
            Some(p) => {
                setup.parts.push(p);
                cam.write_setup(id, &setup);
            }
            None => cam.hint = Some("Every work offset the post has is taken.".into()),
        },
        PanelEvent::ContextMenu {
            id: list, index, ..
        } if list == "parts" => {
            if let Some(i) = index.checked_sub(1)
                && i < setup.parts.len()
            {
                setup.parts.remove(i);
                cam.chosen_part = None;
                cam.write_setup(id, &setup);
            }
        }
        PanelEvent::Button { id: b } if b == "add_fixture" => {
            match cam
                .pointer
                .selected_body
                .clone()
                .or(cam.pointer.body.clone())
            {
                Some(body) if setup.bodies.contains(&body) => {
                    cam.hint = Some("The setup machines that body: it cannot hold it.".into());
                }
                Some(body) if !setup.fixtures.contains(&body) => {
                    setup.fixtures.push(body);
                    cam.write_setup(id, &setup);
                }
                Some(_) => cam.hint = Some("That body is a fixture already.".into()),
                None => cam.hint = Some("Select a body in the view or the tree first.".into()),
            }
        }
        PanelEvent::ContextMenu {
            id: list, index, ..
        } if list == "fixtures" || list == "bodies" => {
            let from = if list == "fixtures" {
                &mut setup.fixtures
            } else {
                &mut setup.bodies
            };
            if index < from.len() {
                from.remove(index);
                cam.lit = None;
                cam.write_setup(id, &setup);
            }
        }
        PanelEvent::Hover { id: list, index } if list == "fixtures" || list == "bodies" => {
            let from = if list == "fixtures" {
                &setup.fixtures
            } else {
                &setup.bodies
            };
            cam.light_body(index.and_then(|i| from.get(i)));
        }
        PanelEvent::Select { id: list, index } | PanelEvent::Activate { id: list, index }
            if list == "operations" =>
        {
            if let Some((op, _)) = operations_of(id).get(index) {
                let op = op.id.clone();
                cam.open(&op, false);
            }
        }
        PanelEvent::ContextMenu {
            id: list,
            index,
            action,
        } if list == "operations" => {
            let Some((op, mut data)) = operations_of(id).into_iter().nth(index) else {
                return;
            };
            match action.as_str() {
                "open" => cam.open(&op.id, false),
                "runs" => {
                    data.operation.enabled = !data.operation.enabled;
                    cam.write_operation(&op.id, &data);
                }
                _ => {}
            }
        }
        PanelEvent::Hover { id: list, index } if list == "operations" => {
            cam.lit = index.map(|i| Lit::Operation(id.to_string(), i));
        }
        PanelEvent::Pick { id: pick } if pick == "zero_pick" => cam.arm(id, Some(Slot::Zero)),
        PanelEvent::Clear { id: pick } => {
            match pick.as_str() {
                "zero_pick" => setup.zero_point = None,
                "axis_face" | "axis_edge" => setup.axis_pick = None,
                "x_edge" => setup.x_pick = None,
                _ => return,
            }
            cam.write_setup(id, &setup);
        }
        PanelEvent::Pick { id: pick } if Slot::named(&pick).is_some() => {
            cam.arm(id, Slot::named(&pick));
        }
        PanelEvent::Toggle { id: t, on } if t == "axis_reverse" || t == "x_reverse" => {
            let pick = if t == "axis_reverse" {
                &mut setup.axis_pick
            } else {
                &mut setup.x_pick
            };
            if let Some(p) = pick {
                p.reverse = on;
                cam.write_setup(id, &setup);
            }
        }
        PanelEvent::Button { id: b } if b == "add_body" => {
            match cam
                .pointer
                .selected_body
                .clone()
                .or(cam.pointer.body.clone())
            {
                Some(body) if !setup.bodies.contains(&body) => {
                    setup.bodies.push(body);
                    cam.write_setup(id, &setup);
                }
                Some(_) => cam.hint = Some("The setup machines that body already.".into()),
                None => cam.hint = Some("Select a body in the view or the tree first.".into()),
            }
        }
        PanelEvent::Button { id: b } if b == "add_operation" => {
            let strategy = STRATEGIES
                .get(cam.new_strategy)
                .and_then(|(n, _)| StrategySpec::named(n));
            if let Some(strategy) = strategy {
                match cam.make_operation(id, strategy, None) {
                    Ok(op) => cam.open(&op, true),
                    Err(e) => cam.hint = Some(e),
                }
            }
        }
        PanelEvent::Toggle { id: t, on } if t == "enabled" => {
            setup.enabled = on;
            cam.write_setup(id, &setup);
        }
        PanelEvent::Toggle { id: t, on } if t == "wcs_known" => {
            setup.wcs_known = on;
            cam.write_setup(id, &setup);
        }
        PanelEvent::Toggle { id: t, on } if t == "wrap" => {
            setup.wrap = on;
            cam.write_setup(id, &setup);
        }
        PanelEvent::Toggle { id: t, on } if t == "probe_correct" => {
            setup.probe_correct = on;
            cam.write_setup(id, &setup);
        }
        PanelEvent::Button { id: b } if b == "load_probe" => {
            host::request(crate::tools::open_probe_results(id));
        }
        PanelEvent::Toggle { id: t, on } if t == "show_material" => cam.show_material = on,
        PanelEvent::Button { id: b } if b == "generate" => cam.refresh(id, true),
        PanelEvent::Button { id: b } if b == "stop" => cam.runs.stop(id),
        PanelEvent::Text { id: t, value } if t == "material" => {
            setup.material = value.trim().to_string();
            cam.write_setup(id, &setup);
        }
        PanelEvent::Button { id: b } if b == "save_sheet" => match cam.sheet_of(id) {
            Ok((name, html)) => Cam::save_sheet(&name, html),
            Err(e) => cam.hint = Some(e),
        },
        PanelEvent::Button { id: b } if b == "save_setup" => match cam.program_of(id, false) {
            Ok(p) => Cam::save(&p),
            Err(e) => cam.hint = Some(e),
        },
        PanelEvent::Button { id: b } if b == "save_all" => {
            let programs: Result<Vec<_>, String> = operations_of(id)
                .iter()
                // One turned off has no program, and is left out.
                .filter(|(_, data)| data.operation.enabled)
                .filter_map(|(op, _)| match cam.program_of(&op.id, false) {
                    Err(e) if e.ends_with("made no toolpath.") => None,
                    other => Some(other),
                })
                .collect();
            match programs {
                Ok(p) if !p.is_empty() => Cam::save_all(&p),
                Ok(_) => cam.hint = Some("No operation made a program.".into()),
                Err(e) => cam.hint = Some(e),
            }
        }
        PanelEvent::Button { id: b } => {
            if let Some(i) = b
                .strip_prefix("save:")
                .and_then(|i| i.parse::<usize>().ok())
                && let Some((op, _)) = operations_of(id).get(i)
            {
                match cam.program_of(&op.id, false) {
                    Ok(p) => Cam::save(&p),
                    Err(e) => cam.hint = Some(e),
                }
            }
        }
        _ => {}
    }
}

/// The tools an operation may pick: its setup's, then the library's it does not
/// have, as the panel lists them, each with its name and whether it is the
/// library's.
fn tool_choices(cam: &Cam, setup: Option<&SetupData>) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = setup
        .map(|s| s.tools.keys().map(|n| (n.clone(), false)).collect())
        .unwrap_or_default();
    for (name, tool) in &cam.settings.tools {
        if setup.is_some_and(|s| s.tools.get(name) == Some(tool)) {
            continue;
        }
        out.push((name.clone(), true));
    }
    out
}

/// The cut direction a strategy has a choice of, if it has one.
fn direction_mut(s: &mut StrategySpec) -> Option<&mut CutDirection> {
    match s {
        StrategySpec::Scallop { direction, .. }
        | StrategySpec::Contour { direction, .. }
        | StrategySpec::Flat { direction, .. }
        | StrategySpec::Spiral { direction, .. }
        | StrategySpec::MorphedSpiral { direction, .. }
        | StrategySpec::Adaptive { direction, .. } => Some(direction),
        _ => None,
    }
}

fn conventional_mut(s: &mut StrategySpec) -> Option<&mut bool> {
    match s {
        StrategySpec::Profile { conventional, .. }
        | StrategySpec::Pocket { conventional }
        | StrategySpec::PocketClearing { conventional, .. }
        | StrategySpec::Bore { conventional, .. }
        | StrategySpec::Circular { conventional, .. }
        | StrategySpec::Thread { conventional, .. } => Some(conventional),
        _ => None,
    }
}

const HOLE_ORDERS: [(HoleOrder, &str); 3] = [
    (HoleOrder::Nearest, "Nearest first"),
    (HoleOrder::XThenY, "By X, then Y, back and forth"),
    (HoleOrder::AsPicked, "As picked"),
];

const DIRECTIONS: [(CutDirection, &str); 3] = [
    (CutDirection::Climb, "Climb"),
    (CutDirection::Conventional, "Conventional"),
    (CutDirection::BothWays, "Both ways"),
];
const SIDES: [(ProfileSide, &str); 3] = [
    (ProfileSide::Outside, "Outside the outline"),
    (ProfileSide::Inside, "Inside the outline"),
    (ProfileSide::On, "On the outline"),
];
const BOUNDARIES: [(Boundary, &str); 4] = [
    (Boundary::Silhouette, "The part's silhouette"),
    (Boundary::BoundingBox, "The part's bounding box"),
    (Boundary::Stock, "The stock"),
    (Boundary::Selection, "The picked faces and edges"),
];
const CONTAINMENTS: [(Containment, &str); 3] = [
    (Containment::Center, "Tool centre on it"),
    (Containment::Inside, "Tool inside it"),
    (Containment::Outside, "Tool outside it"),
];

/// What a height may be measured from, in the order the panel lists them.
const HEIGHT_REFS: [(HeightRef, &str); 6] = [
    (HeightRef::StockTop, "The stock's top"),
    (HeightRef::StockBottom, "The stock's bottom"),
    (HeightRef::ModelTop, "The part's top"),
    (HeightRef::ModelBottom, "The part's bottom"),
    (HeightRef::Absolute, "The work zero"),
    (HeightRef::Selection, "The picks"),
];

/// The heights the panel shows, by the key their fields and choices go by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeightKey {
    Clearance,
    Retract,
    Top,
    Bottom,
}

impl HeightKey {
    fn named(key: &str) -> Option<Self> {
        Some(match key {
            "clearance" => Self::Clearance,
            "retract" => Self::Retract,
            "top" => Self::Top,
            "bottom" => Self::Bottom,
            _ => return None,
        })
    }

    fn key(self) -> &'static str {
        match self {
            Self::Clearance => "clearance",
            Self::Retract => "retract",
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Clearance => "Clearance",
            Self::Retract => "Retract",
            Self::Top => "Top",
            Self::Bottom => "Bottom",
        }
    }

    /// What it is measured from, `None` for the strategy's own.
    fn from(self, data: &OperationData) -> Option<HeightRef> {
        let h = &data.heights;
        match self {
            Self::Clearance => Some(h.clearance_from),
            Self::Retract => Some(h.retract_from),
            Self::Top => h.top_from,
            Self::Bottom => h.bottom_from,
        }
    }

    fn offset(self, data: &OperationData) -> f64 {
        let h = &data.heights;
        match self {
            Self::Clearance => h.clearance,
            Self::Retract => h.retract,
            Self::Top => h.top,
            Self::Bottom => h.bottom,
        }
    }

    fn offset_mut(self, data: &mut OperationData) -> &mut f64 {
        let h = &mut data.heights;
        match self {
            Self::Clearance => &mut h.clearance,
            Self::Retract => &mut h.retract,
            Self::Top => &mut h.top,
            Self::Bottom => &mut h.bottom,
        }
    }

    /// What it is with none given, for a top or a bottom; `None` when it must
    /// be given.
    fn unset(self, data: &OperationData) -> Option<HeightRef> {
        match self {
            Self::Clearance | Self::Retract => None,
            Self::Top => data.strategy().uses().top,
            Self::Bottom => data.bottom_unset(),
        }
    }

    /// The choices of what it is measured from: the strategy's own first for
    /// a top or a bottom, the picks only where it may be measured from them,
    /// or already is.
    fn choices(self, data: &OperationData) -> Vec<Option<HeightRef>> {
        let mut out = Vec::new();
        if self.unset(data).is_some() {
            out.push(None);
        }
        let picks = matches!(self, Self::Top | Self::Bottom) && data.can_measure_from_picks();
        out.extend(
            HEIGHT_REFS
                .iter()
                .map(|(r, _)| *r)
                .filter(|r| *r != HeightRef::Selection || picks || self.from(data) == Some(*r))
                .map(Some),
        );
        out
    }

    /// The heights the strategy has.
    fn shown(data: &OperationData) -> Vec<Self> {
        let uses = data.strategy().uses();
        let mut out = vec![Self::Clearance];
        // A probe comes down to touch from the retract height.
        if uses.stay_down || matches!(data.strategy(), StrategySpec::Probe { .. }) {
            out.push(Self::Retract);
        }
        if uses.top.is_some() {
            out.push(Self::Top);
        }
        if data.bottom_unset().is_some() {
            out.push(Self::Bottom);
        }
        out
    }
}

/// What a top or a bottom is with none given, in words.
fn unset_words(r: HeightRef) -> &'static str {
    match r {
        HeightRef::StockTop => "the stock's top",
        HeightRef::ModelTop => "the part's top",
        HeightRef::Selection => "the picks",
        _ => "the part's bottom",
    }
}

/// The Heights fold: for each height the strategy has, what it is measured
/// from and, when it is given, its offset.
fn heights_group(data: &OperationData, feature: &str) -> Widget {
    let mut rows = Vec::new();
    let mut summary = Vec::new();
    for h in HeightKey::shown(data) {
        let from = h.from(data);
        let choices = h.choices(data);
        let options = choices
            .iter()
            .map(|c| match c {
                Some(r) => HEIGHT_REFS
                    .iter()
                    .find(|(x, _)| x == r)
                    .map_or("?", |(_, l)| l)
                    .to_string(),
                None => format!(
                    "The strategy's own: {}",
                    h.unset(data).map_or("", unset_words)
                ),
            })
            .collect();
        let at = choices.iter().position(|c| *c == from).unwrap_or(0);
        rows.push(choice(
            &format!("{}_from", h.key()),
            &format!("{} from", h.label()),
            options,
            at,
        ));
        if from.is_some() {
            rows.push(number(
                &format!("heights/{}", h.key()),
                &format!("{}, above it", h.label()),
                h.offset(data),
                Dim::Length,
                2,
                feature,
            ));
            summary.push(format!(
                "{} {}",
                h.label(),
                crate::sheet::num(h.offset(data))
            ));
        }
    }
    let uses = data.strategy().uses();
    let about_it = match data.strategy() {
        StrategySpec::Chamfer { .. } => Some(
            "It has no top or bottom: each edge is chamfered from its own height, as \
             deep as the chamfer's width and the tip offset take the cone.",
        ),
        _ if uses.top.is_none() && uses.bottom.is_none() => {
            Some("It follows the part all the way down, so it has no top or bottom.")
        }
        StrategySpec::Contour { .. } => {
            Some("Without a top or bottom, its levels run from the part's top down to its bottom.")
        }
        StrategySpec::Flat { .. } => Some(
            "Only the flats between the top and bottom are finished; without them, every \
             flat of the part.",
        ),
        StrategySpec::Drill { .. } => Some(
            "Each hole starts at its picked rim. Bottom is how deep a hole goes when its \
             bottom edge is not picked.",
        ),
        StrategySpec::Profile { .. } | StrategySpec::Pocket { .. } => Some(
            "Without a bottom, each area is cut down to the lowest point picked in it, or \
             to the part's bottom when nothing is picked. From the picks, each area's top \
             and bottom are the highest and lowest points picked in it, plus the offset.",
        ),
        StrategySpec::Face { .. } => {
            Some("Without a bottom, the stock is faced down to the part's top.")
        }
        StrategySpec::Engrave { .. } | StrategySpec::Slot { .. } => Some(
            "It cuts its depth below the top: the part's, where a sketch drawn on it lies, \
             or the picks' to cut from where the curves are.",
        ),
        StrategySpec::VCarve { .. } => Some(
            "It carves down from the top, as deep as each area is wide, no deeper than its \
             deepest.",
        ),
        _ => None,
    };
    if let Some(text) = about_it {
        rows.push(about(text));
    }
    rows.push(about(
        "The view shows each height as a plane across the stock, from the last run.",
    ));
    pb::section("Heights")
        .summary(&summary.join(" · "))
        .children(rows)
        .into()
}

/// A check box for each sketch of the document that closes something, on
/// for those the operation takes: what their loops enclose, or, engraving,
/// the loops themselves. A sketch taken that is gone is said.
fn sketch_rows(data: &OperationData) -> Vec<Widget> {
    let all = crate::sketches::sketches();
    let mut rows = vec![pb::overline("Sketches")];
    if all.is_empty() {
        rows.push(pb::caption(
            "None of the document's closes a loop.",
            Tone::Muted,
            true,
        ));
    } else {
        rows.push(about("Their closed loops, text among them."));
    }
    rows.extend(
        all.iter()
            .map(|(id, name)| toggle(format!("sketch:{id}"), name, data.sketches.contains(id))),
    );
    let gone = data
        .sketches
        .iter()
        .filter(|s| !all.iter().any(|(id, _)| id == *s))
        .count();
    if gone > 0 {
        rows.push(note(
            NoteKind::Warning,
            format!("{gone} sketch(es) it takes are gone, or close nothing."),
        ));
    }
    rows
}

/// An operation's spindle speed and feeds.
fn feeds_group(data: &OperationData, id: &str) -> Widget {
    let rpm = if data.rpm > 0.0 {
        format!("{} rev/min", crate::sheet::num(data.rpm))
    } else {
        "The tool's speed".into()
    };
    pb::section("Feeds & speeds")
        .id("feeds")
        .summary(&format!("{rpm} · {} mm/min", crate::sheet::num(data.feed)))
        .children([
            number(
                "rpm",
                "Spindle, rev/min (0: the tool's own speed)",
                data.rpm,
                Dim::Number,
                0,
                id,
            ),
            number(
                "feed",
                "Cutting feed, mm/min",
                data.feed,
                Dim::Number,
                0,
                id,
            ),
            number(
                "plunge",
                "Plunge feed, mm/min",
                data.plunge,
                Dim::Number,
                0,
                id,
            ),
        ])
        .into()
}

/// The run rows of operation `id`, in its setup's: or a word to choose one.
fn op_run(cam: &Cam, id: &str, setup: Option<&(Node, SetupData)>) -> Widget {
    let children = match setup {
        Some((setup_node, _)) => {
            let index = operations_of(&setup_node.id)
                .iter()
                .position(|(n, _)| n.id == id);
            run_widgets(cam, &setup_node.id, index)
        }
        None => vec![note(NoteKind::Info, "Choose the setup it belongs to.")],
    };
    pb::section("Run").children(children).into()
}

pub fn operation_panel(cam: &Cam, node: &Node) -> Vec<Widget> {
    let Some(data) = OperationData::from(&node.data) else {
        return vec![note(NoteKind::Error, "The operation's data does not read.")];
    };
    let id = node.id.as_str();
    let all_setups = setups();
    let setup = data
        .setup
        .as_deref()
        .and_then(|s| all_setups.iter().find(|(n, _)| n.id == s));
    let status = match setup {
        Some((n, s)) => {
            let index = operations_of(&n.id).iter().position(|(o, _)| o.id == id);
            run_status(cam, &n.id, s.enabled && data.operation.enabled, index)
        }
        None => pb::status("No setup", Tone::Warning, "Choose the setup it belongs to."),
    };
    let mut panel = vec![pb::task_header(
        &match setup {
            Some((n, _)) => format!("{} · {}", node.name, n.name),
            None => node.name.clone(),
        },
        Some(status),
    )];
    let mut setup_options: Vec<String> = all_setups.iter().map(|(n, _)| n.name.clone()).collect();
    let setup_index = match setup {
        Some((n, _)) => all_setups
            .iter()
            .position(|(m, _)| m.id == n.id)
            .unwrap_or(0),
        None => {
            setup_options.push("(none)".into());
            setup_options.len() - 1
        }
    };
    let strategy = data.strategy();
    let uses = strategy.uses();
    panel.push(choice("setup", "Setup", setup_options, setup_index));
    panel.push(choice(
        "strategy",
        "Strategy",
        STRATEGIES.iter().map(|(_, l)| l.to_string()).collect(),
        STRATEGIES
            .iter()
            .position(|(n, _)| *n == strategy.name())
            .unwrap_or(0),
    ));
    panel.push(toggle("enabled", "Runs", data.operation.enabled));
    if data.is_custom() {
        panel.extend(gcode_rows(&data));
        if let Some(hint) = &cam.hint {
            panel.push(note(NoteKind::Info, hint.clone()));
        }
        panel.push(op_run(cam, id, setup));
        return panel;
    }

    // The tool.
    let setup_data = setup.map(|(_, d)| d);
    let choices = tool_choices(cam, setup_data);
    let current = data.operation.tool.clone();
    let mut tool_widgets = Vec::new();
    let mut options: Vec<String> = choices
        .iter()
        .map(|(n, lib)| {
            if *lib {
                format!("Library › {n}")
            } else {
                n.clone()
            }
        })
        .collect();
    let selected = match &current {
        Some(t) => choices.iter().position(|(n, lib)| n == t && !lib),
        None => None,
    };
    let selected = selected.unwrap_or_else(|| {
        options.insert(0, "(pick a tool)".into());
        0
    });
    tool_widgets.push(choice("tool", "Tool", options, selected));
    let spec = current
        .as_ref()
        .and_then(|t| setup_data.and_then(|s| s.tools.get(t)));
    let mut tool_summary = "None picked".to_string();
    if let Some(tool) = spec {
        tool_summary = tool_line(tool);
        tool_widgets.push(
            pb::item(current.as_deref().unwrap_or(""))
                .id("cutter")
                .icon(&own_icon("tool"))
                .secondary(&format!("{} Ø{}", kind_label(tool.kind), tool.diameter))
                .value(&format!("T{}", tool.number))
                .hint(&tool_line(tool))
                .into(),
        );
        if !tool.presets.is_empty() {
            let mut presets = vec!["(own values)".to_string()];
            presets.extend(tool.presets.iter().map(|p| p.name.clone()));
            let at = data
                .operation
                .preset
                .as_ref()
                .and_then(|p| tool.presets.iter().position(|q| &q.name == p))
                .map_or(0, |i| i + 1);
            tool_widgets.push(choice("preset", "Preset", presets, at));
            if let Some(p) = &data.operation.preset {
                tool_summary = format!("{tool_summary} · {p}");
            }
        }
    }
    panel.push(
        pb::section("Tool")
            .summary(&tool_summary)
            .children(tool_widgets)
            .into(),
    );

    // A probe goes down at its own feed, set with its grid, and is not
    // turned.
    let probing = matches!(strategy, StrategySpec::Probe { .. });
    if !probing {
        panel.push(feeds_group(&data, id));
    }
    // The passes.
    let mut passes = Vec::new();
    let mut summary = Vec::new();
    if uses.stepover {
        let label = if uses.load {
            "Optimal load"
        } else {
            "Stepover"
        };
        passes.push(number("stepover", label, data.stepover, Dim::Length, 3, id));
        summary.push(format!("{label} {}", crate::sheet::num(data.stepover)));
    }
    match uses.depths {
        Depths::Required => {
            passes.push(number(
                "stepdown",
                "Stepdown",
                data.stepdown,
                Dim::Length,
                3,
                id,
            ));
            summary.push(format!("down {}", crate::sheet::num(data.stepdown)));
        }
        Depths::Optional => {
            passes.push(toggle(
                "multiple_depths",
                "Multiple depths",
                data.multiple_depths,
            ));
            if data.multiple_depths {
                passes.push(number(
                    "stepdown",
                    "Stepdown",
                    data.stepdown,
                    Dim::Length,
                    3,
                    id,
                ));
                summary.push(format!("down {}", crate::sheet::num(data.stepdown)));
            }
        }
        Depths::None => {}
    }
    if uses.stock_to_leave == Leave::Both {
        passes.push(number(
            "radial",
            "Stock to leave, walls",
            data.radial,
            Dim::Length,
            3,
            id,
        ));
    }
    if uses.stock_to_leave != Leave::None {
        passes.push(number(
            "axial",
            "Stock to leave, floors",
            data.axial,
            Dim::Length,
            3,
            id,
        ));
    }
    if matches!(
        strategy,
        StrategySpec::Parallel { .. } | StrategySpec::Face { .. }
    ) {
        passes.push(number("angle", "Pass angle", data.angle, Dim::Angle, 1, id));
    }
    let mut s = strategy;
    if let Some(d) = direction_mut(&mut s) {
        passes.push(choice(
            "direction",
            "Direction",
            DIRECTIONS.iter().map(|(_, l)| l.to_string()).collect(),
            DIRECTIONS.iter().position(|(x, _)| x == d).unwrap_or(0),
        ));
    }
    if let Some(c) = conventional_mut(&mut s) {
        passes.push(toggle("conventional", "Cut conventional", *c));
    }
    if let StrategySpec::Profile { side, .. } = strategy {
        passes.push(choice(
            "side",
            "Side",
            SIDES.iter().map(|(_, l)| l.to_string()).collect(),
            SIDES.iter().position(|(x, _)| *x == side).unwrap_or(0),
        ));
    }
    passes.extend(strategy_rows(&data, spec, id));
    panel.push(
        pb::section("Passes")
            .summary(&summary.join(" · "))
            .children(passes)
            .into(),
    );
    panel.extend(tabs_group(&data, id));
    panel.extend(relief_group(&data, id));
    panel.extend(finishing_group(&data, spec, id));

    // What it machines.
    let mut geometry = Vec::new();
    let mut summary = Vec::new();
    if uses.boundary {
        let b = &data.operation.boundary;
        geometry.push(choice(
            "boundary",
            "Boundary",
            BOUNDARIES.iter().map(|(_, l)| l.to_string()).collect(),
            BOUNDARIES
                .iter()
                .position(|(x, _)| *x == b.kind)
                .unwrap_or(0),
        ));
        if let Some((_, l)) = BOUNDARIES.iter().find(|(x, _)| *x == b.kind) {
            summary.push(l.to_string());
        }
        if b.kind != Boundary::Stock {
            geometry.push(choice(
                "containment",
                "Tool",
                CONTAINMENTS.iter().map(|(_, l)| l.to_string()).collect(),
                CONTAINMENTS
                    .iter()
                    .position(|(x, _)| *x == b.containment)
                    .unwrap_or(0),
            ));
        }
        geometry.push(number(
            "margin",
            "Additional offset",
            data.margin,
            Dim::Length,
            2,
            id,
        ));
    }
    if uses.outline {
        geometry.push(choice(
            "outline",
            "Outline",
            vec![
                "The part's silhouette".into(),
                "The picked faces and edges".into(),
            ],
            usize::from(data.operation.outline == Outline::Selection),
        ));
    }
    let picks = uses.holes
        || uses.traces
        || uses.carves
        || (uses.boundary && data.operation.boundary.kind == Boundary::Selection)
        || (uses.outline && data.operation.outline == Outline::Selection);
    if picks {
        let armed = |slot| {
            cam.armed
                .as_ref()
                .is_some_and(|a| a.feature == id && a.slot == slot)
        };
        let picked = |n: usize| (n > 0).then(|| format!("{n} picked"));
        geometry.push(chip(
            "faces",
            if uses.holes { "Hole walls" } else { "Faces" },
            picked(data.faces.len()),
            armed(Slot::Faces),
        ));
        geometry.push(chip(
            "edges",
            if uses.holes {
                "Hole rims"
            } else if uses.traces {
                "Edges"
            } else {
                "Edge loops"
            },
            picked(data.edges.len()),
            armed(Slot::Edges),
        ));
        summary.push(format!(
            "{} faces, {} edges",
            data.faces.len(),
            data.edges.len()
        ));
        if !uses.holes {
            geometry.extend(sketch_rows(&data));
        }
        geometry.push(button(
            "clear_picks",
            "Clear the picks",
            ButtonStyle::PlainSmall,
            !(data.faces.is_empty() && data.edges.is_empty() && data.sketches.is_empty()),
        ));
        if uses.holes {
            let holes = data.operation.holes;
            geometry.push(toggle(
                "same_diameter",
                "Every hole as wide (same diameter)",
                holes.same_diameter,
            ));
            geometry.push(choice(
                "hole_order",
                "Order",
                HOLE_ORDERS.iter().map(|(_, l)| l.to_string()).collect(),
                HOLE_ORDERS
                    .iter()
                    .position(|(x, _)| *x == holes.order)
                    .unwrap_or(0),
            ));
            if let Some(d) = holes.diameter {
                geometry.push(
                    pb::item("Every hole")
                        .id("every_hole")
                        .icon("hole")
                        .plain_value(&format!(
                            "Ø{} along the tool, picked or not",
                            crate::sheet::num(d)
                        ))
                        .into(),
                );
                geometry.push(button(
                    "pick_holes",
                    "Take only the holes picked",
                    ButtonStyle::Small,
                    true,
                ));
            }
            geometry.push(about(
                "Pick a hole's wall, or its rim. With the same diameter, every hole of the \
                 setup's bodies as wide as one picked, along the tool, is taken too; a \
                 boss's, every boss as wide.",
            ));
        }
    }
    if !geometry.is_empty() {
        panel.push(
            pb::section("Geometry")
                .summary(&summary.join(" · "))
                .children(geometry)
                .into(),
        );
    }
    panel.push(heights_group(&data, id));
    // Its touches are its own, each straight down from the retract height,
    // and a grid is not repeated.
    if !probing {
        panel.push(linking_group(&data, spec, id));
        panel.push(pattern_group(&data, id));
    }

    if let Some(problem) = setup_data.and_then(|s| data.problem(&s.tools)) {
        panel.push(note(NoteKind::Warning, problem));
    }
    if let Some(hint) = &cam.hint {
        panel.push(note(NoteKind::Info, hint.clone()));
    }
    panel.push(op_run(cam, id, setup));
    panel
}

/// A Custom G-code operation's lines, typed as text, what each line may not
/// do, and the placeholders they may name.
fn gcode_rows(data: &OperationData) -> Vec<Widget> {
    let lines = data.gcode_lines();
    let mut children = vec![
        about(
            "Written into the program where it stands in the setup: before the next \
             operation's start, or, last, after the one before ends. An empty line is \
             taken out.",
        ),
        Entry {
            multiline: true,
            mono: true,
            // The panel's width: a line of G-code reads whole.
            width: Some(360.0),
            ..pb::entry_with("gcode", "", &data.operation.gcode, "M0 (check the clamps)")
        }
        .into(),
    ];
    for (i, line) in lines.iter().enumerate() {
        let said = camcraft_post::placeholder::problem(line)
            .map(|why| (NoteKind::Error, why))
            .or_else(|| match camcraft_post::custom::concern(line)? {
                camcraft_post::custom::Concern::Refused(why) => Some((NoteKind::Error, why)),
                camcraft_post::custom::Concern::Unread(why) => Some((NoteKind::Warning, why)),
            });
        if let Some((kind, why)) = said {
            children.push(note(kind, format!("Line {}: {why}", i + 1)));
        }
    }
    children.push(about(
        "It may not move the machine or change a mode: no X, Y, Z (or A, B, C) words, \
         no G code but G4, no M6, M2 or M30. Placeholders are filled from the program \
         it goes into: "
            .to_string()
            + &camcraft_post::placeholder::PLACEHOLDERS
                .iter()
                .map(|(n, what)| format!("@{n}@ {what}"))
                .collect::<Vec<_>>()
                .join("; ")
            + ".",
    ));
    vec![
        pb::section("G-code")
            .count(lines.len())
            .children(children)
            .into(),
    ]
}

pub fn operation_event(cam: &mut Cam, id: &str, event: PanelEvent) {
    let Some((_, mut data)) = operation_node(id) else {
        return;
    };
    let setup = data.setup.clone().and_then(|s| setup_node(&s));
    let tolerance = setup.as_ref().map_or(0.05, |(_, s)| s.tolerance);
    let linking = match &event {
        PanelEvent::Number { id, .. }
        | PanelEvent::Toggle { id, .. }
        | PanelEvent::Choice { id, .. } => id.strip_prefix("linking/"),
        _ => None,
    };
    let passes = match &event {
        PanelEvent::Number { id, .. }
        | PanelEvent::Toggle { id, .. }
        | PanelEvent::Choice { id, .. } => id.strip_prefix("passes/"),
        _ => None,
    };
    if let Some(field) = passes {
        let tool = data
            .operation
            .tool
            .as_ref()
            .and_then(|t| setup.as_ref()?.1.tools.get(t).cloned());
        if passes_event(&mut data, field, &event, tool.as_ref()) {
            cam.write_operation(id, &data);
        }
        return;
    }
    let pattern = match &event {
        PanelEvent::Number { id, .. }
        | PanelEvent::Toggle { id, .. }
        | PanelEvent::Choice { id, .. } => id.strip_prefix("transform/"),
        _ => None,
    };
    if let Some(field) = pattern {
        if pattern_event(&mut data, field, &event) {
            cam.write_operation(id, &data);
        }
        return;
    }
    if let Some(field) = linking {
        let tool = data
            .operation
            .tool
            .as_ref()
            .and_then(|t| setup.as_ref()?.1.tools.get(t).cloned());
        if linking_event(&mut data, field, &event, tool.as_ref()) {
            cam.write_operation(id, &data);
        }
        return;
    }
    match event {
        PanelEvent::Number { id: field, value } => {
            let slot = match field.as_str() {
                "stepover" => &mut data.stepover,
                "stepdown" => &mut data.stepdown,
                "feed" => &mut data.feed,
                "plunge" => &mut data.plunge,
                "rpm" => &mut data.rpm,
                "radial" => &mut data.radial,
                "axial" => &mut data.axial,
                "angle" => &mut data.angle,
                "margin" => &mut data.margin,
                other => match other.strip_prefix("heights/").and_then(HeightKey::named) {
                    Some(h) => h.offset_mut(&mut data),
                    None => return,
                },
            };
            *slot = value;
        }
        PanelEvent::Toggle { id: t, on } if t.starts_with("sketch:") => {
            let sketch = t.trim_start_matches("sketch:").to_string();
            data.sketches.retain(|s| *s != sketch);
            if on {
                data.sketches.push(sketch);
            }
        }
        PanelEvent::Toggle { id: t, on } => match t.as_str() {
            "enabled" => data.operation.enabled = on,
            "multiple_depths" => data.multiple_depths = on,
            "same_diameter" => data.operation.holes.same_diameter = on,
            "conventional" => {
                if let Some(c) = conventional_mut(&mut data.operation.strategy) {
                    *c = on;
                }
            }
            _ => return,
        },
        PanelEvent::Choice { id: field, index } => match field.as_str() {
            "setup" => {
                let all = setups();
                let Some((n, _)) = all.get(index) else {
                    return;
                };
                let old = data.setup.replace(n.id.clone());
                // The old setup runs again without it.
                if let Some(old) = old.filter(|o| *o != n.id) {
                    let _ = host::set_feature_data(id, data.to_value());
                    cam.refresh(&old, false);
                }
            }
            "strategy" => {
                let Some(s) = STRATEGIES
                    .get(index)
                    .and_then(|(n, _)| StrategySpec::named(n))
                else {
                    return;
                };
                if s.name() == data.strategy().name() {
                    return;
                }
                let tool = data
                    .operation
                    .tool
                    .as_ref()
                    .and_then(|t| setup.as_ref()?.1.tools.get(t).cloned());
                data.set_strategy(s, tool.as_ref(), tolerance);
            }
            "tool" => {
                let Some((setup_id, mut setup_data)) = setup.map(|(n, d)| (n.id, d)) else {
                    cam.hint = Some("Choose the setup first: the tool is copied into it.".into());
                    return;
                };
                let choices = tool_choices(cam, Some(&setup_data));
                let placeholder = data
                    .operation
                    .tool
                    .as_ref()
                    .is_none_or(|t| !setup_data.tools.contains_key(t));
                let Some(index) = index.checked_sub(usize::from(placeholder)) else {
                    return;
                };
                let Some((name, library)) = choices.get(index).cloned() else {
                    return;
                };
                let spec = if library {
                    cam.settings.tools.get(&name).cloned()
                } else {
                    setup_data.tools.get(&name).cloned()
                };
                let Some(spec) = spec else {
                    return;
                };
                let name = setup_data.adopt(&name, &spec);
                if library {
                    let _ = host::set_feature_data(&setup_id, setup_data.to_value());
                }
                let preset = spec.presets.first().cloned();
                data.use_tool(&name, &spec, preset.as_ref(), tolerance);
            }
            "preset" => {
                let Some(tool_name) = data.operation.tool.clone() else {
                    return;
                };
                let Some(spec) = setup
                    .as_ref()
                    .and_then(|(_, s)| s.tools.get(&tool_name).cloned())
                else {
                    return;
                };
                let preset = index
                    .checked_sub(1)
                    .and_then(|i| spec.presets.get(i).cloned());
                data.use_tool(&tool_name, &spec, preset.as_ref(), tolerance);
            }
            "direction" => {
                if let Some(d) = direction_mut(&mut data.operation.strategy)
                    && let Some((x, _)) = DIRECTIONS.get(index)
                {
                    *d = *x;
                }
            }
            "side" => {
                if let StrategySpec::Profile { side, .. } = &mut data.operation.strategy
                    && let Some((x, _)) = SIDES.get(index)
                {
                    *side = *x;
                }
            }
            "boundary" => {
                if let Some((x, _)) = BOUNDARIES.get(index) {
                    data.operation.boundary.kind = *x;
                }
            }
            "containment" => {
                if let Some((x, _)) = CONTAINMENTS.get(index) {
                    data.operation.boundary.containment = *x;
                }
            }
            other if other.ends_with("_from") => {
                let Some(h) = other.strip_suffix("_from").and_then(HeightKey::named) else {
                    return;
                };
                let Some(from) = h.choices(&data).get(index).copied() else {
                    return;
                };
                let heights = &mut data.heights;
                match (h, from) {
                    (HeightKey::Clearance, Some(r)) => heights.clearance_from = r,
                    (HeightKey::Retract, Some(r)) => heights.retract_from = r,
                    (HeightKey::Top, r) => heights.top_from = r,
                    (HeightKey::Bottom, r) => heights.bottom_from = r,
                    _ => return,
                }
            }
            "hole_order" => {
                if let Some((x, _)) = HOLE_ORDERS.get(index) {
                    data.operation.holes.order = *x;
                }
            }
            "outline" => {
                data.operation.outline = if index == 1 {
                    Outline::Selection
                } else {
                    Outline::Silhouette
                }
            }
            _ => return,
        },
        PanelEvent::Pick { id: pick }
            if (pick == "faces" && cam.pointer.faces.is_empty() && cam.pointer.face.is_none())
                || (pick == "edges" && cam.pointer.edges.is_empty()) =>
        {
            // Nothing selected: pick in the view, camcraft's way.
            cam.arm(id, Slot::named(&pick));
            return;
        }
        PanelEvent::Pick { id: pick } if pick == "faces" => {
            let body = cam
                .pointer
                .selected_body
                .clone()
                .or(cam.pointer.body.clone());
            let faces = if cam.pointer.faces.is_empty() {
                cam.pointer.face.iter().cloned().collect()
            } else {
                cam.pointer.faces.clone()
            };
            match body {
                Some(body) if !faces.is_empty() => {
                    for f in faces {
                        data.faces.push(Pick {
                            body: body.clone(),
                            at: f.point.map(f64::from),
                            name: f.name,
                            faces: None,
                        });
                    }
                }
                _ => {
                    cam.hint = Some("Select faces in the view first.".into());
                    return;
                }
            }
        }
        PanelEvent::Pick { id: pick } if pick == "edges" => {
            if cam.pointer.edges.is_empty() {
                cam.hint = Some("Select edges in the view first.".into());
                return;
            }
            for e in &cam.pointer.edges {
                data.edges.push(Pick {
                    body: e.body.clone(),
                    at: e.point.map(f64::from),
                    name: None,
                    faces: e.faces,
                });
            }
        }
        PanelEvent::Text { id: t, value } if t == "gcode" => data.set_gcode(&value),
        PanelEvent::Clear { id: pick } if pick == "faces" => data.faces.clear(),
        PanelEvent::Clear { id: pick } if pick == "edges" => data.edges.clear(),
        PanelEvent::Button { id: b } if b == "clear_picks" => {
            data.faces.clear();
            data.edges.clear();
            data.sketches.clear();
        }
        // Its holes by a template's diameter no longer: those picked, which
        // it waits for.
        PanelEvent::Button { id: b } if b == "pick_holes" => {
            data.operation.holes.diameter = None;
            data.needs_pick = Some(crate::templates::what_it_picks(&data).into());
        }
        PanelEvent::Button { id: b } if b == "generate" => {
            if let Some((s, _)) = setup {
                cam.refresh(&s.id, true);
            }
            return;
        }
        PanelEvent::Button { id: b } if b == "stop" => {
            if let Some((s, _)) = setup {
                cam.runs.stop(&s.id);
            }
            return;
        }
        PanelEvent::Button { id: b } if b == "save" || b == "save_anyway" => {
            match cam.program_of(id, b == "save_anyway") {
                Ok(p) => Cam::save(&p),
                Err(e) => cam.hint = Some(e),
            }
            return;
        }
        PanelEvent::Button { id: b } if b == "save_sheet" => {
            match cam.sheet_of(id) {
                Ok((name, html)) => Cam::save_sheet(&name, html),
                Err(e) => cam.hint = Some(e),
            }
            return;
        }
        _ => return,
    }
    cam.write_operation(id, &data);
}

pub fn settings_panel(cam: &Cam) -> Vec<Widget> {
    let tools = &cam.settings.tools;
    // The kinds the library has, for its filter, in the order they come.
    let mut kinds = vec!["All kinds"];
    for t in tools.values() {
        let k = kind_label(t.kind);
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    }
    let items = tools.iter().enumerate().map(|(i, (name, t))| {
        pb::item(name)
            .icon(&own_icon("tool"))
            .secondary(&format!(
                "{} Ø{}{}",
                kind_label(t.kind),
                t.diameter,
                if t.corner > 0.0 {
                    format!(" r{}", t.corner)
                } else {
                    String::new()
                }
            ))
            .value(&format!("T{}", t.number))
            .value_hint(&format!("{} rev/min", t.rpm))
            .kind(kind_label(t.kind))
            .selected(cam.chosen_tool == Some(i))
            .menu("copy", "Copy")
            .menu_with(PanelAction {
                id: "remove".into(),
                label: "Remove".into(),
                separated: true,
                ..Default::default()
            })
            .into()
    });
    let mut page = vec![
        pb::section("Tool library")
            .id("library")
            .count(tools.len())
            .action("import_tools", "open", "Import from Fusion…")
            .action("reset_tools", "refresh", "Back to camcraft's library")
            .child(pb::filter("tools_find").hint("Filter").kinds(kinds))
            .child(
                pb::items("tools")
                    .filter("tools_find")
                    .items(items)
                    .empty("No tools: import some, or go back to camcraft's."),
            )
            .child(about(
                "A setup takes a copy of a tool when an operation first cuts with it, so a \
                 document never depends on the library it was made with. The library starts \
                 as camcraft's twenty common router bits, each with presets for the \
                 materials it suits. Import a library exported from Fusion's Tool Library \
                 as CSV or JSON (right-click a library, Export); JSON brings each tool's \
                 holder too.",
            ))
            .into(),
    ];
    if let Some((name, tool)) = cam.chosen_tool.and_then(|i| tools.iter().nth(i)) {
        page.push(tool_editor(name, tool, cam.chosen_preset));
    }
    match &cam.settings_note {
        Some(Ok(said)) => page.push(note(NoteKind::Info, said.clone())),
        Some(Err(why)) => page.push(note(NoteKind::Error, why.clone())),
        None => {}
    }
    page.push(posts_group(&cam.settings.posts));
    let posts = posts(cam);
    page.push(
        pb::section("A new setup")
            .id("new_setup")
            .summary(&cam.settings.post)
            .children([
                choice(
                    "post",
                    "Machine",
                    posts.clone(),
                    posts
                        .iter()
                        .position(|p| *p == cam.settings.post)
                        .unwrap_or(0),
                ),
                Quantity {
                    dim: Dim::Length,
                    min: Some(0.001),
                    decimals: 3,
                    ..pb::quantity("tolerance", "Tolerance", cam.settings.tolerance)
                }
                .into(),
            ])
            .into(),
    );
    page.push(note(
        NoteKind::Warning,
        "camcraft is pre-alpha and has not been proven on a real machine. Check every \
         program yourself before it cuts: in a simulator you trust, then as an air cut \
         above the stock, with a hand near the e-stop.",
    ));
    page
}

pub fn settings_event(cam: &mut Cam, event: PanelEvent) {
    if tool_event(
        &mut cam.settings.tools,
        &mut cam.chosen_tool,
        &mut cam.chosen_preset,
        &event,
    ) {
        return;
    }
    match &event {
        PanelEvent::ContextMenu { id, index, .. } if id == "posts" => {
            if let Some(name) = cam.settings.posts.keys().nth(*index).cloned() {
                cam.settings.posts.remove(&name);
            }
            return;
        }
        PanelEvent::Button { id } if id == "import_tools" || id == "add_post" => {
            cam.settings_note = None;
            host::request(if id == "import_tools" {
                open_tools()
            } else {
                open_post()
            });
            return;
        }
        _ => {}
    }
    let tools = &mut cam.settings.tools;
    let name_at = |tools: &std::collections::BTreeMap<String, ToolSpec>, row: usize| {
        tools.keys().nth(row).cloned()
    };
    match event {
        PanelEvent::Select { id, index } if id == "tools" => {
            cam.chosen_tool = Some(index);
            cam.chosen_preset = None;
        }
        PanelEvent::ContextMenu { id, index, action } if id == "tools" => {
            let Some(name) = name_at(tools, index) else {
                return;
            };
            match action.as_str() {
                "copy" => {
                    if let Some(tool) = tools.get(&name).cloned() {
                        let mut copy = format!("{name} copy");
                        let mut n = 2;
                        while tools.contains_key(&copy) {
                            copy = format!("{name} copy {n}");
                            n += 1;
                        }
                        tools.insert(copy.clone(), tool);
                        cam.chosen_tool = tools.keys().position(|k| *k == copy);
                        cam.chosen_preset = None;
                    }
                }
                "remove" => {
                    tools.remove(&name);
                    cam.chosen_tool = None;
                    cam.chosen_preset = None;
                }
                _ => {}
            }
        }
        PanelEvent::Button { id } if id == "reset_tools" => {
            cam.settings.tools = crate::Settings::default().tools;
            cam.chosen_tool = None;
        }
        PanelEvent::Choice { id, index } if id == "post" => {
            if let Some(p) = posts(cam).get(index) {
                cam.settings.post = p.clone();
            }
        }
        PanelEvent::Number { id, value } if id == "tolerance" && value > 0.0 => {
            cam.settings.tolerance = value
        }
        _ => {}
    }
}
