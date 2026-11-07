//! The task panels of a setup and an operation, the preferences page, and what
//! their widgets do.

use cam_protocol::{Level, Run};
use camcraft_core::job::{
    Boundary, Containment, CutDirection, Depths, HeightRef, HoleOrder, Leave, Outline, ProfileSide,
    Programs, StrategySpec, ToolKind, ToolSpec,
};
use camcraft_core::setup::frame::{Axis, Orientation};
use camcraft_post::profile::Profile;
use printcad_bench_sdk::api::*;
use printcad_bench_sdk::host;

use crate::data::{
    AxisPick, CylinderAxis, MachineX, OperationData, Pick, STRATEGIES, SetupData, StockKind,
    ToolAxis, Zero,
};
use crate::linking::{linking_event, linking_group};
use crate::passes::{finishing_group, passes_event, relief_group, strategy_rows, tabs_group};
use crate::pattern::{pattern_event, pattern_group};
use crate::picking::Slot;
use crate::runs::State;
use crate::tools::{files_groups, open_post, open_tools, tool_editor, tool_event};
use crate::{Cam, operation_node, operations_of, setup_node, setups};

/// The most findings a panel lists.
const FINDINGS: usize = 12;

pub(crate) fn number(
    id: &str,
    label: &str,
    value: f64,
    dim: Dim,
    decimals: usize,
    feature: &str,
) -> Widget {
    crate::hints::with_hint(Widget::Number {
        id: id.into(),
        label: label.into(),
        value,
        dim,
        bind: Some(Bind {
            feature: feature.to_string(),
            key: format!("/{id}"),
            ..Default::default()
        }),
        min: None,
        max: None,
        decimals,
        error: None,
    })
}

pub(crate) fn choice(id: &str, label: &str, options: Vec<String>, selected: usize) -> Widget {
    Widget::Choice {
        id: id.into(),
        label: label.into(),
        options,
        selected,
    }
}

pub(crate) fn button(id: &str, label: &str, style: ButtonStyle, enabled: bool) -> Widget {
    Widget::Button {
        id: id.into(),
        label: label.into(),
        style,
        enabled,
    }
}

fn note(kind: NoteKind, text: impl Into<String>) -> Widget {
    Widget::Note {
        kind,
        title: None,
        text: text.into(),
    }
}

