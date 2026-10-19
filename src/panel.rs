//! The task panels of a setup and an operation, the preferences page, and what
//! their widgets do.

use cam_protocol::{Level, Run};
use camcraft_core::job::{
    Boundary, Containment, CutDirection, Depths, HeightRef, Leave, Outline, ProfileSide,
    StrategySpec, ToolKind, ToolSpec,
};
use camcraft_core::setup::frame::{Axis, Orientation};
use camcraft_post::profile::Profile;
use printcad_bench_sdk::api::*;
use printcad_bench_sdk::host;

use crate::data::{OperationData, Pick, STRATEGIES, SetupData, StockKind, Zero};
use crate::linking::{linking_event, linking_group};
use crate::passes::{finishing_group, passes_event, strategy_rows, tabs_group};
use crate::runs::State;
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
    Widget::Number {
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
    }
}

pub(crate) fn choice(id: &str, label: &str, options: Vec<String>, selected: usize) -> Widget {
    Widget::Choice {
        id: id.into(),
        label: label.into(),
        options,
        selected,
    }
}

fn button(id: &str, label: &str, style: ButtonStyle, enabled: bool) -> Widget {
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
        .filter(|&x| Orientation { z, x }.check().is_ok())
        .collect()
}

fn posts() -> Vec<String> {
    Profile::builtin_names().map(String::from).collect()
}

fn kind_label(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Flat => "flat",
        ToolKind::Ball => "ball",
        ToolKind::Bullnose => "bull-nose",
        ToolKind::Cone => "V-bit",
        ToolKind::Drill => "drill",
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
                    out.push(Widget::Text {
                        text: op.summary.clone(),
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
                if let Some(p) = &op.program {
                    out.push(Widget::Value {
                        label: "Program".into(),
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
            }
        }
        None => {
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
        }
    }
    out
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
        vec!["Round the bodies".into(), "A box of a fixed size".into()],
        usize::from(setup.stock == StockKind::Fixed),
    )];
    match setup.stock {
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
    }
    panel.push(group("Stock", stock));
    let xs = x_axes(setup.z_axis);
    panel.push(group(
        "Orientation and work zero",
        vec![
            choice(
                "z_axis",
                "Tool axis (Z) along",
                Axis::ALL.into_iter().map(axis_label).collect(),
                Axis::ALL
                    .iter()
                    .position(|&a| a == setup.z_axis)
                    .unwrap_or(4),
            ),
            choice(
                "x_axis",
                "Machine X along",
                xs.iter().copied().map(axis_label).collect(),
                xs.iter().position(|&a| a == setup.x_axis).unwrap_or(0),
            ),
            choice(
                "zero",
                "Work zero",
                Zero::ALL.iter().map(|z| z.label().to_string()).collect(),
                Zero::ALL.iter().position(|&z| z == setup.zero).unwrap_or(0),
            ),
            number("offset_x", "Offset X", setup.offset_x, Dim::Length, 2, id),
            number("offset_y", "Offset Y", setup.offset_y, Dim::Length, 2, id),
            number("offset_z", "Offset Z", setup.offset_z, Dim::Length, 2, id),
        ],
    ));
    let posts = posts();
    panel.push(group(
        "Post",
        vec![
            choice(
                "post",
                "Machine",
                posts.clone(),
                posts.iter().position(|p| *p == setup.post).unwrap_or(0),
            ),
            number("wcs", "Work offset (G54…)", setup.wcs, Dim::Number, 0, id),
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
    let ops = operations_of(id);
    let run = cam.runs.get(id).and_then(|c| c.shown());
    let items = ops
        .iter()
        .enumerate()
        .map(|(i, (n, d))| ListItem {
            label: n.name.clone(),
            detail: Some(match run.and_then(|r| r.operations.get(i)) {
                Some(o) if o.made == "generated" => format!("{:.0} mm cut", o.cut_mm),
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
                    value: tool_line(t),
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
                "offset_x" => &mut setup.offset_x,
                "offset_y" => &mut setup.offset_y,
                "offset_z" => &mut setup.offset_z,
                "wcs" => &mut setup.wcs,
                "tolerance" => &mut setup.tolerance,
                _ => return,
            };
            *slot = value;
            cam.write_setup(id, &setup);
        }
        PanelEvent::Choice { id: field, index } => {
            match field.as_str() {
                "stock" => {
                    setup.stock = if index == 1 {
                        StockKind::Fixed
                    } else {
                        StockKind::Around
                    }
                }
                "z_axis" => {
                    setup.z_axis = Axis::ALL.get(index).copied().unwrap_or(Axis::PosZ);
                    setup.square_axes();
                }
                "x_axis" => {
                    if let Some(&x) = x_axes(setup.z_axis).get(index) {
                        setup.x_axis = x;
                    }
                }
                "zero" => setup.zero = Zero::ALL.get(index).copied().unwrap_or_default(),
                "post" => {
                    if let Some(p) = posts().get(index) {
                        setup.post = p.clone();
                    }
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
        PanelEvent::Toggle { id: t, on } if t == "show_material" => cam.show_material = on,
        PanelEvent::Button { id: b } if b == "generate" => cam.refresh(id, true),
        PanelEvent::Button { id: b } if b == "stop" => cam.runs.stop(id),
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
        | StrategySpec::PocketClearing { conventional, .. } => Some(conventional),
        _ => None,
    }
}

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
        if uses.stay_down {
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

    panel.push(group(
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
    ));

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
    passes.extend(strategy_rows(&data, id));
    panel.push(group("Passes", passes));
    panel.extend(tabs_group(&data, id));
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
        || (uses.boundary && data.operation.boundary.kind == Boundary::Selection)
        || (uses.outline && data.operation.outline == Outline::Selection);
    if picks {
        geometry.push(Widget::Pick {
            id: "faces".into(),
            label: "Add the selected faces".into(),
            value: Some(format!("{} face(s)", data.faces.len())),
            armed: false,
        });
        geometry.push(Widget::Pick {
            id: "edges".into(),
            label: if uses.holes {
                "Add the selected hole rims".into()
            } else {
                "Add the selected edges".into()
            },
            value: Some(format!("{} edge(s)", data.edges.len())),
            armed: false,
        });
        geometry.push(button(
            "clear_picks",
            "Clear the picks",
            ButtonStyle::Small,
            !(data.faces.is_empty() && data.edges.is_empty()),
        ));
    }
    if !geometry.is_empty() {
        panel.push(group("Geometry", geometry));
    }
    panel.push(heights_group(&data, id));
    panel.push(linking_group(&data, spec, id));

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
        PanelEvent::Toggle { id: t, on } => match t.as_str() {
            "enabled" => data.operation.enabled = on,
            "multiple_depths" => data.multiple_depths = on,
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
            "outline" => {
                data.operation.outline = if index == 1 {
                    Outline::Selection
                } else {
                    Outline::Silhouette
                }
            }
            _ => return,
        },
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
                });
            }
        }
        PanelEvent::Button { id: b } if b == "clear_picks" => {
            data.faces.clear();
            data.edges.clear();
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
    let posts = posts();
    vec![
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
    ]
}

pub fn settings_event(cam: &mut Cam, event: PanelEvent) {
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
        PanelEvent::Select { id, index } if id == "tools" => cam.chosen_tool = Some(index),
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
            if let Some(p) = posts().get(index) {
                cam.settings.post = p.clone();
            }
        }
        PanelEvent::Number { id, value } if id == "tolerance" && value > 0.0 => {
            cam.settings.tolerance = value
        }
        _ => {}
    }
}