pub(crate) fn group(title: &str, children: Vec<Widget>) -> Widget {
    Widget::Group {
        title: title.into(),
        open: true,
        children,
    }
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

/// A face or an edge picked for an axis, as a pick row says it.
fn axis_pick_text(what: &str, pick: Option<&AxisPick>) -> String {
    match pick {
        Some(p) => format!("{what} at {:.2}, {:.2}, {:.2}", p.at[0], p.at[1], p.at[2]),
        None => "Nothing picked".into(),
    }
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
    let mut rows = vec![choice("z_axis", "Tool axis (Z) along", options, selected)];
    let toggle = |id: &str, on: bool| Widget::Toggle {
        id: id.into(),
        label: "The other way".into(),
        on,
    };
    match setup.tool_axis {
        ToolAxis::Square => {}
        ToolAxis::Face | ToolAxis::Edge => {
            let (slot, what) = if setup.tool_axis == ToolAxis::Face {
                ("axis_face", "Face")
            } else {
                ("axis_edge", "Edge")
            };
            rows.push(Widget::Pick {
                id: slot.into(),
                label: "Tool axis on the part".into(),
                value: Some(axis_pick_text(what, setup.axis_pick.as_ref())),
                armed: Slot::named(slot).is_some_and(armed),
            });
            rows.push(toggle(
                "axis_reverse",
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
    x_options.push("Along an edge picked on the part".into());
    let x_selected = match setup.machine_x {
        MachineX::Axis => xs.iter().position(|&a| a == setup.x_axis).unwrap_or(0),
        MachineX::Edge => xs.len(),
    };
    rows.push(choice("x_axis", "Machine X along", x_options, x_selected));
    if setup.machine_x == MachineX::Edge {
        rows.push(Widget::Pick {
            id: "x_edge".into(),
            label: "Machine X on the part".into(),
            value: Some(axis_pick_text("Edge", setup.x_pick.as_ref())),
            armed: armed(Slot::XEdge),
        });
        rows.push(toggle(
            "x_reverse",
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

fn kind_label(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Flat => "flat",
        ToolKind::Ball => "ball",
        ToolKind::Bullnose => "bull-nose",
        ToolKind::Cone => "V-bit",
        ToolKind::Drill => "drill",
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
        out.push(Widget::Text {
            text: "What follows is from the last run.".into(),
            mono: false,
        });
    }
    out.extend(verdict(run, only));
    // The program being played: the operation's own, or in a setup's panel
    // the one under way.
    let here = cam.under_way(setup);
    if let Some(op) = only.or(here.map(|(op, _)| op)) {
        let mut kept = cam.listing.borrow_mut();
        out.extend(crate::listing::widgets(run, op, here, &mut kept));
    }
    out
}

fn verdict(run: &Run, only: Option<usize>) -> Vec<Widget> {
    let mut out = Vec::new();
    let mine =
        |f: &&cam_protocol::Finding| only.is_none() || f.operation.is_none() || f.operation == only;
    let failed = run
        .findings
        .iter()
        .filter(|f| f.level == Level::Fail)
        .count();
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
            format!("{failed} check(s) failed: camcraft will not post this setup. Do not run it."),
        )
    });
    let lines: Vec<Widget> = run
        .findings
        .iter()
        .filter(mine)
        .filter(|f| f.level != Level::Pass)
        .take(FINDINGS)
        .map(|f| Widget::Text {
            text: format!(
                "{} {}",
                match f.level {
                    Level::Fail => "✖",
                    Level::Warn => "⚠",
                    _ => "·",
                },
                f.text
            ),
            mono: false,
        })
        .collect();
    if !lines.is_empty() {
        out.push(Widget::Group {
            title: "Checks".into(),
            open: !run.passed,
            children: lines,
        });
    }
    match only {
        Some(i) => {
            if let Some(op) = run.operations.get(i) {
                // Short lines: a long value widens the panel over the view.
                if op.made == "generated" || op.made == "kept" {
                    out.push(Widget::Value {
                        label: "Toolpath".into(),
                        value: format!("{:.0} mm cut, {:.0} mm rapid", op.cut_mm, op.rapid_mm),
                        mono: false,
                    });
                    out.push(Widget::Value {
                        label: "Machining time".into(),
                        value: crate::sheet::duration(op.seconds),
                        mono: false,
                    });
                    out.push(Widget::Text {
                        text: op.summary.clone(),
                        mono: false,
                    });
                } else if op.made == "custom-gcode" {
                    out.push(Widget::Value {
                        label: "G-code".into(),
                        value: op.summary.clone(),
                        mono: false,
                    });
                } else {
                    out.push(Widget::Value {
                        label: "Toolpath".into(),
                        value: op.made.clone(),
                        mono: false,
                    });
                }
                for w in &op.warnings {
                    out.push(note(NoteKind::Warning, w.clone()));
                }
                // Its own, or, posted as one, its setup's.
                if let Some(p) = op.program.as_ref().or(run.program.as_ref()) {
                    out.push(Widget::Value {
                        label: if p.is_setups() {
                            "Setup's program"
                        } else {
                            "Program"
                        }
                        .into(),
                        value: format!("{} lines, {}", p.lines, p.post),
                        mono: true,
                    });
                    out.push(button(
                        "save",
                        "Save G-code…",
                        ButtonStyle::Primary,
                        run.passed,
                    ));
                    if !run.passed {
                        out.push(button(
                            "save_anyway",
                            "Save anyway, unchecked…",
                            ButtonStyle::Destructive,
                            true,
                        ));
                    }
                }
                out.push(sheet_button());
            }
        }
        None if run.program.is_some() => {
            out.push(machining(run));
            out.push(button(
                "save_setup",
                "Save G-code…",
                ButtonStyle::Primary,
                run.passed,
            ));
            out.push(sheet_button());
        }
        None => {
            out.push(machining(run));
            let programs = run
                .operations
                .iter()
                .filter(|o| o.program.is_some())
                .count();
            if programs > 1 {
                out.push(button(
                    "save_all",
                    "Save every program…",
                    ButtonStyle::Primary,
                    run.passed,
                ));
            }
            for (i, op) in run.operations.iter().enumerate() {
                if op.program.is_some() {
                    out.push(button(
                        &format!("save:{i}"),
                        &format!("Save {}…", op.name),
                        ButtonStyle::Secondary,
                        run.passed,
                    ));
                }
            }
            out.push(sheet_button());
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
    let mut rows: Vec<Widget> = run
        .operations
        .iter()
        .filter(|o| o.seconds > 0.0)
        .map(|o| Widget::Value {
            label: o.name.clone(),
            value: crate::sheet::duration(o.seconds),
            mono: false,
        })
        .collect();
    rows.push(Widget::Value {
        label: "Tool changes".into(),
        value: match m.tool_change {
            Some(each) => format!("{} × {}", m.tool_changes, crate::sheet::duration(each)),
            None => format!("{}, not timed", m.tool_changes),
        },
        mono: false,
    });
    rows.push(Widget::Value {
        label: "Total".into(),
        value: crate::sheet::duration(m.total()),
        mono: false,
    });
    let [x, y, z] = m.rapid.map(crate::sheet::num);
    rows.push(Widget::Text {
        text: format!(
            "Rapids at X {x}, Y {y}, Z {z} mm/min{}. Nothing speeds up or slows down \
             here, so a machine takes longer.",
            if m.rapid_given {
                ", as the post says"
            } else {
                ", assumed"
            }
        ),
        mono: false,
    });
    Widget::Group {
        title: format!("Machining time: {}", crate::sheet::duration(m.total())),
        open: false,
        children: rows,
    }
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

pub fn setup_panel(cam: &Cam, node: &Node) -> Vec<Widget> {
    let Some(setup) = SetupData::from(&node.data) else {
        return vec![note(NoteKind::Error, "The setup's data does not read.")];
    };
    let id = node.id.as_str();
    let bodies = host::bodies();
    let body_items = setup
        .bodies
        .iter()
        .map(|b| ListItem {
            label: bodies
                .iter()
                .find(|x| &x.id == b)
                .map_or_else(|| "A body no longer there".into(), |x| x.name.clone()),
            ..Default::default()
        })
        .collect();
    let mut panel = vec![Widget::Heading {
        text: node.name.clone(),
    }];
    panel.push(Widget::Toggle {
        id: "enabled".into(),
        label: "Runs".into(),
        on: setup.enabled,
    });
    if !setup.enabled {
        panel.push(note(
            NoteKind::Info,
            "Turned off: none of its operations is run, simulated, checked or posted.",
        ));
    }
    panel.push(group(
        "Bodies",
        vec![
            Widget::List {
                id: "bodies".into(),
                items: body_items,
                selected: cam.chosen_body,
            },
            Widget::Pick {
                id: "body".into(),
                label: "Add the selected body".into(),
                value: None,
                armed: false,
            },
            button(
                "remove_body",
                "Remove the chosen body",
                ButtonStyle::Small,
                cam.chosen_body.is_some(),
            ),
        ],
    ));
    let mut stock = vec![choice(
        "stock",
        "Stock",
        vec![
            "A box round the bodies".into(),
            "A box of a fixed size".into(),
            "Cylinder".into(),
            "From an earlier setup".into(),
        ],
        match setup.stock {
            StockKind::Around => 0,
            StockKind::Fixed => 1,
            StockKind::Cylinder => 2,
            StockKind::Earlier => 3,
        },
    )];
    match setup.stock {
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
            stock.push(Widget::Text {
                text: "What that setup leaves after its last operation, the same material \
                       turned as this setup sees it: the checks, the simulation, playback \
                       and the times start from it, and it runs again when that one changes. \
                       Seen down this setup's tool, a pocket hidden under material is taken \
                       as material."
                    .into(),
                mono: false,
            });
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
            stock.push(Widget::Text {
                text: "Centred on the bodies, its top flush with theirs.".into(),
                mono: false,
            });
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
                stock.push(Widget::Text {
                    text: if setup.cylinder_axis == CylinderAxis::Tool {
                        "Centred on the bodies, its top flush with theirs."
                    } else {
                        "Centred on the bodies."
                    }
                    .into(),
                    mono: false,
                });
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
        }
    }
    stock.push(Widget::TextField {
        id: "material".into(),
        label: "Material".into(),
        value: setup.material.clone(),
    });
    panel.push(group("Stock", stock));
    let mut orientation = orientation_rows(cam, id, &setup);
    orientation.extend([
        choice(
            "zero",
            "Work zero",
            Zero::ALL.iter().map(|z| z.label().to_string()).collect(),
            Zero::ALL.iter().position(|&z| z == setup.zero).unwrap_or(0),
        ),
        Widget::Pick {
            id: "zero_pick".into(),
            label: "On the part".into(),
            value: Some(match &setup.zero_point {
                Some(z) if setup.zero == Zero::Picked => format!(
                    "{} at {:.2}, {:.2}, {:.2}",
                    if z.snap == camcraft_core::job::Snap::Vertex {
                        "Corner"
                    } else {
                        "Centre"
                    },
                    z.at[0],
                    z.at[1],
                    z.at[2]
                ),
                _ => "Nothing picked".into(),
            }),
            armed: cam
                .armed
                .as_ref()
                .is_some_and(|a| a.feature == id && a.slot == Slot::Zero),
        },
        number("offset_x", "Offset X", setup.offset_x, Dim::Length, 2, id),
        number("offset_y", "Offset Y", setup.offset_y, Dim::Length, 2, id),
        number("offset_z", "Offset Z", setup.offset_z, Dim::Length, 2, id),
    ]);
    panel.push(group("Orientation and work zero", orientation));
    let mut posts = posts(cam);
    // A post of someone else's the setup carries, though this library has
    // not got it.
    if !posts.contains(&setup.post) {
        posts.push(setup.post.clone());
    }
    panel.push(group(
        "Post",
        vec![
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
        ],
    ));
    panel.push(group("Work offset", work_offset_rows(id, &setup)));
    panel.push(group("Probed surface", probe_rows(id, &setup)));
    let ops = operations_of(id);
    let run = cam.runs.get(id).and_then(|c| c.shown());
    let items = ops
        .iter()
        .enumerate()
        .map(|(i, (n, d))| ListItem {
            label: n.name.clone(),
            detail: Some(match run.and_then(|r| r.operations.get(i)) {
                Some(o) if o.made == "generated" => format!(
                    "{:.0} mm cut, {}",
                    o.cut_mm,
                    crate::sheet::duration(o.seconds)
                ),
                Some(o) if o.made == "custom-gcode" => o.summary.clone(),
                Some(o) => o.made.clone(),
                None => crate::data::strategy_word(&d.strategy()).into(),
            }),
            icon: Some("operation".into()),
        })
        .collect();
    panel.push(group(
        "Operations",
        vec![
            Widget::List {
                id: "operations".into(),
                items,
                selected: None,
            },
            choice(
                "new_strategy",
                "Strategy",
                STRATEGIES.iter().map(|(_, l)| l.to_string()).collect(),
                cam.new_strategy,
            ),
            button(
                "add_operation",
                "Add operation",
                ButtonStyle::Secondary,
                true,
            ),
        ],
    ));
    if !setup.tools.is_empty() {
        panel.push(Widget::Group {
            title: "Its tools".into(),
            open: false,
            children: setup
                .tools
                .iter()
                .map(|(name, t)| Widget::Value {
                    label: name.clone(),
                    value: match run.and_then(|r| r.tools.iter().find(|u| &u.name == name)) {
                        Some(used) => format!(
                            "{} · {} · to Z{}",
                            tool_line(t),
                            used.operations
                                .iter()
                                .filter_map(|&k| ops.get(k).map(|(n, _)| n.name.as_str()))
                                .collect::<Vec<_>>()
                                .join(", "),
                            used.lowest.map_or_else(|| "?".into(), crate::sheet::num)
                        ),
                        None => tool_line(t),
                    },
                    mono: false,
                })
                .collect(),
        });
    }
    if let Some(hint) = &cam.hint {
        panel.push(note(NoteKind::Info, hint.clone()));
    }
    panel.push(Widget::Separator);
    panel.push(Widget::Toggle {
        id: "show_material".into(),
        label: "Show the material left".into(),
        on: cam.show_material,
    });
    panel.extend(run_widgets(cam, id, None));
    panel
}

/// Whether a setup's cut depths are corrected to probe results, and from
/// which.
fn probe_rows(id: &str, setup: &SetupData) -> Vec<Widget> {
    let mut rows = vec![Widget::Toggle {
        id: "probe_correct".into(),
        label: "Correct Z from probe results".into(),
        on: setup.probe_correct,
    }];
    if !setup.probe_correct {
        return rows;
    }
    let units = match setup.profile().map(|p| p.units) {
        Some(camcraft_post::profile::ProgramUnits::Inch) => "in",
        _ => "mm",
    };
    let results = if !setup.probe_points.is_empty() {
        match camcraft_core::probe::ProbeMap::from_points(&setup.probe_points, 1.0) {
            Ok(map) => {
                let (columns, rows) = map.size();
                let (low, high) = map.heights();
                format!(
                    "{}: {columns} x {rows} points, Z {low:.3} to {high:.3} {units}",
                    setup.probe_file
                )
            }
            Err(e) => format!("{}: {e}", setup.probe_file),
        }
    } else if !setup.probe_file.is_empty() {
        format!("{}, read each run", setup.probe_file)
    } else {
        "none loaded".into()
    };
    rows.push(Widget::Value {
        label: "Results".into(),
        value: results,
        mono: false,
    });
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
    rows.push(Widget::Text {
        text: format!(
            "Every move of every operation but a probe's goes up or down by the probed \
             surface's height there, from the work zero, bilinearly between the points; \
             a cut is split into pieces no longer than the segment length, and an arc \
             into straight pieces where the surface is not level under it. Outside the \
             probed area the surface is taken as at its nearest edge, and the operation \
             says how far it went. The stock's top and the part follow it in the \
             simulation and the checks. Results are a LinuxCNC probe log (x y z ...) or \
             a CSV of x, y, z, in the post's units ({units}) and the setup's work \
             coordinates."
        ),
        mono: false,
    });
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
        Widget::Toggle {
            id: "wcs_known".into(),
            label: format!("Say where {word} is on the machine"),
            on: setup.wcs_known,
        },
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
        rows.push(Widget::Text {
            text: format!(
                "The work zero in machine coordinates, as {word} holds it, along the \
                 setup's axes (for a tilted setup, with the part turned). {z}."
            ),
            mono: false,
        });
    }
    rows.push(Widget::Text {
        text: match (travel, machine, setup.wcs_known) {
            (None, ..) => "Its post gives no travel: nothing is checked against it.".into(),
            (Some(_), false, _) => {
                "Its post's travel is from the work zero, not in machine coordinates.".into()
            }
            (Some(_), true, true) => {
                "Its programs are checked against its post's travel there.".into()
            }
            (Some(_), true, false) => format!(
                "Its post's travel is in machine coordinates: say where {word} is to check \
                 it."
            ),
        },
        mono: false,
    });
    rows
}

pub fn setup_event(cam: &mut Cam, id: &str, event: PanelEvent) {
    let Some((_, mut setup)) = setup_node(id) else {
        return;
    };
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
                _ => return,
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
                        _ => StockKind::Around,
                    }
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
        PanelEvent::Select { id: list, index } if list == "bodies" => cam.chosen_body = Some(index),
        PanelEvent::Select { id: list, index } if list == "operations" => {
            if let Some((op, _)) = operations_of(id).get(index) {
                let op = op.id.clone();
                cam.open(&op, false);
            }
        }
        PanelEvent::Pick { id: pick } if pick == "zero_pick" => cam.arm(id, Some(Slot::Zero)),
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
        PanelEvent::Pick { id: pick } if pick == "body" => {
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
        PanelEvent::Button { id: b } if b == "remove_body" => {
            if let Some(i) = cam.chosen_body.take()
                && i < setup.bodies.len()
            {
                setup.bodies.remove(i);
                cam.write_setup(id, &setup);
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

/// The Heights group: for each height the strategy has, what it is measured
/// from and, when it is given, its offset.
fn heights_group(data: &OperationData, feature: &str) -> Widget {
    let mut rows = Vec::new();
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
        }
    }
    let uses = data.strategy().uses();
    let about = match data.strategy() {
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
    if let Some(about) = about {
        rows.push(Widget::Text {
            text: about.into(),
            mono: false,
        });
    }
    rows.push(Widget::Text {
        text: "The view shows each height as a plane across the stock, from the last run.".into(),
        mono: false,
    });
    group("Heights", rows)
}

/// A toggle for each sketch of the document that closes something, on for
/// those the operation takes: what their loops enclose, or, engraving, the
/// loops themselves. A sketch taken that is gone is said.
fn sketch_rows(data: &OperationData) -> Vec<Widget> {
    let all = crate::sketches::sketches();
    let mut rows = vec![Widget::Text {
        text: if all.is_empty() {
            "Sketches: none of the document's closes a loop.".into()
        } else {
            "Sketches (their closed loops, text among them):".into()
        },
        mono: false,
    }];
    rows.extend(all.iter().map(|(id, name)| Widget::Toggle {
        id: format!("sketch:{id}"),
        label: name.clone(),
        on: data.sketches.contains(id),
    }));
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
    group(
        "Feeds and speeds",
        vec![
            number(
                "rpm",
                "Spindle, rev/min (0: the tool's)",
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
        ],
    )
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
    let mut panel = vec![Widget::Heading {
        text: node.name.clone(),
    }];
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
    panel.push(Widget::Toggle {
        id: "enabled".into(),
        label: "Runs".into(),
        on: data.operation.enabled,
    });
    if data.is_custom() {
        panel.extend(gcode_rows(&data));
        panel.push(Widget::Separator);
        if let Some((setup_node, _)) = setup {
            let index = operations_of(&setup_node.id)
                .iter()
                .position(|(n, _)| n.id == id);
            panel.extend(run_widgets(cam, &setup_node.id, index));
        } else {
            panel.push(note(NoteKind::Info, "Choose the setup it belongs to."));
        }
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
    if let Some(tool) = spec {
        tool_widgets.push(Widget::Value {
            label: "Cutter".into(),
            value: tool_line(tool),
            mono: false,
        });
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
        }
    }
    panel.push(group("Tool", tool_widgets));

    // A probe goes down at its own feed, set with its grid, and is not
    // turned.
    let probing = matches!(strategy, StrategySpec::Probe { .. });
    if !probing {
        panel.push(feeds_group(&data, id));
    }
    // The passes.
    let mut passes = Vec::new();
    if uses.stepover {
        let label = if uses.load {
            "Optimal load"
        } else {
            "Stepover"
        };
        passes.push(number("stepover", label, data.stepover, Dim::Length, 3, id));
    }
    match uses.depths {
        Depths::Required => passes.push(number(
            "stepdown",
            "Stepdown",
            data.stepdown,
            Dim::Length,
            3,
            id,
        )),
        Depths::Optional => {
            passes.push(Widget::Toggle {
                id: "multiple_depths".into(),
                label: "Multiple depths".into(),
                on: data.multiple_depths,
            });
            if data.multiple_depths {
                passes.push(number(
                    "stepdown",
                    "Stepdown",
                    data.stepdown,
                    Dim::Length,
                    3,
                    id,
                ));
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
        passes.push(Widget::Toggle {
            id: "conventional".into(),
            label: "Cut conventional".into(),
            on: *c,
        });
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
    panel.push(group("Passes", passes));
    panel.extend(tabs_group(&data, id));
    panel.extend(relief_group(&data, id));
    panel.extend(finishing_group(&data, spec, id));

    // What it machines.
    let mut geometry = Vec::new();
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
        geometry.push(Widget::Pick {
            id: "faces".into(),
            label: if uses.holes {
                "Hole walls".into()
            } else {
                "Faces".into()
            },
            value: Some(format!("{} picked", data.faces.len())),
            armed: armed(Slot::Faces),
        });
        geometry.push(Widget::Pick {
            id: "edges".into(),
            label: if uses.holes {
                "Hole rims".into()
            } else if uses.traces {
                "Edges".into()
            } else {
                "Edge loops".into()
            },
            value: Some(format!("{} picked", data.edges.len())),
            armed: armed(Slot::Edges),
        });
        if !uses.holes {
            geometry.extend(sketch_rows(&data));
        }
        geometry.push(button(
            "clear_picks",
            "Clear the picks",
            ButtonStyle::Small,
            !(data.faces.is_empty() && data.edges.is_empty() && data.sketches.is_empty()),
        ));
        if uses.holes {
            let holes = data.operation.holes;
            geometry.push(Widget::Toggle {
                id: "same_diameter".into(),
                label: "Every hole as wide (same diameter)".into(),
                on: holes.same_diameter,
            });
            geometry.push(choice(
                "hole_order",
                "Order",
                HOLE_ORDERS.iter().map(|(_, l)| l.to_string()).collect(),
                HOLE_ORDERS
                    .iter()
                    .position(|(x, _)| *x == holes.order)
                    .unwrap_or(0),
            ));
            geometry.push(note(
                NoteKind::Info,
                "Pick a hole's wall, or its rim. With the same diameter, every hole of the \
                 setup's bodies as wide as one picked, along the tool, is taken too; a \
                 boss's, every boss as wide.",
            ));
        }
    }
    if !geometry.is_empty() {
        panel.push(group("Geometry", geometry));
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
    panel.push(Widget::Separator);
    if let Some((setup_node, _)) = setup {
        let index = operations_of(&setup_node.id)
            .iter()
            .position(|(n, _)| n.id == id);
        panel.extend(run_widgets(cam, &setup_node.id, index));
    } else {
        panel.push(note(NoteKind::Info, "Choose the setup it belongs to."));
    }
    panel
}

/// A Custom G-code operation's lines, a row each and an empty one to add
/// one, what each line may not do, and the placeholders they may name.
fn gcode_rows(data: &OperationData) -> Vec<Widget> {
    let lines = data.gcode_lines();
    let row = |i: usize, text: &str| {
        vec![
            Cell::Text {
                text: format!("{}", i + 1),
                mono: true,
                strong: false,
            },
            Cell::Edit { text: text.into() },
        ]
    };
    let mut rows: Vec<Vec<Cell>> = lines.iter().enumerate().map(|(i, l)| row(i, l)).collect();
    rows.push(row(lines.len(), ""));
    let mut children = vec![
        Widget::Text {
            text: "Written into the program where it stands in the setup: before the next \
                   operation's start, or, last, after the one before ends. Type a line into \
                   the empty row to add one; empty a line to take it out."
                .into(),
            mono: false,
        },
        Widget::Sheet {
            id: "gcode".into(),
            columns: vec!["".into(), "G-code".into()],
            rows,
        },
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
    children.push(Widget::Text {
        text: "It may not move the machine or change a mode: no X, Y, Z (or A, B, C) words, \
               no G code but G4, no M6, M2 or M30. Placeholders are filled from the program \
               it goes into: "
            .to_string()
            + &camcraft_post::placeholder::PLACEHOLDERS
                .iter()
                .map(|(n, what)| format!("@{n}@ {what}"))
                .collect::<Vec<_>>()
                .join("; ")
            + ".",
        mono: false,
    });
    vec![Widget::Group {
        title: "G-code".into(),
        open: true,
        children,
    }]
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
        PanelEvent::CellText {
            id: table,
            row,
            column: 1,
            value,
        } if table == "gcode" => data.set_gcode_line(row, &value),
        PanelEvent::Button { id: b } if b == "clear_picks" => {
            data.faces.clear();
            data.edges.clear();
            data.sketches.clear();
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
    let rows = cam
        .settings
        .tools
        .iter()
        .map(|(name, t)| {
            vec![
                name.clone(),
                kind_label(t.kind).to_string(),
                format!("{}", t.diameter),
                format!("{}", t.corner),
                format!("{}", t.number),
                format!("{}", t.rpm),
            ]
        })
        .collect();
    let posts = posts(cam);
    let mut editor = Vec::new();
    if let Some((name, tool)) = cam
        .chosen_tool
        .and_then(|i| cam.settings.tools.iter().nth(i))
    {
        editor.push(tool_editor(name, tool, cam.chosen_preset));
    }
    editor.extend(files_groups(&cam.settings.posts, cam.chosen_post));
    match &cam.settings_note {
        Some(Ok(said)) => editor.push(note(NoteKind::Info, said.clone())),
        Some(Err(why)) => editor.push(note(NoteKind::Error, why.clone())),
        None => {}
    }
    let mut page = vec![
        Widget::Heading {
            text: "Tool library".into(),
        },
        Widget::Text {
            text: "A setup takes a copy of a tool when an operation first cuts with it, so a \
                   document never depends on the library it was made with. The library starts \
                   as camcraft's twenty common router bits, each with presets for the \
                   materials it suits."
                .into(),
            mono: false,
        },
        Widget::Table {
            id: "tools".into(),
            columns: ["Name", "Kind", "Ø mm", "Corner", "T", "rev/min"]
                .map(String::from)
                .to_vec(),
            rows,
            selected: cam.chosen_tool,
            editable: vec![false, false, true, true, true, true],
        },
        button(
            "add_tool",
            "Copy the chosen tool",
            ButtonStyle::Secondary,
            cam.chosen_tool.is_some(),
        ),
        button(
            "remove_tool",
            "Remove the chosen tool",
            ButtonStyle::Destructive,
            cam.chosen_tool.is_some(),
        ),
        button(
            "reset_tools",
            "Back to camcraft's library",
            ButtonStyle::Small,
            true,
        ),
    ];
    page.extend(editor);
    page.extend([
        Widget::Separator,
        Widget::Heading {
            text: "A new setup".into(),
        },
        choice(
            "post",
            "Machine",
            posts.clone(),
            posts
                .iter()
                .position(|p| *p == cam.settings.post)
                .unwrap_or(0),
        ),
        Widget::Number {
            id: "tolerance".into(),
            label: "Tolerance".into(),
            value: cam.settings.tolerance,
            dim: Dim::Length,
            bind: None,
            min: Some(0.001),
            max: None,
            decimals: 3,
            error: None,
        },
        note(
            NoteKind::Warning,
            "camcraft is pre-alpha and has not been proven on a real machine. Check every \
             program yourself before it cuts: in a simulator you trust, then as an air cut \
             above the stock, with a hand near the e-stop.",
        ),
    ]);
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
        PanelEvent::Select { id, index } if id == "posts" => {
            cam.chosen_post = Some(*index);
            return;
        }
        PanelEvent::Button { id } if id == "remove_post" => {
            if let Some(name) = cam
                .chosen_post
                .take()
                .and_then(|i| cam.settings.posts.keys().nth(i).cloned())
            {
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
        PanelEvent::Cell {
            id,
            row,
            column,
            value,
        } if id == "tools" => {
            let Some(name) = name_at(tools, row) else {
                return;
            };
            let Some(tool) = tools.get_mut(&name) else {
                return;
            };
            let number = value.trim().replace(',', ".").parse::<f64>().ok();
            match (column, number) {
                (2, Some(v)) if v > 0.0 => tool.diameter = v,
                (3, Some(v)) if v >= 0.0 && v <= tool.diameter / 2.0 => tool.corner = v,
                (4, Some(v)) if v >= 1.0 => tool.number = v.round() as u32,
                (5, Some(v)) if v >= 0.0 => tool.rpm = v.round() as u32,
                _ => {}
            }
        }
        PanelEvent::Select { id, index } if id == "tools" => {
            cam.chosen_tool = Some(index);
            cam.chosen_preset = None;
        }
        PanelEvent::Button { id } if id == "add_tool" => {
            if let Some(name) = cam.chosen_tool.and_then(|i| name_at(tools, i))
                && let Some(tool) = tools.get(&name).cloned()
            {
                let mut copy = format!("{name} copy");
                let mut n = 2;
                while tools.contains_key(&copy) {
                    copy = format!("{name} copy {n}");
                    n += 1;
                }
                tools.insert(copy.clone(), tool);
                cam.chosen_tool = tools.keys().position(|k| *k == copy);
            }
        }
        PanelEvent::Button { id } if id == "remove_tool" => {
            if let Some(name) = cam.chosen_tool.take().and_then(|i| name_at(tools, i)) {
                tools.remove(&name);
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
