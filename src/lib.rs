//! CAM: a workbench package for 3-axis CNC milling, after camcraft.
//!
//! A setup and its operations are features of the package's own kinds that
//! build no solid. A setup names the bodies it machines, its stock, how the
//! part sits and where its work zero is, and its post; each operation names
//! its setup, its strategy (adaptive or pocket clearing, the finishing
//! strategies, 2D profiles and pockets, facing, drilling, engraving and
//! V-carving) and its tool.
//!
//! None of the machining happens here. A setup runs as a job that hands it, as
//! a camcraft job with each body's B-rep, to the package's native helper
//! (`helper/`), which runs camcraft's engine on every core: the toolpaths, the
//! simulation against the material, the checks, and a G-code program for the
//! setup, or one for each operation. The package draws what comes back, shows the checks, and saves
//! the programs through the save dialog, refusing one camcraft refuses.

// Every literal of the contract's structs ends in `..Default::default()`,
// even one naming every field, so a field a later 0.1.x adds leaves the
// package building.
#![allow(clippy::needless_update)]

mod data;
mod draw;
mod hints;
mod linking;
mod listing;
mod panel;
mod passes;
mod pattern;
mod picking;
mod playback;
mod runs;
mod session;
mod sheet;
mod sketches;
mod templates;
mod tools;

use std::collections::BTreeMap;
use std::rc::Rc;

use cam_protocol::{HELPER, IDENTITY, Level, Request as HelperRequest, Shape, VERSION};
use camcraft_core::job::{StrategySpec, ToolSpec};
use camcraft_core::setup::frame::Axis;
use printcad_bench_sdk::api::*;
use printcad_bench_sdk::{Bench, Value, bench, host, json, serde_json};

use data::{
    CylinderAxis, OperationData, STRATEGIES, SetupData, StockKind, models_of, strategy_word,
};
use runs::{Computed, JOB, Runs, State};

/// The package's id: its feature kinds, tools, commands and menu entries all
/// start with it.
const PACKAGE: &str = "io.github.gilbertorconde.cam";
const SETUP_KIND: &str = "io.github.gilbertorconde.cam.setup";
const OPERATION_KIND: &str = "io.github.gilbertorconde.cam.operation";

const NEW_SETUP: &str = "new_setup";
const NEW_OPERATION: &str = "new_operation";
const GENERATE: &str = "generate";
const EDIT: &str = "edit";
const ADD_OPERATION: &str = "add_operation";
const TURN_OFF: &str = "turn_off";
const TURN_ON: &str = "turn_on";
const FROM_TEMPLATE: &str = "from_template";
const SAVE_AS_TEMPLATE: &str = "save_as_template";

const SETUP: &str = "setup";
const OPERATION: &str = "operation";
const TOOLS: &str = "tools";
const IMPORT_TOOLS: &str = "import_tools";
const ADD_POST: &str = "add_post";
const RUN: &str = "run";
const STATUS: &str = "status";
const GCODE: &str = "gcode";
const ENABLE: &str = "enable";
const SETUP_SHEET: &str = "setup_sheet";
const SAVE_TEMPLATE: &str = "save_template";
const APPLY_TEMPLATE: &str = "apply_template";
const TEMPLATES: &str = "templates";
const DELETE_TEMPLATE: &str = "delete_template";

/// The package's id for `suffix`.
fn id(suffix: &str) -> String {
    format!("{PACKAGE}.{suffix}")
}

/// The suffix of one of the package's ids.
fn suffix(id: &str) -> Option<&str> {
    id.strip_prefix(PACKAGE)?.strip_prefix('.')
}

/// The most points the helper sends back to draw one operation.
const DRAWN_POINTS: usize = 20_000;

/// The package's preferences: the tool library, and what a new setup starts
/// with.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Settings {
    /// The library, by name; a setup takes copies.
    tools: BTreeMap<String, ToolSpec>,
    /// A new setup's post.
    post: String,
    /// A new setup's tolerance, millimetres.
    tolerance: f64,
    /// The user's own machine profiles, as camcraft's TOML, by name.
    #[serde(default)]
    posts: BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        // camcraft's System library: twenty common router bits, each with
        // presets for the materials it suits.
        let mut root = camcraft_session::library::Folder::default();
        camcraft_session::library::system::add_system(&mut root);
        let tools = root
            .walk()
            .into_iter()
            .map(|(_, name, tool)| (name.to_string(), tool.clone()))
            .collect();
        Self {
            tools,
            post: "grbl".into(),
            tolerance: 0.05,
            posts: BTreeMap::new(),
        }
    }
}

/// What a task is open on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Setup,
    Operation,
}

#[derive(Debug, Clone)]
struct Editing {
    id: String,
    kind: Kind,
    /// Its data when the task opened, put back by Cancel.
    opened: Value,
    /// The tool just made it: Cancel removes it.
    fresh: bool,
}

#[derive(Default)]
struct Cam {
    editing: Option<Editing>,
    runs: Runs,
    /// What was selected when the frame was last drawn.
    pointer: Pointer,
    settings: Settings,
    /// The settings page's selected tool.
    chosen_tool: Option<usize>,
    /// The setup panel's selected body.
    chosen_body: Option<usize>,
    /// The setup panel's strategy for Add operation, by its place in
    /// [`STRATEGIES`].
    new_strategy: usize,
    /// Whether the open setup draws its finished material.
    show_material: bool,
    /// The open setup's run, being played back.
    playing: Option<playback::Playing>,
    /// How the transport plays.
    transport: playback::Transport,
    /// Whether Shift is held, for the transport's keys.
    shift: bool,
    /// The played-back setup's run, kept in a helper at full resolution.
    session: Option<session::Session>,
    /// A line of the program listing clicked before the session knew when
    /// each move ends: its operation and moves, and the playhead it was
    /// sent to meanwhile, between the run's moments. Once the session
    /// knows, the playhead goes to the move itself, unless it has moved.
    aim: Option<(usize, usize, f64)>,
    /// Where the listed program's lines start, worked out once for it.
    listing: std::cell::RefCell<listing::Lines>,
    /// The material drawn, kept by the host.
    material: std::cell::RefCell<draw::KeptMaterial>,
    /// A slot picking in the view.
    armed: Option<picking::Armed>,
    /// The settings page's chosen preset of the chosen tool, and chosen post.
    chosen_preset: Option<usize>,
    chosen_post: Option<usize>,
    /// What the settings page last did with a file.
    settings_note: Option<Result<String, String>>,
    /// A word for the task panel about the last thing tried.
    hint: Option<String>,
    /// Setups to check for changes once the [`LATER`] job under way ends:
    /// every setup when `None`.
    later: Option<(u64, Option<Vec<String>>)>,
    /// The templates, built in and the user's, read once
    /// ([`Cam::templates`]), with each file that does not read.
    templates: std::cell::RefCell<Option<templates::Shelf>>,
    /// The template file jobs under way, and what each does.
    template_jobs: std::cell::RefCell<Vec<(u64, templates::Work)>>,
    /// What the setup panel's Templates group has chosen.
    template_ui: templates::Ui,
}

/// A job that does nothing, so its end comes as an event printCAD gives
/// a second rather than a frame's 25 ms: checking whether the setups
/// changed reads every setup's operations, sketches and solids, and the
/// events that ask for it (the workbench shown, values moved) are timed
/// as a frame.
const LATER: &str = "later";

thread_local! {
    /// The document's features, read once for the call under way while it
    /// is [`reading`]: `None` outside one, `Some(None)` until first read.
    static READ: std::cell::RefCell<Option<Option<Rc<Vec<Node>>>>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `f`, a call that does not change the document, reading its features
/// at most once. Drawing, a menu and working out whether the setups changed
/// look up each setup and its operations several times; each look has the
/// host write out every feature and the package read them all back, which on
/// a big document is most of the call's time.
fn reading<R>(f: impl FnOnce() -> R) -> R {
    let outer = READ.with(|r| r.borrow().is_some());
    if outer {
        return f();
    }
    READ.with(|r| *r.borrow_mut() = Some(None));
    let out = f();
    READ.with(|r| *r.borrow_mut() = None);
    out
}

/// Every feature of the document, in history order: as read once while
/// [`reading`], else from the host.
fn features() -> Rc<Vec<Node>> {
    READ.with(|r| {
        let mut r = r.borrow_mut();
        match &mut *r {
            Some(Some(all)) => all.clone(),
            Some(slot) => slot.insert(Rc::new(host::features())).clone(),
            None => Rc::new(host::features()),
        }
    })
}

/// Feature `id`: from the features read once while [`reading`], else from
/// the host.
fn feature(id: &str) -> Option<Node> {
    if READ.with(|r| r.borrow().is_some()) {
        return features().iter().find(|n| n.id == id).cloned();
    }
    host::feature(id)
}

/// A feature of the document, if it is a setup.
fn setup_node(id: &str) -> Option<(Node, SetupData)> {
    let node = feature(id).filter(|n| n.kind == SETUP_KIND)?;
    let data = SetupData::from(&node.data)?;
    Some((node, data))
}

/// A feature of the document, if it is an operation.
fn operation_node(id: &str) -> Option<(Node, OperationData)> {
    let node = feature(id).filter(|n| n.kind == OPERATION_KIND)?;
    let data = OperationData::from(&node.data)?;
    Some((node, data))
}

fn setups() -> Vec<(Node, SetupData)> {
    features()
        .iter()
        .filter(|n| n.kind == SETUP_KIND)
        .filter_map(|n| {
            let data = SetupData::from(&n.data)?;
            Some((n.clone(), data))
        })
        .collect()
}

/// Every operation of setup `setup`, in the document's order.
fn operations_of(setup: &str) -> Vec<(Node, OperationData)> {
    features()
        .iter()
        .filter(|n| n.kind == OPERATION_KIND)
        .filter_map(|n| {
            let data = OperationData::from(&n.data)?;
            (data.setup.as_deref() == Some(setup)).then(|| (n.clone(), data))
        })
        .collect()
}

/// Whether feature `id`, a setup or an operation, runs.
fn runs(id: &str) -> Option<bool> {
    setup_node(id)
        .map(|(_, d)| d.enabled)
        .or_else(|| operation_node(id).map(|(_, d)| d.operation.enabled))
}

/// Turn feature `id`, a setup or an operation, on or off. An operation off is
/// left out of its setup's run, so what comes after it cuts as if it were
/// not there; a setup off is not run at all. Either is hidden while off, so
/// the tree greys it and nothing of it is drawn, and shown again when on.
fn set_enabled(id: &str, on: bool) -> Result<(), String> {
    if let Some((_, mut data)) = setup_node(id) {
        data.enabled = on;
        host::set_feature_data(id, data.to_value())?;
    } else if let Some((_, mut data)) = operation_node(id) {
        data.operation.enabled = on;
        host::set_feature_data(id, data.to_value())?;
    } else {
        return Err("no setup or operation by that id".into());
    }
    host::call(
        calls::SET_FEATURE_VISIBLE,
        json!({ "id": id, "visible": on }),
    )
    .map(|_| ())
}

/// The setup `setup` (feature `id`) carries on from, when its stock is what an
/// earlier one leaves: the one it names, or the one just before it, which
/// must come before it in the document.
fn source_of(id: &str, setup: &SetupData) -> Option<Result<(Node, SetupData), String>> {
    if setup.stock != StockKind::Earlier {
        return None;
    }
    let all = setups();
    let earlier = &all[..all
        .iter()
        .position(|(n, _)| n.id == id)
        .unwrap_or(all.len())];
    let source = match &setup.stock_from {
        Some(from) => earlier
            .iter()
            .find(|(n, _)| &n.id == from)
            .cloned()
            .ok_or_else(|| {
                "The setup its stock carries on from is gone, or no longer before it.".into()
            }),
        None => earlier
            .last()
            .cloned()
            .ok_or_else(|| "There is no setup before it for its stock to carry on from.".into()),
    };
    // One turned off leaves nothing: it never runs.
    Some(source.and_then(|(n, d)| {
        if d.enabled {
            Ok((n, d))
        } else {
            Err(format!(
                "The setup its stock carries on from, {}, is turned off: turn it on, or \
                 take the stock from another.",
                n.name
            ))
        }
    }))
}

/// The request that runs setup `id`, or why it cannot run.
/// A setup that carries on from an earlier one asks with that one's own
/// request beside it.
fn request_for(node: &Node, setup: &SetupData) -> Result<HelperRequest, String> {
    if let Some(problem) = setup.problem() {
        return Err(problem);
    }
    let bodies = host::bodies();
    let models = models_of(setup, |b| {
        bodies.iter().find(|x| x.id == b).map(|x| x.name.clone())
    });
    let mut operations = operations_of(&node.id);
    if operations.is_empty() {
        return Err("The setup has no operations yet: add one.".into());
    }
    for (op, data) in &mut operations {
        if data.operation.enabled
            && let Some(problem) = data.problem(&setup.tools)
        {
            return Err(format!("{}: {problem}", op.name));
        }
        // Each sketch's curves as they are now, at the setup's tolerance.
        let sketches = if data.operation.enabled {
            data.sketches.clone()
        } else {
            Vec::new()
        };
        for sketch in &sketches {
            let curves = sketches::of(sketch, setup.tolerance).ok_or_else(|| {
                format!(
                    "{}: a sketch it follows is gone, or closes nothing.",
                    op.name
                )
            })?;
            data.curves.extend(curves);
        }
    }
    let named: Vec<(String, &OperationData)> = operations
        .iter()
        .map(|(n, d)| (n.name.clone(), d))
        .collect();
    let job = data::job(&node.name, setup, &models, &named);
    let mut shapes = BTreeMap::new();
    for m in &models {
        let body = bodies
            .iter()
            .find(|b| b.id == m.body)
            .ok_or_else(|| format!("The body {} is no longer there.", m.name))?;
        let blob = host::body_shape(&m.body)
            .ok_or_else(|| format!("{} has no solid to machine.", m.name))?;
        let text = String::from_utf8(blob)
            .map_err(|_| format!("{}'s solid is not in a form the helper reads.", m.name))?;
        shapes.insert(
            m.name.clone(),
            Shape {
                text,
                placement: if body.placement.iter().all(|v| v.is_finite()) {
                    body.placement
                } else {
                    IDENTITY
                },
            },
        );
    }
    let from = match source_of(&node.id, setup) {
        None => None,
        Some(source) => {
            let (n, s) = source?;
            let earlier = request_for(&n, &s)
                .map_err(|e| format!("The setup it carries on from, {}: {e}", n.name))?;
            Some(Box::new(earlier))
        }
    };
    Ok(HelperRequest {
        version: VERSION,
        job: camcraft_session::job_text(&job).map_err(|e| e.to_string())?,
        shapes,
        max_points: DRAWN_POINTS,
        post: setup.post_profile.clone(),
        read: None,
        from,
    })
}

fn status(state: FeatureState, note: &str) -> FeatureStatus {
    FeatureStatus {
        state,
        note: Some(note.to_string()),
        ..Default::default()
    }
}

/// What a finished run says of a setup, or of its operation `index`: failed
/// when a check failed, warnings when one warned or camcraft did, done when
/// all passed.
fn run_status(run: &cam_protocol::Run, index: Option<usize>) -> FeatureStatus {
    let op = index.and_then(|i| run.operations.get(i));
    match op.map(|o| o.made.as_str()) {
        Some("disabled") => return status(FeatureState::NotRun, "Turned off"),
        Some("skipped") => return status(FeatureState::NotRun, "Skipped: nothing to cut"),
        _ => {}
    }
    let findings: Vec<&cam_protocol::Finding> = run
        .findings
        .iter()
        .filter(|f| index.is_none() || f.operation == index)
        .collect();
    let first = |level: Level| findings.iter().find(|f| f.level == level);
    if let Some(f) = first(Level::Fail) {
        return status(FeatureState::Failed, &f.text);
    }
    if index.is_none() && !run.passed {
        return status(FeatureState::Failed, "It failed its checks");
    }
    let warning = first(Level::Warn)
        .map(|f| f.text.clone())
        .or_else(|| match op {
            Some(o) => o.warnings.first().cloned(),
            None => run.warnings.first().cloned(),
        });
    match (warning, op) {
        (Some(w), _) => status(FeatureState::Warnings, &w),
        (None, Some(o)) => status(FeatureState::Done, &o.summary),
        (None, None) => status(
            FeatureState::Done,
            &format!("Every check passed ({:.1} s)", run.seconds),
        ),
    }
}

/// How a setup's run stands, for its badge, or for operation `op`'s (its
/// feature id) when one is given.
fn computed_status(computed: &Computed, op: Option<&str>) -> FeatureStatus {
    match &computed.state {
        State::Queued => status(FeatureState::Running, "Waiting for another setup to finish"),
        State::Running(job) => FeatureStatus {
            job: Some(*job),
            ..status(FeatureState::Running, "Working out the toolpaths")
        },
        State::Stopped => status(FeatureState::OutOfDate, "Its run was stopped"),
        State::Failed(why) => status(FeatureState::Failed, why),
        State::Ready(run) => match op {
            None => run_status(run, None),
            Some(id) => match computed.operations.iter().position(|o| o == id) {
                Some(index) => run_status(run, Some(index)),
                // Added since it ran: it runs with the next.
                None => status(FeatureState::NotRun, "Not run yet"),
            },
        },
    }
}

impl Cam {
    /// The setup whose run plays back: the open setup, or the open
    /// operation's.
    fn played_setup(&self) -> Option<String> {
        let e = self.editing.as_ref()?;
        match e.kind {
            Kind::Setup => Some(e.id.clone()),
            Kind::Operation => operation_node(&e.id)?.1.setup,
        }
    }

    /// An operation dropped in the tree just before or after `target`, as
    /// camcraft's app drops one: in its setup or another, at that place,
    /// the setups it left and joined run again. Whether it was one this
    /// takes: a setup's place is the host's to change.
    fn move_operation(&mut self, feature: &str, target: &str, before: bool) -> bool {
        let Some((_, mut op)) = operation_node(feature) else {
            return false;
        };
        // Into the target's setup: the target, or the setup it is in.
        let joins = match operation_node(target) {
            Some((_, t)) => t.setup,
            None => setup_node(target).map(|(n, _)| n.id),
        };
        let Some(joins) = joins else {
            return false;
        };
        // After the feature before the target, or after the target; first
        // of all when nothing comes before it.
        // Counted without the one moved, which leaves its place.
        let mut order = host::features();
        order.retain(|n| n.id != feature);
        let at = order.iter().position(|n| n.id == target);
        let after = match (at, before) {
            (Some(i), true) => i.checked_sub(1).and_then(|j| order.get(j)),
            (Some(i), false) => order.get(i),
            (None, _) => return false,
        }
        .map(|n| n.id.clone());
        let mut args = json!({ "id": feature });
        if let Some(after) = &after {
            args["after"] = json!(after);
        }
        if let Err(e) = host::call(calls::MOVE_FEATURE, args) {
            host::warn(&format!("Not moved: {e}"));
            return true;
        }
        let left = op.setup.replace(joins.clone());
        if left.as_deref() != Some(joins.as_str()) {
            let _ = host::set_feature_data(feature, op.to_value());
        }
        if let Some(left) = left.filter(|l| *l != joins) {
            self.refresh(&left, false);
        }
        self.refresh(&joins, false);
        true
    }

    /// Arm `slot` for `feature`, or disarm it if it is armed already.
    fn arm(&mut self, feature: &str, slot: Option<picking::Slot>) {
        let Some(slot) = slot else {
            return;
        };
        let again = self
            .armed
            .as_ref()
            .is_some_and(|a| a.feature == feature && a.slot == slot);
        let alone = operation_node(feature).is_some_and(|(_, d)| d.strategy().uses().traces);
        self.armed = (!again).then(|| picking::Armed {
            feature: feature.into(),
            slot,
            alone,
            hover: None,
        });
    }

    /// The tool's axis of the setup `feature` is, or is in: what an edge
    /// loop is flat along.
    fn axis_of(&self, feature: &str) -> [f32; 3] {
        let setup = self.setup_of(feature).and_then(|s| setup_node(&s));
        setup.map_or([0.0, 0.0, 1.0], |(_, d)| d.up().map(|v| v as f32))
    }

    /// An armed slot's share of `input`: what a click would take as the
    /// pointer moves, and taking it on a click. `None` for input it leaves.
    fn pick_input(&mut self, input: &Input) -> Option<bool> {
        let armed = self.armed.clone()?;
        match &input.event {
            Event::Move => {
                let up = self.axis_of(&armed.feature);
                let hover = picking::hover(armed.slot, &input.pointer, up, armed.alone);
                if let Some(a) = &mut self.armed {
                    a.hover = hover;
                }
                host::redraw();
                Some(false)
            }
            Event::Press {
                button: Button::Left,
            } => {
                // What the pointer is over now, when no move since said:
                // the view can tell an input what is under it before it has
                // found it.
                let hover = armed.hover.or_else(|| {
                    let up = self.axis_of(&armed.feature);
                    picking::hover(armed.slot, &input.pointer, up, armed.alone)
                })?;
                self.take(&armed.feature, armed.slot, hover, &input.pointer);
                Some(true)
            }
            Event::Key { key, down: true } if key == "Escape" => {
                self.armed = None;
                Some(true)
            }
            _ => None,
        }
    }

    /// What an armed slot's click takes, into `feature`'s data: a face or an
    /// edge loop for an operation, taken out again when it is in already; a
    /// work zero for a setup, which disarms it.
    fn take(
        &mut self,
        feature: &str,
        slot: picking::Slot,
        hover: picking::Hover,
        pointer: &Pointer,
    ) {
        use picking::{Hover, Slot};
        if matches!(slot, Slot::AxisFace | Slot::AxisEdge | Slot::XEdge) {
            // Which way it points, as the view has it under the pointer.
            let (body, at, toward) = match hover {
                Hover::Face { body, at, .. } => {
                    let normal = pointer
                        .hover_face
                        .as_ref()
                        .or(self.pointer.hover_face.as_ref())
                        .map(|f| f.normal);
                    (body, at, normal)
                }
                Hover::Edge { body, at, .. } => {
                    let along = pointer
                        .hover_edge
                        .as_ref()
                        .or(self.pointer.hover_edge.as_ref())
                        .map(|e| e.direction);
                    (body, at, along)
                }
                Hover::Point { .. } => return,
            };
            if let (Some((_, mut setup)), Some(toward)) = (setup_node(feature), toward) {
                let pick = data::AxisPick {
                    body,
                    at: at.map(f64::from),
                    toward: toward.map(f64::from),
                    reverse: false,
                };
                if slot == Slot::XEdge {
                    setup.machine_x = data::MachineX::Edge;
                    setup.x_pick = Some(pick);
                } else {
                    setup.tool_axis = if slot == Slot::AxisFace {
                        data::ToolAxis::Face
                    } else {
                        data::ToolAxis::Edge
                    };
                    setup.axis_pick = Some(pick);
                }
                self.write_setup(feature, &setup);
            }
            self.armed = None;
            return;
        }
        match hover {
            Hover::Point { body, snap, at, .. } => {
                if let Some((_, mut setup)) = setup_node(feature) {
                    setup.zero = data::Zero::Picked;
                    setup.zero_point = Some(data::ZeroPoint {
                        body,
                        snap,
                        at: at.map(f64::from),
                    });
                    let _ = host::set_feature_data(feature, setup.to_value());
                    self.refresh(feature, false);
                }
                self.armed = None;
            }
            Hover::Face { body, at, .. } => {
                let Some((_, mut op)) = operation_node(feature) else {
                    return;
                };
                let name = self.pointer.hover_face.as_ref().and_then(|f| f.name);
                let had = op.faces.len();
                op.faces
                    .retain(|p| !(p.body == body && name.is_some() && p.name == name));
                if op.faces.len() == had {
                    op.faces.push(data::Pick {
                        body,
                        at: at.map(f64::from),
                        name,
                        faces: None,
                    });
                }
                self.write_operation(feature, &op);
            }
            Hover::Edge { body, at, .. } => {
                let Some((_, mut op)) = operation_node(feature) else {
                    return;
                };
                let faces = self.pointer.hover_edge.as_ref().and_then(|e| e.faces);
                let had = op.edges.len();
                op.edges
                    .retain(|p| !(p.body == body && faces.is_some() && p.faces == faces));
                if op.edges.len() == had {
                    op.edges.push(data::Pick {
                        body,
                        at: at.map(f64::from),
                        name: None,
                        faces,
                    });
                }
                self.write_operation(feature, &op);
            }
        }
    }

    /// Keep a session playing back `setup`'s current run while it plays
    /// back, and let it answer; none otherwise.
    fn keep_session(&mut self, setup: Option<&str>) {
        let wanted = setup.filter(|_| self.playing.is_some()).and_then(|setup| {
            let c = self.runs.get(setup)?;
            matches!(c.state, State::Ready(_)).then_some((setup.to_string(), c.key))
        });
        let Some((setup, key)) = wanted else {
            self.session = None;
            return;
        };
        let current = self
            .session
            .as_ref()
            .is_some_and(|s| s.setup == setup && s.key == key);
        if !current {
            self.session = None;
            // The run's own request, not the document's as it is now: the
            // document can change without the package hearing of it (the
            // Data panel, an undo), and a session run from it would time
            // its moves differently from the run the timeline shows.
            let opened = self
                .runs
                .get(&setup)
                .and_then(|c| Some((c.ran_from()?, c.shown()?.timing)))
                .ok_or_else(|| "the run's request is not kept".to_string())
                .and_then(|(r, timing)| session::Session::open(&setup, key, r, timing));
            match opened {
                Ok(s) => self.session = Some(s),
                Err(e) => host::warn(&format!("Playback stays coarse: {e}")),
            }
        }
        if let (Some(s), Some(p)) = (&mut self.session, &self.playing) {
            s.pump(p.at);
        }
        if let Some(why) = self.session.as_mut().and_then(session::Session::failure) {
            host::warn(&format!("Playback stays coarse: {why}"));
        }
    }

    /// Where the transport steps for `setup`'s `run`: every move once the
    /// session knows them, else the run's moments.
    fn steps(&self, setup: &str, run: &cam_protocol::Run) -> Vec<f64> {
        let times = self
            .session
            .as_ref()
            .filter(|s| s.setup == setup)
            .and_then(|s| s.times());
        playback::steps(run, times)
    }

    /// The operation under way in the playback of `setup`'s run, and how
    /// many of its moves are made.
    fn under_way(&self, setup: &str) -> Option<(usize, usize)> {
        let run = self.runs.get(setup)?.shown()?;
        let exact = self
            .session
            .as_ref()
            .filter(|s| s.setup == setup)
            .and_then(|s| s.at.as_ref())
            .map(|a| (a.op, a.moves, a.of));
        let playing = self.playing.as_ref().filter(|p| p.setup == setup);
        playback::under_way(run, playing, exact).map(|(op, made, _)| (op, made))
    }

    /// A click on a line of the program listing: the playhead goes to its
    /// move. Whether it was one.
    fn listing_event(&mut self, event: &PanelEvent) -> bool {
        let Some(setup) = self.played_setup() else {
            return false;
        };
        let Some(run) = self.runs.get(&setup).and_then(|c| c.shown()) else {
            return false;
        };
        let Some((op, made)) = listing::clicked(run, event, &self.listing.borrow()) else {
            return false;
        };
        self.seek_move(&setup, op, made);
        true
    }

    /// Take the playhead of `setup`'s playback to where operation `op` has
    /// made `made` moves: exactly once the session knows when each move
    /// ends, between the run's moments until then.
    fn seek_move(&mut self, setup: &str, op: usize, made: usize) {
        let Some(run) = self.runs.get(setup).and_then(|c| c.shown()) else {
            return;
        };
        let times = self
            .session
            .as_ref()
            .filter(|s| s.setup == setup)
            .and_then(|s| s.times());
        let at = playback::time_of(run, times, op, made);
        let steps = self.steps(setup, run);
        playback::apply(
            &mut self.playing,
            &mut self.transport,
            setup,
            run,
            playback::Command::Seek(at),
            &steps,
        );
        self.aim = times.is_none().then_some((op, made, at));
    }

    /// Once the session knows the moves, a line clicked before it did gets
    /// its own move, if the playhead still stands where the click left it.
    fn take_aim(&mut self, setup: Option<&str>) {
        let Some((op, made, at)) = self.aim else {
            return;
        };
        let standing = self
            .playing
            .as_ref()
            .is_some_and(|p| p.at == at && p.running.is_none() && Some(p.setup.as_str()) == setup);
        if !standing {
            self.aim = None;
            return;
        }
        let known = self
            .session
            .as_ref()
            .is_some_and(|s| Some(s.setup.as_str()) == setup && s.times().is_some());
        if let (true, Some(setup)) = (known, setup) {
            self.seek_move(setup, op, made);
        }
    }

    /// How many steps Shift and an arrow take.
    fn big_step(&self) -> usize {
        match self.session.as_ref().and_then(|s| s.times()) {
            Some(_) => playback::BIG_STEP_MOVES,
            None => playback::BIG_STEP_MOMENTS,
        }
    }

    /// Run setup `id` unless what it is run from is unchanged; `again` runs a
    /// stopped or failed one again too.
    fn refresh(&mut self, id: &str, again: bool) {
        self.refresh_from(id, again, true);
    }

    /// [`Cam::refresh`], and the setups that carry on from it after it;
    /// `earlier` runs first the setups it carries on from, unless they have
    /// run as they are, so the helper takes what each left from its cache
    /// rather than run it again.
    fn refresh_from(&mut self, id: &str, again: bool, earlier: bool) {
        let Some((node, setup)) = setup_node(id) else {
            self.runs.forget(id);
            return;
        };
        // Turned off, it has no run, and what it had is gone with it.
        if !setup.enabled {
            self.runs.forget(id);
        } else {
            if earlier {
                self.queue_earlier(id, &setup);
            }
            self.queue(&node, &setup, again);
        }
        // A setup whose stock is what this one leaves runs again with it,
        // or says it cannot. Each comes after the one it carries on from, so
        // this ends.
        for (later, data) in setups() {
            if source_of(&later.id, &data)
                .and_then(Result::ok)
                .is_some_and(|(n, _)| n.id == id)
            {
                self.refresh_from(&later.id, again, false);
            }
        }
        // The tree's badges follow the run.
        host::redraw();
    }

    /// Run `setup` (feature `node`) unless it has run, or is to, as it is.
    fn queue(&mut self, node: &Node, setup: &SetupData, again: bool) {
        match request_for(node, setup) {
            Ok(request) => {
                let operations = operations_of(&node.id)
                    .into_iter()
                    .map(|(n, _)| n.id)
                    .collect();
                let after = source_of(&node.id, setup)
                    .and_then(Result::ok)
                    .map(|(n, _)| n.id);
                self.runs.want(&node.id, request, operations, again, after);
            }
            Err(why) => self.runs.refuse(&node.id, why),
        }
    }

    /// Run the setups `setup` (feature `id`) carries on from, the earliest
    /// first, unless each has run, or is to, as it is.
    fn queue_earlier(&mut self, id: &str, setup: &SetupData) {
        if let Some(Ok((node, data))) = source_of(id, setup) {
            self.queue_earlier(&node.id, &data);
            self.queue(&node, &data, false);
        }
    }

    /// How a setup's or an operation's work stands, for its badge in the
    /// tree: its setup's run, and of that run, what is the operation's.
    ///
    /// printCAD asks for it without the document, so the setup's operations
    /// cannot be looked up then: the run says which of its operations this
    /// is.
    fn status_of(&self, node: &Node) -> FeatureStatus {
        // An operation's data read once: a big one takes a while.
        let op_data = (node.kind != SETUP_KIND)
            .then(|| OperationData::from(&node.data))
            .flatten();
        let off = match &op_data {
            None if node.kind == SETUP_KIND => {
                SetupData::from(&node.data).is_some_and(|d| !d.enabled)
            }
            None => false,
            Some(d) => !d.operation.enabled,
        };
        if off {
            return status(
                FeatureState::NotRun,
                "Turned off: not run, simulated, checked or posted",
            );
        }
        // Made from a template, it waits for its picks on this part.
        if let Some(what) = op_data.as_ref().and_then(|d| d.waits_for_pick()) {
            return status(FeatureState::Warnings, &format!("Needs a pick: {what}"));
        }
        let (setup, op) = if node.kind == SETUP_KIND {
            (Some(node.id.clone()), None)
        } else {
            (op_data.and_then(|d| d.setup), Some(node.id.as_str()))
        };
        match setup.and_then(|s| self.runs.get(&s)) {
            Some(computed) => computed_status(computed, op),
            None => status(FeatureState::NotRun, "Not run yet"),
        }
    }

    /// Run the setup a feature is, or belongs to, again if it changed.
    fn refresh_for(&mut self, feature: &str, again: bool) {
        if setup_node(feature).is_some() {
            self.refresh(feature, again);
        } else if let Some((_, op)) = operation_node(feature)
            && let Some(setup) = op.setup
        {
            self.refresh(&setup, again);
        }
    }

    fn refresh_all(&mut self, again: bool) {
        for (node, _) in setups() {
            self.refresh(&node.id, again);
        }
    }

    /// The setup a new operation goes in: the open one, the selected one or the
    /// one the selected operation is in, else the last.
    fn current_setup(&self) -> Option<String> {
        let from = |id: &str| -> Option<String> {
            if setup_node(id).is_some() {
                return Some(id.to_string());
            }
            operation_node(id)?
                .1
                .setup
                .filter(|s| setup_node(s).is_some())
        };
        self.editing
            .as_ref()
            .and_then(|e| from(&e.id))
            .or_else(|| self.pointer.active_feature.as_deref().and_then(from))
            .or_else(|| setups().last().map(|(n, _)| n.id.clone()))
    }

    /// The bodies a new setup machines: the selected one, else every shown one.
    fn chosen_bodies(&self) -> Vec<String> {
        if let Some(b) = self.pointer.selected_body.clone() {
            return vec![b];
        }
        host::bodies()
            .into_iter()
            .filter(|b| !b.hidden && b.bounds.is_some())
            .map(|b| b.id)
            .collect()
    }

    fn make_setup(&self, bodies: Vec<String>, name: Option<&str>) -> Result<String, String> {
        let count = setups().len();
        let name = name
            .map(str::to_string)
            .unwrap_or_else(|| format!("Setup {}", count + 1));
        let mut data = SetupData::new(bodies, &self.settings.post, self.settings.tolerance);
        data.post_profile = self.settings.posts.get(&data.post).cloned();
        host::add_feature(SETUP_KIND, &name, None, data.to_value())
    }

    /// Add an operation of `strategy` to setup `setup`, cutting with the tool
    /// the setup's last operation used.
    fn make_operation(
        &self,
        setup: &str,
        strategy: StrategySpec,
        name: Option<&str>,
    ) -> Result<String, String> {
        let (_, setup_data) = setup_node(setup).ok_or("no such setup")?;
        let ops = operations_of(setup);
        let mut data = OperationData::new(Some(setup.to_string()), strategy);
        // Custom G-code cuts with nothing.
        let last = ops
            .iter()
            .rev()
            .find_map(|(_, d)| d.operation.tool.clone())
            .filter(|_| !data.is_custom());
        if let Some(tool) = last.and_then(|t| Some((setup_data.tools.get(&t)?.clone(), t))) {
            let preset = tool.0.presets.first().cloned();
            data.use_tool(&tool.1, &tool.0, preset.as_ref(), setup_data.tolerance);
        }
        let word = strategy_word(&strategy);
        let count = host::features()
            .iter()
            .filter(|n| n.kind == OPERATION_KIND && n.name.starts_with(word))
            .count();
        let name = name
            .map(str::to_string)
            .unwrap_or_else(|| format!("{word} {}", count + 1));
        host::add_feature(OPERATION_KIND, &name, None, data.to_value())
    }

    fn open(&mut self, id: &str, fresh: bool) {
        let kind = if setup_node(id).is_some() {
            Kind::Setup
        } else if operation_node(id).is_some() {
            Kind::Operation
        } else {
            return;
        };
        let Some(node) = host::feature(id) else {
            return;
        };
        self.editing = Some(Editing {
            id: id.to_string(),
            kind,
            opened: node.data,
            fresh,
        });
        self.hint = None;
        self.chosen_body = None;
        self.template_ui.first = false;
        self.template_ui.name = None;
        self.template_ui.rules = None;
        self.load_templates();
        self.refresh_for(id, false);
        if let Some(setup) = self.setup_of(id) {
            self.find_lines(&setup);
        }
    }

    /// Check setups `setups` (every setup when `None`) for changes, and run
    /// those that changed, once the [`LATER`] job ends; at once if it will
    /// not start.
    fn refresh_later(&mut self, setups: Option<Vec<String>>) {
        if setups.as_ref().is_some_and(Vec::is_empty) {
            return;
        }
        if let Some((_, waiting)) = &mut self.later {
            match (waiting.as_mut(), setups) {
                (Some(w), Some(more)) => {
                    for s in more {
                        if !w.contains(&s) {
                            w.push(s);
                        }
                    }
                }
                (_, None) => *waiting = None,
                (None, Some(_)) => {}
            }
            return;
        }
        match host::start_job(LATER, "") {
            Ok(job) => self.later = Some((job, setups)),
            Err(_) => reading(|| match setups {
                None => self.refresh_all(false),
                Some(setups) => {
                    for setup in setups {
                        self.refresh(&setup, false);
                    }
                }
            }),
        }
    }

    /// Where the lines of the program `setup`'s panel lists start, found
    /// now, in an event printCAD gives a second, rather than in the first
    /// frame that lists it: a setup's program can be megabytes.
    fn find_lines(&self, setup: &str) {
        if self.played_setup().as_deref() != Some(setup) {
            return;
        }
        let (Some(run), Some((op, _))) = (
            self.runs.get(setup).and_then(|c| c.shown()),
            self.under_way(setup),
        ) else {
            return;
        };
        self.listing.borrow_mut().find(run, op);
    }

    fn write_setup(&mut self, id: &str, data: &SetupData) {
        match host::set_feature_data(id, data.to_value()) {
            Ok(()) => self.refresh(id, false),
            Err(e) => host::error(&e),
        }
    }

    fn write_operation(&mut self, id: &str, data: &OperationData) {
        match host::set_feature_data(id, data.to_value()) {
            Ok(()) => {
                if let Some(setup) = &data.setup {
                    self.refresh(setup, false);
                }
            }
            Err(e) => host::error(&e),
        }
    }

    /// Ask where to save `program`.
    fn save(program: &cam_protocol::Program) {
        host::request(Request::SaveFile {
            name: format!("{}.{}", program.name, program.extension),
            kind: "G-code".into(),
            extension: program.extension.clone(),
            contents: program.text.clone().into_bytes(),
        });
    }

    /// Every program of a setup, written into a folder the user picks.
    fn save_all(programs: &[cam_protocol::Program]) {
        host::request(Request::SaveFiles {
            files: programs
                .iter()
                .map(|p| FileToWrite {
                    name: format!("{}.{}", p.name, p.extension),
                    contents: p.text.clone().into_bytes(),
                })
                .collect(),
        });
    }

    /// Ask where to save a setup's sheet, `html`, named after it.
    fn save_sheet(setup: &str, html: String) {
        host::request(Request::SaveFile {
            name: format!(
                "{}-setup-sheet.html",
                camcraft_session::setup_program_name(setup)
            ),
            kind: "Setup sheet".into(),
            extension: "html".into(),
            contents: html.into_bytes(),
        });
    }

    /// The sheet of the setup of `id` (the setup, or one of its operations),
    /// as HTML, and the setup's name, once it has run: failed checks and
    /// all, which the sheet says first.
    fn sheet_of(&self, id: &str) -> Result<(String, String), String> {
        let setup = self
            .setup_of(id)
            .ok_or("no setup or operation by that id")?;
        let (node, data) = setup_node(&setup).ok_or("no such setup")?;
        if !data.enabled {
            return Err("The setup is turned off.".into());
        }
        let computed = self.runs.get(&setup).ok_or("The setup has not been run.")?;
        let run = match &computed.state {
            State::Ready(run) => run,
            State::Queued | State::Running(_) => {
                return Err("The setup is being run; ask again when it is done.".into());
            }
            State::Stopped => return Err("The setup's run was stopped.".into()),
            State::Failed(e) => return Err(e.clone()),
        };
        let bodies = host::bodies();
        let html = sheet::Sheet {
            name: &node.name,
            setup: &data,
            bodies: data
                .bodies
                .iter()
                .map(|b| {
                    bodies
                        .iter()
                        .find(|x| &x.id == b)
                        .map_or_else(|| b.clone(), |x| x.name.clone())
                })
                .collect(),
            run,
            carried_from: source_of(&setup, &data)
                .and_then(Result::ok)
                .map(|(n, _)| n.name),
        }
        .html();
        Ok((node.name, html))
    }

    /// The program of operation `op`, or of its setup when it is posted as
    /// one; or of setup `op`, posted as one; when the setup has run.
    fn program_of(&self, op: &str, force: bool) -> Result<cam_protocol::Program, String> {
        let (node, setup) = match operation_node(op) {
            Some((node, data)) => {
                if !data.operation.enabled {
                    return Err(format!("{} is turned off.", node.name));
                }
                (
                    Some(node),
                    data.setup.ok_or("The operation is in no setup.")?,
                )
            }
            None if setup_node(op).is_some() => (None, op.to_string()),
            None => return Err(format!("no operation or setup {op}")),
        };
        if runs(&setup) == Some(false) {
            return Err("The setup is turned off.".into());
        }
        let computed = self.runs.get(&setup).ok_or("Its setup has not been run.")?;
        let run = match &computed.state {
            State::Ready(run) => run,
            State::Queued | State::Running(_) => {
                return Err("Its setup is being run; ask again when it is done.".into());
            }
            State::Stopped => return Err("Its setup's run was stopped.".into()),
            State::Failed(e) => return Err(e.clone()),
        };
        if !run.passed && !force {
            return Err(
                "camcraft refuses to post a setup that failed its checks: read them first.".into(),
            );
        }
        if let Some(p) = &run.program {
            return Ok(p.clone());
        }
        let Some(node) = node else {
            return Err(
                "The setup is posted one program an operation: ask for an operation's.".into(),
            );
        };
        let index = operations_of(&setup)
            .iter()
            .position(|(n, _)| n.id == node.id)
            .ok_or("The operation is not in its setup.")?;
        run.operations
            .get(index)
            .and_then(|o| o.program.clone())
            .ok_or_else(|| format!("{} made no toolpath.", node.name))
    }
}

impl Bench for Cam {
    fn describe(&self) -> Registration {
        let param = |name: &str, kind: ParamKind, required: bool, doc: &str| Param {
            name: name.into(),
            kind,
            required,
            doc: doc.into(),
            ..Default::default()
        };
        let id_only = |doc: &str| vec![param("id", ParamKind::Id, true, doc)];
        let tool = |suffix: &str, label: &str, icon: &str| Tool {
            id: id(suffix),
            label: label.into(),
            icon: icon.into(),
            behavior: ToolBehavior::Action,
            ..Default::default()
        };
        let strategies = STRATEGIES
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>()
            .join(", ");
        Registration {
            label: "CAM".into(),
            description: "3-axis CNC toolpaths on the document's solids, checked against the \
                          material and written as G-code"
                .into(),
            icon: "setup".into(),
            tools: vec![
                tool(NEW_SETUP, "New setup", "setup"),
                tool(NEW_OPERATION, "New operation", "operation"),
                tool(GENERATE, "Generate", "generate"),
                tool(FROM_TEMPLATE, "Setup from template", "template"),
            ],
            commands: vec![
                Command {
                    id: id(SETUP),
                    summary: "Make a setup: the bodies it machines, its stock and its post".into(),
                    params: vec![
                        param(
                            "bodies",
                            ParamKind::List,
                            false,
                            "body ids; every shown body when left out",
                        ),
                        param(
                            "margin",
                            ParamKind::Number,
                            false,
                            "mm of stock round the bodies and on top (a cylinder's: \
                             round them and at each end); 2",
                        ),
                        param(
                            "stock",
                            ParamKind::Any,
                            false,
                            "box (round the bodies, the default), fixed-box or cylinder; or \
                             {from = \"previous\"}, what the setup before it leaves, or \
                             {from = <a setup's name>}, what that earlier one leaves",
                        ),
                        param(
                            "size",
                            ParamKind::List,
                            false,
                            "a fixed box's {x, y, z} in mm",
                        ),
                        param(
                            "axis",
                            ParamKind::String,
                            false,
                            "a cylinder's: tool (up the tool, the default), x or y (lying \
                             along the machine's)",
                        ),
                        param(
                            "diameter",
                            ParamKind::Number,
                            false,
                            "a cylinder of a fixed size, mm; with length",
                        ),
                        param("length", ParamKind::Number, false, "a fixed cylinder's, mm"),
                        param(
                            "tool_axis",
                            ParamKind::Any,
                            false,
                            "+x, -x, +y, -y, +z (the default) or -z: the document axis the \
                             tool points up; or {x, y, z}, a direction, which tilts the part \
                             for 3+2 machining",
                        ),
                        param(
                            "tilt",
                            ParamKind::Any,
                            false,
                            "{x, y}: the tool axis tilted from the document's +Z, degrees \
                             about its X then its Y, for 3+2 machining",
                        ),
                        param(
                            "machine_x",
                            ParamKind::String,
                            false,
                            "+x, -x, +y, -y, +z or -z: the document axis machine X lies \
                             along, made square to a tilted tool axis (+x, or one square to \
                             the tool)",
                        ),
                        param(
                            "post",
                            ParamKind::String,
                            false,
                            "grbl, masso, masso-inch, centroid-avid, centroid-avid-inch, \
                             linuxcnc, linuxcnc-inch, fanuc, fanuc-inch, haas, haas-inch, \
                             mach, mach-inch, heidenhain or heidenhain-inch",
                        ),
                        param(
                            "wcs",
                            ParamKind::String,
                            false,
                            "the work offset its programs select: G54 (the default) to G59, \
                             or one past them its post has, as G59.1 on LinuxCNC",
                        ),
                        param(
                            "work_offset",
                            ParamKind::Any,
                            false,
                            "{x, y, z}: where the work zero is in machine coordinates, mm, as \
                             the work offset holds it (Z the gauge line's for a post with G43, \
                             else the tip's), for checking the post's travel there",
                        ),
                        param(
                            "tolerance",
                            ParamKind::Number,
                            false,
                            "mm; the preferences' (0.05)",
                        ),
                        param(
                            "programs",
                            ParamKind::String,
                            false,
                            "setup (one program for the setup, the default) or operation \
                             (one an operation)",
                        ),
                        param(
                            "number",
                            ParamKind::Number,
                            false,
                            "the program's number, @number@ in a post's lines; 1001",
                        ),
                        param(
                            "material",
                            ParamKind::String,
                            false,
                            "what the stock is, for its setup sheet: 6061 aluminium, MDF",
                        ),
                        param(
                            "probe_map",
                            ParamKind::Any,
                            false,
                            "{path = a probe result file, read each run, or text = its \
                             contents; segment = mm, 1}: every cut's depth corrected to the \
                             probed surface. A LinuxCNC probe log (x y z ...) or a CSV of x, y, \
                             z, in the post's units and the setup's work coordinates",
                        ),
                        param("name", ParamKind::String, false, "the setup's name"),
                    ],
                    returns: "{feature}".into(),
                    notes: vec![
                        "The work zero is the stock's top, front-left corner.".into(),
                        "A tilted setup (tool_axis as a direction, or tilt) lays its stock out \
                         square to the tool. Its programs are in the tilted frame: a post with \
                         rotary axes stands the part up with G0 A.. B.. first; without, the part \
                         is fixtured tilted by hand. Pick a face or an edge for the tool axis in \
                         the setup's panel."
                            .into(),
                        "stock = {from = \"previous\"} starts from what the setup before it \
                         leaves after its last operation, turned as this one sees the part \
                         (as Fusion's Continue from previous setup); {from = \"Top\"} from \
                         what the earlier setup named Top leaves. It runs after that one, and \
                         again when that one changes."
                            .into(),
                        "A setup runs when it has operations; io.github.gilbertorconde.cam.status \
                         says where it stands."
                            .into(),
                    ],
                    see_also: vec![id(OPERATION), id(STATUS)],
                    ..Default::default()
                },
                Command {
                    id: id(OPERATION),
                    summary: "Add an operation to a setup".into(),
                    params: vec![
                        param("setup", ParamKind::Id, true, "the setup"),
                        param("strategy", ParamKind::String, true, &strategies),
                        param(
                            "tool",
                            ParamKind::String,
                            false,
                            "a tool of the setup, or of the library (copied in)",
                        ),
                        param("stepover", ParamKind::Number, false, "mm"),
                        param(
                            "stepdown",
                            ParamKind::Number,
                            false,
                            "mm per level; multiple depths on",
                        ),
                        param("feed", ParamKind::Number, false, "mm/min"),
                        param("plunge", ParamKind::Number, false, "mm/min"),
                        param(
                            "heights",
                            ParamKind::Any,
                            false,
                            "{clearance, retract, top, bottom: mm above what each is measured \
                             from; clearance_from, retract_from, top_from, bottom_from: \
                             stock-top|stock-bottom|model-top|model-bottom|absolute|selection, \
                             a top or bottom without one is the strategy's own}",
                        ),
                        param(
                            "linking",
                            ParamKind::Any,
                            false,
                            "{retract_feed, safe_distance, ramp_angle, lead_sweep: numbers; \
                             retraction: minimum|full; entry: helix|plunge|ramp; leads, \
                             reorder, allow_reverse: booleans; stay_down, lead_radius, \
                             ramp_feed, ramp_stepdown, helix_diameter, lead_vertical, \
                             lead_straight, lead_arc: {value, given}, the strategy's own \
                             unless given}",
                        ),
                        param(
                            "passes",
                            ParamKind::Any,
                            false,
                            "{max_cut_step, slope_from, slope_to, centre_x, centre_y, \
                             inner_radius, angle_from, angle_to, min_ring_length, \
                             pass_extension, tab_width, tab_height, tab_spacing, \
                             finishing_passes: numbers; centre, tabs, finishing, \
                             finishing_repeat, relief, use_holder (a 3D finish's Use shank \
                             and holder): booleans; tab_shape: rectangular|triangular; \
                             relief_style: dogbone|t-bone-x|t-bone-y|t-bone-long|\
                             t-bone-short; relief_angle: {value, given}, degrees, only \
                             corners sharper; \
                             hand: right|left; depth: engrave's and slot's, mm; chamfer_width, \
                             tip_offset: a chamfer's, mm; bore_pitch: a bore's helix, mm a \
                             turn; boss_width: circular's, mm; thread_pitch, thread_passes: \
                             a milled thread's; rest: adaptive's rest machining, a boolean, \
                             from the operations before it; rest_adjustment: mm, rest thinner \
                             than it left; probe_x_spacing, probe_y_spacing, probe_inset (mm), \
                             probe_feed (mm/min), probe_depth (mm below the top): a probe \
                             grid's; outer_radius, peck, chip_break, dwell, pitch, max_depth, \
                             finishing_stepover, finishing_feed, thread_diameter, \
                             probe_x_count, probe_y_count: {value, given}}",
                        ),
                        param(
                            "boundary",
                            ParamKind::Any,
                            false,
                            "what a 3D cut is kept to: stock|bounding-box|silhouette|selection \
                             (the picks and sketches), or {kind, containment: \
                             center|inside|outside, margin: mm, none unless given}",
                        ),
                        param(
                            "transform",
                            ParamKind::Any,
                            false,
                            "{linear, polar, mirror: booleans, each part on; x_count, \
                             x_spacing, y_count, y_spacing: rows; polar_count, polar_angle \
                             (degrees, 360 a whole turn), centre_x, centre_y: round a centre; \
                             mirror_axis: x|y, through_x, through_y: the mirror's line, along \
                             the setup's X or Y through that point; all from the work zero}",
                        ),
                        param(
                            "sketches",
                            ParamKind::List,
                            false,
                            "a list of sketch ids: their closed loops, text among them, are \
                             what it follows or cuts within (a 2D cut's outline then, and a \
                             3D cut's boundary unless one is given)",
                        ),
                        param(
                            "faces",
                            ParamKind::List,
                            false,
                            "faces picked: a list of {body, at = {x, y, z}}, three numbers for a point on each, \
                             in document coordinates; a hole's wall for a hole operation",
                        ),
                        param(
                            "edges",
                            ParamKind::List,
                            false,
                            "edges picked, as faces are: a hole's rim for a hole operation",
                        ),
                        param(
                            "holes",
                            ParamKind::Any,
                            false,
                            "drill, bore, circular, thread: {same_diameter: boolean, every \
                             hole as wide as one picked; diameter: mm, every hole that wide, \
                             picked or not; order: nearest|x-then-y|as-picked (nearest)}",
                        ),
                        param(
                            "conventional",
                            ParamKind::Bool,
                            false,
                            "profile, pocket, pocket-clearing, bore, circular, thread: cut \
                             conventional rather than climb",
                        ),
                        param(
                            "spring_pass",
                            ParamKind::Bool,
                            false,
                            "thread: the last radial pass cut twice",
                        ),
                        param(
                            "direction",
                            ParamKind::String,
                            false,
                            "engrave: climb|conventional|as-drawn (climb)",
                        ),
                        param(
                            "reverse",
                            ParamKind::Bool,
                            false,
                            "engrave, slot: each curve the other way",
                        ),
                        param(
                            "pattern",
                            ParamKind::String,
                            false,
                            "slot: bidirectional|directional (bidirectional)",
                        ),
                        param(
                            "side",
                            ParamKind::String,
                            false,
                            "chamfer: auto|flipped (auto: away from the material)",
                        ),
                        param(
                            "gcode",
                            ParamKind::String,
                            false,
                            "custom-gcode: its lines, newline between them, @name@ \
                             placeholders filled",
                        ),
                        param("name", ParamKind::String, false, "the operation's name"),
                        param(
                            "enabled",
                            ParamKind::Bool,
                            false,
                            "false makes it turned off (see enable)",
                        ),
                    ],
                    returns: "{feature}".into(),
                    notes: vec![
                        "Without a tool it takes the one the setup's last operation used.".into(),
                        "Engrave follows its sketches' loops and its picked edges, open or \
                         closed, passes.depth below the top (0.5 mm), ramping down along each, \
                         a stepdown at most each time with multiple depths. V-carve cuts the \
                         areas its sketches close down their middle with a V-bit, as deep as \
                         they are wide, no deeper than passes.max_depth = {value, given}."
                            .into(),
                        "Slot cuts along the same curves and edges as Engrave, as wide as its \
                         tool, passes.depth below the top (3 mm), a stepdown a level (1 mm): \
                         bidirectional ramps back and forth along each, directional cuts each \
                         level from its start, lifting back between them."
                            .into(),
                        "Chamfer (deburr) runs a chamfer mill or V-bit along picked edges, or \
                         a sketch's loops, each from its own height: passes.chamfer_width wide \
                         (0.5 mm), its tip passes.tip_offset (0.2 mm) below the chamfer, its \
                         centre beside the edge away from the material. A cone too small for \
                         it is a failed check."
                            .into(),
                        "A transform (its Pattern group) repeats the operation's toolpath before it is linked: in \
                         rows, then each round a centre, then all mirrored with the originals \
                         kept; a mirrored copy's rings are turned round so it still climbs. \
                         Adaptive and pocket clearing are worked out again for each copy, \
                         against the material where it is. Every copy is simulated and \
                         checked, and one outside the stock is a warning."
                            .into(),
                        "Corner relief (passes.relief) on a 2D profile, pocket or slot cuts out \
                         each inside corner the tool leaves round, to its sharp point, so a \
                         square part fits: a dogbone goes into the corner, halfway between \
                         its walls; a T-bone on along one wall into the other, the one \
                         nearer X or Y, or the longer or shorter. At every level, and in \
                         every copy of a pattern; it is no gouge."
                            .into(),
                        "A library tool's first preset sets its feeds and speeds.".into(),
                        "Custom G-code (strategy custom-gcode) has no tool and no toolpath: its \
                         gcode lines are written into the program where it stands, before the \
                         next operation's start, or after the last one's end. Placeholders such \
                         as @tool@, @rpm@ and @operation@ are the program's. A line that moves \
                         the machine or changes a mode (an axis word, any G code but G4, M6, \
                         M2, M30) is a failed check; a subprogram call a warning."
                            .into(),
                        "A probe (strategy probe, a touch probe tool) touches a grid over the \
                         stock's top, or its boundary, an inset in: probe_x_count/probe_y_count \
                         columns and rows, or as few as keep them probe_x_spacing/ \
                         probe_y_spacing apart (10 mm), each down from the retract height at \
                         probe_feed (50 mm/min) by G38.2, at most probe_depth (3 mm) below the \
                         top. LinuxCNC logs the points to a file named after the operation \
                         (PROBEOPEN); GRBL reports them over the serial line; other posts \
                         cannot probe. A setup's probe_map then corrects every cut to them."
                            .into(),
                        "Drilling: peck backs out between pecks, and with chip_break only that \
                         far (G73); pitch, mm a turn, taps each hole instead (hand = left for a \
                         left-hand thread), fed at the pitch times the spindle's speed: posted \
                         only for a machine that taps: LinuxCNC (G33.1), Fanuc (M29, G84, G74), \
                         Haas and Centroid CNC12 (G84, G74), Heidenhain (cycle 207), or a post \
                         of your own with tap = \"g84\", \"m29\" or \"g33.1\"; on Masso, GRBL \
                         and Mach it is a failed check."
                            .into(),
                        "Holes (drill, bore, circular, thread) are picked by a rim (edges) or a \
                         wall (faces); holes.same_diameter takes every hole of the setup's \
                         bodies as wide as one picked, a boss's every boss as wide. Bore goes \
                         down a helix, passes.bore_pitch a turn (1 mm), with finishing passes \
                         at the bottom. Circular clears holes from the middle out and bosses \
                         from passes.boss_width out, in circles a stepover apart, a stepdown a \
                         level. Thread mills a thread with a thread mill at \
                         passes.thread_pitch, passes.hand, in passes.thread_passes radial \
                         passes, to passes.thread_diameter (a hole's plus the thread's depth, \
                         a boss's own, unless given). Each helix is G2 or G3 with a Z."
                            .into(),
                    ],
                    see_also: vec![id(TOOLS), id(RUN)],
                    ..Default::default()
                },
                Command {
                    id: id(TOOLS),
                    summary: "The tool library".into(),
                    returns: "a list of {name, kind, diameter, number, presets}".into(),
                    read_only: true,
                    ..Default::default()
                },
                Command {
                    id: id(IMPORT_TOOLS),
                    summary: "Add a Fusion tool library, exported as CSV or JSON, to the tool \
                              library"
                        .into(),
                    params: vec![param(
                        "text",
                        ParamKind::String,
                        true,
                        "the CSV or JSON file's text",
                    )],
                    returns: "{said}: what was added, and what left out".into(),
                    see_also: vec![id(TOOLS)],
                    ..Default::default()
                },
                Command {
                    id: id(ADD_POST),
                    summary: "Keep a machine profile of one's own, in camcraft's TOML, \
                              beside the built-in posts"
                        .into(),
                    params: vec![param("text", ParamKind::String, true, "the profile's TOML")],
                    returns: "{name}: the post's name, for a setup's `post`".into(),
                    notes: vec![
                        "A profile that does not load is refused, saying which field and why: \
                         an unknown @placeholder@ names its block and line."
                            .into(),
                        "Besides camcraft's fields, it may give the spindle's range \
                         (spindle = [min, max]), the travel its programs must keep inside \
                         ([travel] x = [min, max], y, z, in the program's units: from the work \
                         zero, or with coordinates = \"machine\" in machine coordinates, \
                         where a setup's work_offset places them), the work offsets it has \
                         past G59 (extended_offsets = [\"G59.1\", ...]) and its own tool \
                         change (tool_change_lines, naming @tool@)."
                            .into(),
                    ],
                    ..Default::default()
                },
                Command {
                    id: id(RUN),
                    summary: "Run a setup again, or queue it behind the one running".into(),
                    params: id_only("the setup, or one of its operations"),
                    returns: "{state}".into(),
                    see_also: vec![id(STATUS)],
                    ..Default::default()
                },
                Command {
                    id: id(STATUS),
                    summary: "Where a setup's run stands: its checks and its operations".into(),
                    params: id_only("the setup, or one of its operations"),
                    returns: "{state = none|queued|running|stopped|failed|ready, error?, passed?, \
                              findings?, operations?, machining_seconds?, tool_changes?}"
                        .into(),
                    read_only: true,
                    see_also: vec![id(GCODE)],
                    ..Default::default()
                },
                Command {
                    id: id(GCODE),
                    summary: "An operation's G-code, or its setup's when the setup is posted \
                              as one program, once the setup has run and passed its checks"
                        .into(),
                    params: vec![
                        param(
                            "id",
                            ParamKind::Id,
                            true,
                            "the operation, or the setup (posted as one program)",
                        ),
                        param(
                            "force",
                            ParamKind::Bool,
                            false,
                            "give it even though a check failed",
                        ),
                    ],
                    returns: "the program's text".into(),
                    read_only: true,
                    see_also: vec![id(SETUP_SHEET)],
                    ..Default::default()
                },
                Command {
                    id: id(SETUP_SHEET),
                    summary: "A setup's sheet, for the shop floor, once it has run: the part, \
                              stock and material, work zero and offset, tool axis, post and \
                              program, each tool and operation, machining time, and the checks"
                        .into(),
                    params: id_only("the setup, or one of its operations"),
                    returns: "the sheet, a self-contained HTML page to print".into(),
                    notes: vec![
                        "A setup that failed its checks has a sheet too, which says so first."
                            .into(),
                        "Machining time is the moves at their feeds and the post's rapid \
                         rates ([rapid] in its profile, 5000 mm/min an axis unless it says), \
                         plus its tool_change_seconds a change: nothing accelerates, so a \
                         machine takes longer."
                            .into(),
                    ],
                    read_only: true,
                    see_also: vec![id(GCODE), id(STATUS)],
                    ..Default::default()
                },
                Command {
                    id: id(SAVE_TEMPLATE),
                    summary: "Keep a setup, or some of its operations, as a template to make \
                              again on other parts"
                        .into(),
                    params: vec![
                        param("name", ParamKind::String, true, "the template's name"),
                        param(
                            "setup",
                            ParamKind::Id,
                            false,
                            "the setup: its rules and every operation",
                        ),
                        param(
                            "operations",
                            ParamKind::List,
                            false,
                            "operation ids, all of one setup: those alone, without the \
                             setup's rules",
                        ),
                        param(
                            "whole",
                            ParamKind::Bool,
                            false,
                            "keep the setup's rules too (true for a setup, false for \
                             operations)",
                        ),
                        param("description", ParamKind::String, false, "a line about it"),
                        param(
                            "fill",
                            ParamKind::Any,
                            false,
                            "{<operation name> = nothing|pick|silhouette|{holes = mm}}: how \
                             each is filled on the next part; as it picks unless given",
                        ),
                        param(
                            "replace",
                            ParamKind::Bool,
                            false,
                            "replace a template of that name; refused otherwise",
                        ),
                    ],
                    returns: "{name, file, replaced, operations = [{name, fill}], text}".into(),
                    notes: vec![
                        "A template keeps the setup's stock, work zero, orientation, post, \
                         tolerance and programs, and each operation's strategy, tool, \
                         heights, linking, passes and pattern; not its bodies, what is \
                         picked on them, or its probe results. It is a JSON file in the \
                         package's data folder (templates/), which says its version."
                            .into(),
                        "An operation that cuts what is picked waits for a pick on the next \
                         part (pick), unless filled by a rule: silhouette makes its boundary \
                         or outline the part's silhouette; {holes = 6.8} takes every hole \
                         6.8 mm across along the tool. A hole operation taking every hole as \
                         wide as one picked keeps that hole's diameter as its rule."
                            .into(),
                    ],
                    see_also: vec![id(APPLY_TEMPLATE), id(TEMPLATES)],
                    ..Default::default()
                },
                Command {
                    id: id(APPLY_TEMPLATE),
                    summary: "Make a setup from a template on bodies, or add a template's \
                              operations to a setup"
                        .into(),
                    params: vec![
                        param(
                            "template",
                            ParamKind::String,
                            false,
                            "a template's name, built in or saved",
                        ),
                        param(
                            "text",
                            ParamKind::String,
                            false,
                            "a template file's text, instead of a name",
                        ),
                        param(
                            "bodies",
                            ParamKind::List,
                            false,
                            "body ids for a new setup; every shown body when neither this \
                             nor setup is given",
                        ),
                        param(
                            "setup",
                            ParamKind::Id,
                            false,
                            "a setup to add the operations to, after its own",
                        ),
                        param(
                            "rules",
                            ParamKind::Bool,
                            false,
                            "with setup: take the template's stock, work zero and post too \
                             (false)",
                        ),
                        param("name", ParamKind::String, false, "a new setup's name"),
                    ],
                    returns: "{setup, operations, tools = [{template, as, from = \
                              setup|document|library|template}], needs_pick}"
                        .into(),
                    notes: vec![
                        "Each tool is the setup's, another setup's or the library's with \
                         the same number, kind, diameter, corner and angle, else one of the \
                         same shape, else the template's own, added. An operation in \
                         needs_pick does not run until something is picked for it."
                            .into(),
                    ],
                    see_also: vec![id(SAVE_TEMPLATE), id(TEMPLATES), id(STATUS)],
                    ..Default::default()
                },
                Command {
                    id: id(TEMPLATES),
                    summary: "The templates, built in and saved".into(),
                    returns: "a list of {name, built_in, file?, description, setup, \
                              operations = [{name, strategy, tool, fill}]}"
                        .into(),
                    read_only: true,
                    see_also: vec![id(APPLY_TEMPLATE)],
                    ..Default::default()
                },
                Command {
                    id: id(DELETE_TEMPLATE),
                    summary: "Delete a saved template".into(),
                    params: vec![param("name", ParamKind::String, true, "its name")],
                    returns: "{deleted}".into(),
                    see_also: vec![id(TEMPLATES)],
                    ..Default::default()
                },
                Command {
                    id: id(ENABLE),
                    summary: "Turn a setup or an operation off, or on again".into(),
                    params: vec![
                        param("id", ParamKind::Id, true, "the setup or the operation"),
                        param("enabled", ParamKind::Bool, true, "false to turn it off"),
                    ],
                    returns: "{feature, enabled}".into(),
                    notes: vec![
                        "An operation turned off stays in its setup, but is not run, \
                         simulated, checked or posted: the operations after it cut the \
                         material as if it were not there. A setup turned off is not run \
                         at all. Either is hidden while off, and shown when turned on."
                            .into(),
                    ],
                    see_also: vec![id(OPERATION), id(RUN)],
                    ..Default::default()
                },
            ],
            length_keys: [
                "margin_side",
                "margin_top",
                "margin_below",
                "size_x",
                "size_y",
                "size_z",
                "margin_radial",
                "margin_ends",
                "diameter",
                "length",
                "offset_x",
                "offset_y",
                "offset_z",
                "wcs_x",
                "wcs_y",
                "wcs_z",
                "tolerance",
                "stepover",
                "stepdown",
                "radial",
                "axial",
                "margin",
            ]
            .map(String::from)
            .to_vec(),
            ..Default::default()
        }
    }

    fn feature_info(&self, node: &Node) -> FeatureInfo {
        let status = Some(self.status_of(node));
        if node.kind == SETUP_KIND {
            let on = SetupData::from(&node.data).is_none_or(|d| d.enabled);
            return FeatureInfo {
                icon: if on { "setup" } else { "setup-off" }.into(),
                kind_label: "CAM setup".into(),
                family_label: "CAM".into(),
                status,
                ..Default::default()
            };
        }
        let data = OperationData::from(&node.data);
        let label = data
            .as_ref()
            .map_or("CAM operation", |d| data::strategy_label(&d.strategy()));
        let on = data.is_none_or(|d| d.operation.enabled);
        FeatureInfo {
            icon: if on { "operation" } else { "operation-off" }.into(),
            kind_label: label.into(),
            family_label: "CAM".into(),
            status,
            ..Default::default()
        }
    }

    fn parameters(&self, node: &Node) -> Vec<Parameter> {
        let p = |key: &str, label: &str, dim: Dim| Parameter {
            key: format!("/{key}"),
            name: Some(key.into()),
            label: label.into(),
            dim,
            pointer: format!("/{key}"),
            ..Default::default()
        };
        if node.kind == SETUP_KIND {
            return vec![
                p("margin_side", "Stock margin, sides", Dim::Length),
                p("margin_top", "Stock margin, top", Dim::Length),
                p("margin_below", "Stock margin, below", Dim::Length),
                p("size_x", "Stock X", Dim::Length),
                p("size_y", "Stock Y", Dim::Length),
                p("size_z", "Stock Z", Dim::Length),
                p("margin_radial", "Stock margin, round", Dim::Length),
                p("margin_ends", "Stock margin, each end", Dim::Length),
                p("diameter", "Stock diameter", Dim::Length),
                p("length", "Stock length", Dim::Length),
                p("offset_x", "Work zero X offset", Dim::Length),
                p("offset_y", "Work zero Y offset", Dim::Length),
                p("offset_z", "Work zero Z offset", Dim::Length),
                p("tilt_x", "Tool axis tilt about X", Dim::Angle),
                p("tilt_y", "Tool axis tilt about Y", Dim::Angle),
                p("axis_dx", "Tool axis direction X", Dim::Number),
                p("axis_dy", "Tool axis direction Y", Dim::Number),
                p("axis_dz", "Tool axis direction Z", Dim::Number),
                p("wcs_x", "Work offset, machine X", Dim::Length),
                p("wcs_y", "Work offset, machine Y", Dim::Length),
                p("wcs_z", "Work offset, machine Z", Dim::Length),
                p("tolerance", "Tolerance", Dim::Length),
            ];
        }
        // Custom G-code has no numbers to bind.
        if OperationData::from(&node.data).is_some_and(|d| d.is_custom()) {
            return Vec::new();
        }
        let mut out = vec![
            p("stepover", "Stepover", Dim::Length),
            p("stepdown", "Stepdown", Dim::Length),
            p("feed", "Feed", Dim::Number),
            p("plunge", "Plunge feed", Dim::Number),
            p("rpm", "Spindle speed", Dim::Number),
            p("radial", "Stock to leave, walls", Dim::Length),
            p("axial", "Stock to leave, floors", Dim::Length),
            p("angle", "Pass angle", Dim::Angle),
            p("margin", "Boundary offset", Dim::Length),
        ];
        let data = OperationData::from(&node.data);
        if let Some(d) = &data {
            let numbers = d
                .linking
                .parameters()
                .into_iter()
                .chain(d.passes.parameters(&d.operation))
                .chain(d.transform.parameters());
            for (pointer, name, label, dim) in numbers {
                out.push(Parameter {
                    key: pointer.clone(),
                    name: Some(name.into()),
                    label: label.into(),
                    dim,
                    pointer,
                    ..Default::default()
                });
            }
        }
        // The heights, by their own names; a top or a bottom only while it is
        // given.
        let heights = data.map(|d| d.heights);
        for (key, label, given) in [
            ("clearance", "Clearance height", true),
            ("retract", "Retract height", true),
            (
                "top",
                "Top height",
                heights.is_some_and(|h| h.top_from.is_some()),
            ),
            (
                "bottom",
                "Bottom height",
                heights.is_some_and(|h| h.bottom_from.is_some()),
            ),
        ] {
            if given {
                out.push(Parameter {
                    key: format!("/heights/{key}"),
                    name: Some(format!("{key}_height")),
                    label: label.into(),
                    dim: Dim::Length,
                    pointer: format!("/heights/{key}"),
                    ..Default::default()
                });
            }
        }
        out
    }

    fn run_command(&mut self, command: &str, args: Value) -> Result<Value, String> {
        let text = |key: &str| args.get(key).and_then(Value::as_str).map(String::from);
        let number = |key: &str| args.get(key).and_then(Value::as_f64);
        match suffix(command).unwrap_or_default() {
            SETUP => {
                let bodies = match args.get("bodies") {
                    Some(list) => serde_json::from_value(list.clone())
                        .map_err(|_| "bodies is a list of body ids")?,
                    None => self.chosen_bodies(),
                };
                // Read before the setup is made, so a wrong one makes none.
                let mut stock_from = None;
                let stock = match args.get("stock") {
                    None => None,
                    Some(Value::Object(o)) => {
                        let from = o
                            .get("from")
                            .and_then(Value::as_str)
                            .ok_or("stock is a kind, or {from = \"previous\"} or a setup's name")?;
                        if from != "previous" {
                            let (n, _) = setups()
                                .into_iter()
                                .find(|(n, _)| n.name == from)
                                .ok_or_else(|| format!("no setup '{from}' to carry on from"))?;
                            stock_from = Some(n.id);
                        } else if setups().is_empty() {
                            return Err("no setup before it to carry on from".into());
                        }
                        Some(StockKind::Earlier)
                    }
                    Some(_) => match text("stock").as_deref() {
                        Some("box") => Some(StockKind::Around),
                        Some("fixed-box") => Some(StockKind::Fixed),
                        Some("cylinder") => Some(StockKind::Cylinder),
                        other => {
                            return Err(format!(
                                "no stock '{}': box, fixed-box, cylinder or {{from = ...}}",
                                other.unwrap_or_default()
                            ));
                        }
                    },
                };
                let size: Option<[f64; 3]> = match args.get("size") {
                    Some(v) => Some(
                        serde_json::from_value(v.clone()).map_err(|_| "size is {x, y, z} in mm")?,
                    ),
                    None => None,
                };
                let axis = match text("axis").as_deref() {
                    None | Some("tool") => CylinderAxis::Tool,
                    Some("x") => CylinderAxis::X,
                    Some("y") => CylinderAxis::Y,
                    Some(other) => return Err(format!("no axis '{other}': tool, x or y")),
                };
                let axis_named = |key: &str| -> Result<Option<Axis>, String> {
                    match text(key) {
                        None => Ok(None),
                        Some(name) => Axis::ALL
                            .into_iter()
                            .find(|a| a.label().replace('−', "-").eq_ignore_ascii_case(&name))
                            .map(Some)
                            .ok_or_else(|| format!("no axis '{name}': +x, -x, +y, -y, +z or -z")),
                    }
                };
                // The document axis it points up, or a direction.
                let (up_axis, up_along) = match args.get("tool_axis") {
                    Some(Value::String(_)) => (axis_named("tool_axis")?, None),
                    Some(v) => {
                        let d: Direction3 = serde_json::from_value(v.clone())
                            .map_err(|_| "tool_axis is +z or the like, or {x, y, z}")?;
                        (None, Some([d.x, d.y, d.z]))
                    }
                    None => (None, None),
                };
                let tilt: Option<Tilt2> = match args.get("tilt") {
                    Some(v) => Some(
                        serde_json::from_value(v.clone())
                            .map_err(|_| "tilt is {x, y} in degrees")?,
                    ),
                    None => None,
                };
                let machine_x = axis_named("machine_x")?;
                let work_offset: Option<Direction3> = match args.get("work_offset") {
                    Some(v) => Some(
                        serde_json::from_value(v.clone())
                            .map_err(|_| "work_offset is {x, y, z} in mm")?,
                    ),
                    None => None,
                };
                // G55, or 55, among those the post it will have has.
                let wcs = match args.get("wcs") {
                    None => None,
                    Some(given) => {
                        let w = match given {
                            Value::String(w) => w.trim().to_ascii_uppercase(),
                            v => format!("G{}", v.as_f64().ok_or("wcs is G54 or the like")?),
                        };
                        let mut post = SetupData::new(
                            Vec::new(),
                            &text("post").unwrap_or_else(|| self.settings.post.clone()),
                            0.0,
                        );
                        post.post_profile = self.settings.posts.get(&post.post).cloned();
                        let offsets = post.work_offsets();
                        let Some(&(n, _)) = offsets.iter().find(|(_, o)| *o == w) else {
                            let all: Vec<&str> = offsets.iter().map(|(_, o)| o.as_str()).collect();
                            return Err(format!(
                                "no work offset {w} on {}: {}",
                                post.post,
                                all.join(", ")
                            ));
                        };
                        Some(f64::from(n))
                    }
                };
                let programs = match text("programs").as_deref() {
                    None | Some("setup") => camcraft_core::job::Programs::Setup,
                    Some("operation") => camcraft_core::job::Programs::Operation,
                    Some(other) => {
                        return Err(format!("no programs '{other}': setup or operation"));
                    }
                };
                let probe: Option<ProbeMapArgs> = match args.get("probe_map") {
                    Some(v) => Some(serde_json::from_value(v.clone()).map_err(
                        |_| "probe_map is {path = a file} or {text = its contents}, with segment",
                    )?),
                    None => None,
                };
                // Read before the setup is made, so results that are no map
                // make none.
                let mut probed = SetupData::new(Vec::new(), "", 0.0);
                if let Some(ProbeMapArgs {
                    text: Some(t),
                    path,
                    ..
                }) = &probe
                {
                    probed
                        .load_probe_results(path.as_deref().unwrap_or("the script's"), t)
                        .map_err(|e| format!("probe_map: {e}"))?;
                }
                let id = self.make_setup(bodies, text("name").as_deref())?;
                if let Some((_, mut data)) = setup_node(&id) {
                    if let Some(p) = &probe {
                        data.probe_correct = true;
                        data.probe_points = probed.probe_points;
                        data.probe_file = p.path.clone().unwrap_or(probed.probe_file);
                        if let Some(s) = p.segment {
                            data.probe_segment = s;
                        }
                    }
                    if let Some(z) = up_axis {
                        data.z_axis = z;
                        data.square_axes();
                    }
                    if let Some([x, y, z]) = up_along {
                        data.tool_axis = data::ToolAxis::Direction;
                        (data.axis_dx, data.axis_dy, data.axis_dz) = (x, y, z);
                    }
                    if let Some(t) = tilt {
                        data.tool_axis = data::ToolAxis::Tilt;
                        (data.tilt_x, data.tilt_y) = (t.x, t.y);
                    }
                    if let Some(x) = machine_x {
                        data.x_axis = x;
                        if data.tool_axis == data::ToolAxis::Square {
                            data.square_axes();
                        }
                    }
                    if let Some(m) = number("margin") {
                        data.margin_side = m;
                        data.margin_top = m;
                        data.margin_radial = m;
                        data.margin_ends = m;
                    }
                    if let Some(kind) = stock {
                        data.stock = kind;
                        data.stock_from = stock_from;
                    }
                    if let Some([x, y, z]) = size {
                        (data.size_x, data.size_y, data.size_z) = (x, y, z);
                    }
                    data.cylinder_axis = axis;
                    if let Some(d) = number("diameter") {
                        data.diameter = d;
                        data.cylinder_fixed = true;
                    }
                    if let Some(l) = number("length") {
                        data.length = l;
                        data.cylinder_fixed = true;
                    }
                    if let Some(post) = text("post") {
                        data.post_profile = self.settings.posts.get(&post).cloned();
                        data.post = post;
                    }
                    if let Some(t) = number("tolerance") {
                        data.tolerance = t;
                    }
                    if let Some(n) = wcs {
                        data.wcs = n;
                    }
                    if let Some(at) = work_offset {
                        data.wcs_known = true;
                        (data.wcs_x, data.wcs_y, data.wcs_z) = (at.x, at.y, at.z);
                    }
                    data.programs = programs;
                    if let Some(n) = number("number") {
                        data.program_number = n;
                    }
                    if let Some(m) = text("material") {
                        data.material = m;
                    }
                    host::set_feature_data(&id, data.to_value())?;
                }
                Ok(json!({ "feature": id }))
            }
            OPERATION => {
                let setup = text("setup").ok_or("which setup?")?;
                let (_, mut setup_data) = setup_node(&setup).ok_or("no such setup")?;
                let name = text("strategy").ok_or("which strategy?")?;
                let strategy =
                    StrategySpec::named(&name).ok_or_else(|| format!("no strategy '{name}'"))?;
                let id = self.make_operation(&setup, strategy, text("name").as_deref())?;
                let (_, mut data) = operation_node(&id).ok_or("the operation was not made")?;
                if let Some(tool) = text("tool") {
                    let spec = setup_data
                        .tools
                        .get(&tool)
                        .or_else(|| self.settings.tools.get(&tool))
                        .cloned()
                        .ok_or_else(|| format!("no tool '{tool}'"))?;
                    let name = setup_data.adopt(&tool, &spec);
                    host::set_feature_data(&setup, setup_data.to_value())?;
                    let preset = spec.presets.first().cloned();
                    data.use_tool(&name, &spec, preset.as_ref(), setup_data.tolerance);
                }
                for (key, field) in [
                    ("stepover", &mut data.stepover),
                    ("feed", &mut data.feed),
                    ("plunge", &mut data.plunge),
                ] {
                    if let Some(v) = number(key) {
                        *field = v;
                    }
                }
                if let Some(v) = number("stepdown") {
                    data.stepdown = v;
                    data.multiple_depths = true;
                }
                // Over what it has: `heights = {clearance = 15, top_from = "model-top"}`.
                if let Some(given) = args.get("heights") {
                    data.heights =
                        over(&data.heights, given).map_err(|e| format!("heights: {e}"))?;
                }
                if let Some(given) = args.get("passes") {
                    data.passes = over(&data.passes, given).map_err(|e| format!("passes: {e}"))?;
                }
                if let Some(given) = args.get("transform") {
                    data.transform =
                        over(&data.transform, given).map_err(|e| format!("transform: {e}"))?;
                }
                if let Some(given) = args.get("linking") {
                    data.linking =
                        over(&data.linking, given).map_err(|e| format!("linking: {e}"))?;
                }
                if let Some(given) = args.get("sketches") {
                    let ids: Vec<String> = serde_json::from_value(given.clone())
                        .map_err(|_| "sketches: a list of sketch ids")?;
                    if let Some(bad) = ids.iter().find(|s| host::profile(s).is_none()) {
                        return Err(format!("sketches: '{bad}' is no sketch that closes a loop"));
                    }
                    data.take_sketches(ids);
                }
                // `boundary = "selection"`, or `{kind = "selection", containment =
                // "inside", margin = 1}`.
                if let Some(given) = args.get("boundary") {
                    let given = match given {
                        Value::String(kind) => json!({ "kind": kind }),
                        other => other.clone(),
                    };
                    let mut given = given
                        .as_object()
                        .cloned()
                        .ok_or("boundary: a kind, or a table of kind, containment and margin")?;
                    // None given is none: not the channel a new operation
                    // gets round the part, which would shrink any other
                    // boundary by the tool's width.
                    data.margin = match given.remove("margin") {
                        Some(m) => m.as_f64().ok_or("boundary: margin, a number")?,
                        None => 0.0,
                    };
                    data.operation.boundary = over(&data.operation.boundary, &Value::Object(given))
                        .map_err(|e| format!("boundary: {e}"))?;
                }
                if let StrategySpec::Engrave {
                    direction, reverse, ..
                } = &mut data.operation.strategy
                {
                    if let Some(d) = text("direction") {
                        *direction = serde_json::from_value(json!(d))
                            .map_err(|_| format!("no direction '{d}'"))?;
                    }
                    if let Some(r) = args.get("reverse").and_then(Value::as_bool) {
                        *reverse = r;
                    }
                }
                for (key, picks) in [("faces", &mut data.faces), ("edges", &mut data.edges)] {
                    if let Some(given) = args.get(key) {
                        *picks = serde_json::from_value(given.clone())
                            .map_err(|_| format!("{key}: a list of {{body, at = {{x, y, z}}}}"))?;
                    }
                }
                if let Some(given) = args.get("holes") {
                    data.operation.holes = serde_json::from_value(given.clone()).map_err(|_| {
                        "holes: {same_diameter = true|false, order = nearest|x-then-y|as-picked}"
                    })?;
                }
                if let Some(on) = args.get("conventional").and_then(Value::as_bool) {
                    match &mut data.operation.strategy {
                        StrategySpec::Profile { conventional, .. }
                        | StrategySpec::Pocket { conventional }
                        | StrategySpec::PocketClearing { conventional, .. }
                        | StrategySpec::Bore { conventional, .. }
                        | StrategySpec::Circular { conventional, .. }
                        | StrategySpec::Thread { conventional, .. } => *conventional = on,
                        _ => return Err("conventional: this strategy has no such choice".into()),
                    }
                }
                if let StrategySpec::Thread { spring_pass, .. } = &mut data.operation.strategy
                    && let Some(on) = args.get("spring_pass").and_then(Value::as_bool)
                {
                    *spring_pass = on;
                }
                if let StrategySpec::Chamfer { side, .. } = &mut data.operation.strategy
                    && let Some(d) = text("side")
                {
                    *side = serde_json::from_value(json!(d))
                        .map_err(|_| format!("no chamfer side '{d}'"))?;
                }
                if data.is_custom()
                    && let Some(text) = text("gcode")
                {
                    data.operation.gcode = text;
                }
                if let StrategySpec::Slot {
                    pattern, reverse, ..
                } = &mut data.operation.strategy
                {
                    if let Some(p) = text("pattern") {
                        *pattern = serde_json::from_value(json!(p))
                            .map_err(|_| format!("no slot pattern '{p}'"))?;
                    }
                    if let Some(r) = args.get("reverse").and_then(Value::as_bool) {
                        *reverse = r;
                    }
                }
                host::set_feature_data(&id, data.to_value())?;
                if args.get("enabled").and_then(Value::as_bool) == Some(false) {
                    set_enabled(&id, false)?;
                }
                self.refresh(&setup, false);
                Ok(json!({ "feature": id }))
            }
            IMPORT_TOOLS => {
                let text = text("text").ok_or("the CSV's or JSON's text, as `text`")?;
                let said = tools::import_tools(&mut self.settings.tools, &text)?;
                Ok(json!({ "said": said }))
            }
            ADD_POST => {
                let text = text("text").ok_or("the profile's TOML, as `text`")?;
                let name = tools::add_post(&mut self.settings.posts, &text)?;
                Ok(json!({ "name": name }))
            }
            TOOLS => Ok(Value::Array(
                self.settings
                    .tools
                    .iter()
                    .map(|(name, t)| {
                        json!({
                            "name": name,
                            "kind": format!("{:?}", t.kind).to_lowercase(),
                            "diameter": t.diameter,
                            "number": t.number,
                            "presets": t.presets.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
                        })
                    })
                    .collect(),
            )),
            RUN => {
                let id = text("id").unwrap_or_default();
                let setup = self
                    .setup_of(&id)
                    .ok_or("no setup or operation by that id")?;
                self.refresh(&setup, true);
                Ok(self.status(&setup))
            }
            STATUS => {
                let id = text("id").unwrap_or_default();
                let setup = self
                    .setup_of(&id)
                    .ok_or("no setup or operation by that id")?;
                Ok(self.status(&setup))
            }
            ENABLE => {
                let id = text("id").unwrap_or_default();
                let on = args
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .ok_or("enabled: true or false")?;
                set_enabled(&id, on)?;
                self.refresh_for(&id, false);
                Ok(json!({ "feature": id, "enabled": on }))
            }
            GCODE => {
                let id = text("id").unwrap_or_default();
                let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
                Ok(Value::String(self.program_of(&id, force)?.text))
            }
            SETUP_SHEET => {
                let id = text("id").unwrap_or_default();
                Ok(Value::String(self.sheet_of(&id)?.1))
            }
            SAVE_TEMPLATE => {
                self.load_templates_now();
                let name = text("name").ok_or("the template's name, as `name`")?;
                let ops: Vec<String> = match args.get("operations") {
                    Some(v) => serde_json::from_value(v.clone())
                        .map_err(|_| "operations is a list of operation ids")?,
                    None => Vec::new(),
                };
                let setup = match (text("setup"), ops.first()) {
                    (Some(s), _) => s,
                    (None, Some(op)) => operation_node(op)
                        .and_then(|(_, d)| d.setup)
                        .ok_or_else(|| format!("no operation '{op}' in a setup"))?,
                    (None, None) => return Err("give setup, or operations".into()),
                };
                if let Some(bad) = ops.iter().find(|o| {
                    operation_node(o).and_then(|(_, d)| d.setup).as_deref() != Some(&setup)
                }) {
                    return Err(format!("'{bad}' is no operation of that setup"));
                }
                let whole = args
                    .get("whole")
                    .and_then(Value::as_bool)
                    .unwrap_or(ops.is_empty());
                let mut fills = BTreeMap::new();
                if let Some(given) = args.get("fill") {
                    let given = given
                        .as_object()
                        .ok_or("fill: {<operation name> = a rule}")?;
                    for (op, rule) in given {
                        fills.insert(
                            op.clone(),
                            templates::Fill::named(rule).map_err(|e| format!("fill.{op}: {e}"))?,
                        );
                    }
                }
                let t = self.template_of(
                    &name,
                    &text("description").unwrap_or_default(),
                    &setup,
                    &ops,
                    whole,
                    &fills,
                )?;
                if let Some(unknown) = fills
                    .keys()
                    .find(|n| !t.operations.iter().any(|o| &o.name == *n))
                {
                    return Err(format!("fill: no operation '{unknown}' kept"));
                }
                let replace = args
                    .get("replace")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let (file, replaced) = self.keep_template(&t, replace)?;
                Ok(json!({
                    "name": t.name,
                    "file": file,
                    "replaced": replaced,
                    "operations": t.operations.iter().map(|o| json!({
                        "name": o.name,
                        "fill": o.fill,
                    })).collect::<Vec<_>>(),
                    "text": templates::text_of(&t),
                }))
            }
            APPLY_TEMPLATE => {
                self.load_templates_now();
                let t = match (text("template"), text("text")) {
                    (Some(name), _) => self.template_named(&name)?,
                    (None, Some(t)) => templates::parse(&t).map_err(|e| format!("text: {e}"))?,
                    (None, None) => return Err("give template, a name, or text".into()),
                };
                let target = match text("setup") {
                    Some(id) => {
                        setup_node(&id).ok_or("no such setup")?;
                        templates::Target::Setup {
                            id,
                            rules: args.get("rules").and_then(Value::as_bool).unwrap_or(false),
                        }
                    }
                    None => templates::Target::New {
                        bodies: match args.get("bodies") {
                            Some(list) => serde_json::from_value(list.clone())
                                .map_err(|_| "bodies is a list of body ids")?,
                            None => self.chosen_bodies(),
                        },
                        name: text("name"),
                    },
                };
                Ok(self.apply_template(&t, target)?.value())
            }
            TEMPLATES => Ok(Value::Array({
                self.load_templates_now();
                self.templates()
                    .0
                    .iter()
                    .map(|k| {
                        json!({
                            "name": k.template.name,
                            "built_in": k.file.is_none(),
                            "file": k.file,
                            "description": k.template.description,
                            "setup": k.template.setup.is_some(),
                            "operations": k.template.operations.iter().map(|o| json!({
                                "name": o.name,
                                "strategy": strategy_word(&o.data.strategy()),
                                "tool": o.data.operation.tool,
                                "fill": o.fill,
                            })).collect::<Vec<_>>(),
                        })
                    })
                    .collect()
            })),
            DELETE_TEMPLATE => {
                self.load_templates_now();
                let name = text("name").ok_or("the template's name, as `name`")?;
                let all = self.templates();
                let kept = all
                    .0
                    .iter()
                    .find(|k| k.template.name == name)
                    .ok_or_else(|| format!("no template '{name}'"))?;
                let file = kept.file.as_deref().ok_or("a built-in template stays")?;
                self.drop_template(file);
                Ok(json!({ "deleted": name }))
            }
            _ => Err(format!("no command `{command}`")),
        }
    }

    fn input(&mut self, input: &Input) -> bool {
        if self.armed.is_some()
            && let Some(taken) = self.pick_input(input)
        {
            return taken;
        }
        match &input.event {
            Event::ToolActivated => {
                match input.tool.as_deref().and_then(suffix) {
                    Some(NEW_SETUP) => match self.make_setup(self.chosen_bodies(), None) {
                        Ok(id) => {
                            host::request(Request::JournalLabel {
                                label: "New setup".into(),
                            });
                            self.open(&id, true);
                        }
                        Err(e) => host::error(&format!("No setup: {e}")),
                    },
                    Some(NEW_OPERATION) => {
                        let setup = match self.current_setup() {
                            Some(s) => Ok(s),
                            None => self.make_setup(self.chosen_bodies(), None),
                        };
                        let strategy = STRATEGIES
                            .get(self.new_strategy)
                            .and_then(|(n, _)| StrategySpec::named(n));
                        let made = setup.and_then(|s| {
                            self.make_operation(&s, strategy.ok_or("no strategy")?, None)
                        });
                        match made {
                            Ok(id) => {
                                host::request(Request::JournalLabel {
                                    label: "New operation".into(),
                                });
                                self.open(&id, true);
                            }
                            Err(e) => host::error(&format!("No operation: {e}")),
                        }
                    }
                    Some(GENERATE) => self.refresh_all(true),
                    // A setup to apply a template to, its Templates group
                    // first and open.
                    Some(FROM_TEMPLATE) => match self.make_setup(self.chosen_bodies(), None) {
                        Ok(id) => {
                            host::request(Request::JournalLabel {
                                label: "Setup from template".into(),
                            });
                            self.open(&id, true);
                            self.template_ui.first = true;
                        }
                        Err(e) => host::error(&format!("No setup: {e}")),
                    },
                    _ => return false,
                }
                true
            }
            Event::EditFeature { feature } => {
                self.open(feature, false);
                true
            }
            Event::Activated => {
                self.refresh_later(None);
                true
            }
            Event::ValuesMoved { features } => {
                let moved = reading(|| features.iter().filter_map(|f| self.setup_of(f)).collect());
                self.refresh_later(Some(moved));
                true
            }
            Event::FeatureMoved {
                feature,
                target,
                before,
            } => self.move_operation(feature, target, *before),
            Event::FileOpened { id, files } if id == templates::OPEN_TEMPLATE => {
                self.hint = Some(templates::imported(self, files));
                host::redraw();
                true
            }
            Event::FileOpened { id, files } => {
                if let Some(setup) = id.strip_prefix(tools::OPEN_PROBE) {
                    let Some(f) = files.first() else {
                        return false;
                    };
                    let said = match (std::str::from_utf8(&f.contents), setup_node(setup)) {
                        (Ok(text), Some((_, mut data))) => {
                            match data.load_probe_results(&f.name, text) {
                                Ok(said) => {
                                    self.write_setup(setup, &data);
                                    said
                                }
                                Err(e) => format!("{}: {e}", f.name),
                            }
                        }
                        (Err(_), _) => format!("{} is not text.", f.name),
                        (_, None) => "The setup is gone.".into(),
                    };
                    self.hint = Some(said);
                    host::redraw();
                    return true;
                }
                let opened = tools::files_opened(
                    id,
                    files,
                    &mut self.settings.tools,
                    &mut self.settings.posts,
                );
                let Some(said) = opened else {
                    return false;
                };
                self.settings_note = Some(said);
                // The preferences page shows what came of it.
                host::redraw();
                true
            }
            Event::JobFinished { job, .. }
                if self.later.as_ref().is_some_and(|(j, _)| j == job) =>
            {
                let setups = self.later.take().and_then(|(_, s)| s);
                reading(|| match setups {
                    None => self.refresh_all(false),
                    Some(setups) => {
                        for setup in setups {
                            self.refresh(&setup, false);
                        }
                    }
                });
                true
            }
            Event::JobFinished { job, result } if self.template_job_finished(*job, result) => {
                host::redraw();
                true
            }
            Event::JobFinished { job, result } => {
                host::redraw();
                if let Some(setup) = self.runs.finished(*job, result) {
                    self.find_lines(&setup);
                    match self.runs.get(&setup).map(|c| &c.state) {
                        Some(State::Failed(e)) => host::warn(&format!("Setup not run: {e}")),
                        Some(State::Ready(run)) if !run.passed => {
                            host::warn("A setup failed its checks: open it to read them.")
                        }
                        _ => {}
                    }
                }
                true
            }
            Event::Key { key, down } if key == "Shift" => {
                self.shift = *down;
                false
            }
            Event::Key { key, down: true }
                if let Some(command) = playback::key_command(key, self.shift, self.big_step())
                    && let Some(setup) = self.played_setup() =>
            {
                let Some(run) = self.runs.get(&setup).and_then(|c| c.shown_rc()) else {
                    return false;
                };
                if run.playback.is_none() {
                    return false;
                }
                let steps = self.steps(&setup, &run);
                playback::apply(
                    &mut self.playing,
                    &mut self.transport,
                    &setup,
                    &run,
                    command,
                    &steps,
                );
                true
            }
            Event::Key { key, down: true } if key == "Escape" && self.editing.is_some() => {
                self.task_close(false);
                true
            }
            _ => false,
        }
    }

    fn frame(&mut self, pointer: &Pointer) -> Frame {
        reading(|| self.draw_frame(pointer))
    }

    fn panel_event(&mut self, slot: PanelSlot, event: PanelEvent) {
        if slot == PanelSlot::Settings {
            panel::settings_event(self, event);
            return;
        }
        if slot == PanelSlot::Transport {
            if let Some(setup) = self.played_setup()
                && let Some(run) = self.runs.get(&setup).and_then(|c| c.shown_rc())
            {
                let steps = self.steps(&setup, &run);
                playback::transport_event(
                    &mut self.playing,
                    &mut self.transport,
                    &setup,
                    &run,
                    &event,
                    &steps,
                );
            }
            return;
        }
        if self.listing_event(&event) {
            return;
        }
        let Some(editing) = self.editing.clone() else {
            return;
        };
        self.hint = None;
        match editing.kind {
            Kind::Setup => panel::setup_event(self, &editing.id, event),
            Kind::Operation => panel::operation_event(self, &editing.id, event),
        }
    }

    fn task_close(&mut self, accept: bool) -> Option<String> {
        let editing = self.editing.take()?;
        self.hint = None;
        if accept {
            self.refresh_for(&editing.id, false);
            return Some(match editing.kind {
                Kind::Setup => "Edit setup".into(),
                Kind::Operation => "Edit operation".into(),
            });
        }
        let setup = self.setup_of(&editing.id);
        let result = if editing.fresh {
            if editing.kind == Kind::Setup {
                self.runs.forget(&editing.id);
            }
            host::remove_feature(&editing.id)
        } else {
            host::set_feature_data(&editing.id, editing.opened)
        };
        if let Err(e) = result {
            host::error(&e);
        }
        if let Some(setup) = setup {
            self.refresh(&setup, false);
        }
        None
    }

    fn delete_feature(&mut self, id: &str) -> bool {
        if setup_node(id).is_some() {
            self.runs.forget(id);
        } else if let Some((_, op)) = operation_node(id)
            && let Some(setup) = op.setup
        {
            // The setup runs again without it once it is gone; until then the
            // run it had is shown, out of date.
            self.runs.stop(&setup);
        }
        false
    }

    fn menu_items(&mut self, scope: &MenuScope) -> Vec<MenuItem> {
        let item = |suffix: &str, label: &str, icon: &str| MenuItem {
            id: id(suffix),
            label: label.into(),
            icon: Some(icon.into()),
            enabled: true,
            ..Default::default()
        };
        // Off, it is turned on again; on, off.
        let toggle = |f: &str, what: &str, icon: &str| match runs(f) {
            Some(false) => MenuItem {
                hint: Some(format!("Run the {what} again, and post it")),
                ..item(TURN_ON, "Turn on", icon)
            },
            _ => MenuItem {
                hint: Some(format!(
                    "Keep the {what}, but leave it out: not run, simulated, checked or posted"
                )),
                ..item(TURN_OFF, "Turn off", &format!("{icon}-off"))
            },
        };
        reading(|| match scope {
            MenuScope::TreeFeature(f) if setup_node(f).is_some() => vec![
                item(EDIT, "Edit setup", "setup"),
                item(ADD_OPERATION, "Add operation", "operation"),
                item(GENERATE, "Generate", "generate"),
                toggle(f, "setup", "setup"),
                MenuItem {
                    hint: Some(
                        "Keep its stock, work zero, post and operations, not its picks, to \
                         make again on another part"
                            .into(),
                    ),
                    separator_before: true,
                    ..item(SAVE_AS_TEMPLATE, "Save as template", "template")
                },
            ],
            MenuScope::TreeFeature(f) if operation_node(f).is_some() => vec![
                item(EDIT, "Edit operation", "operation"),
                item(GENERATE, "Generate its setup", "generate"),
                toggle(f, "operation", "operation"),
                MenuItem {
                    hint: Some("Keep it, not its picks, to add to another setup".into()),
                    separator_before: true,
                    ..item(SAVE_AS_TEMPLATE, "Save as template", "template")
                },
            ],
            _ => Vec::new(),
        })
    }

    fn menu_command(&mut self, command: &str, scope: &MenuScope) -> bool {
        let MenuScope::TreeFeature(feature) = scope else {
            return false;
        };
        match suffix(command) {
            Some(EDIT) => self.open(feature, false),
            Some(GENERATE) => self.refresh_for(feature, true),
            Some(s @ (TURN_OFF | TURN_ON)) => {
                let on = s == TURN_ON;
                match set_enabled(feature, on) {
                    Ok(()) => {
                        host::request(Request::JournalLabel {
                            label: if on { "Turn on" } else { "Turn off" }.into(),
                        });
                        self.refresh_for(feature, false);
                    }
                    Err(e) => host::error(&e),
                }
            }
            Some(SAVE_AS_TEMPLATE) => match self.save_from_tree(feature) {
                Ok(said) => host::info(&said),
                Err(e) => host::error(&format!("Not saved as a template: {e}")),
            },
            Some(ADD_OPERATION) => {
                let strategy = STRATEGIES
                    .get(self.new_strategy)
                    .and_then(|(n, _)| StrategySpec::named(n));
                match strategy.map(|s| self.make_operation(feature, s, None)) {
                    Some(Ok(id)) => self.open(&id, true),
                    Some(Err(e)) => host::error(&e),
                    None => {}
                }
            }
            _ => return false,
        }
        true
    }

    fn settings_panel(&mut self) -> Vec<Widget> {
        panel::settings_panel(self)
    }

    fn settings(&self) -> Option<Value> {
        serde_json::to_value(&self.settings).ok()
    }

    fn apply_settings(&mut self, settings: Value) {
        if let Ok(settings) = serde_json::from_value(settings) {
            self.settings = settings;
        }
    }

    fn job(entry: &str, input: &str) -> Result<String, String> {
        if entry == LATER {
            return Ok(String::new());
        }
        if entry == templates::JOB {
            let work = serde_json::from_str(input).map_err(|e| e.to_string())?;
            return templates::work(std::path::Path::new(templates::DIR), &work);
        }
        if entry != JOB {
            return Err(format!("no job `{entry}`"));
        }
        let out = host::helper(HELPER, input.as_bytes())?;
        String::from_utf8(out).map_err(|_| "the helper's answer is not text".into())
    }
}

impl Cam {
    /// The frame ([`Bench::frame`]), the document read once for it.
    fn draw_frame(&mut self, pointer: &Pointer) -> Frame {
        self.pointer = pointer.clone();
        if let Some(e) = &self.editing
            && feature(&e.id).is_none()
        {
            self.editing = None;
        }
        // Playback is the open setup's, or the open operation's setup's.
        let open = self.played_setup();
        if self.playing.as_ref().map(|p| &p.setup) != open.as_ref() {
            self.playing = None;
        }
        let run = open
            .as_ref()
            .and_then(|s| self.runs.get(s))
            .and_then(|c| c.shown_rc())
            .filter(|r| r.playback.is_some());
        if let (Some(p), Some(run)) = (&mut self.playing, &run) {
            p.tick(run, &self.transport);
        }
        self.keep_session(open.as_deref());
        self.take_aim(open.as_deref());
        // A slot is armed only while its feature is open.
        if self
            .armed
            .as_ref()
            .is_some_and(|a| self.editing.as_ref().is_none_or(|e| e.id != a.feature))
        {
            self.armed = None;
        }
        let mut frame = Frame::default();
        draw::draw(self, &mut frame);
        if let Some(armed) = &self.armed {
            picking::draw(&mut frame, armed, [0.31, 0.64, 0.9]);
        }
        if let Some(run) = &run {
            let exact = self
                .session
                .as_ref()
                .and_then(|s| s.at.as_ref())
                .map(|a| (a.op, a.moves, a.of));
            frame.transport =
                playback::transport_widgets(run, self.playing.as_ref(), &self.transport, exact);
            // Playing, or waiting on the session's answer to a seek.
            let waiting = self
                .session
                .as_ref()
                .is_some_and(|s| s.failed.is_none() && self.playing.is_some());
            frame.animating = self.playing.as_ref().is_some_and(|p| p.running.is_some()) || waiting;
        }
        let Some(editing) = self.editing.clone() else {
            return frame;
        };
        let Some(node) = feature(&editing.id) else {
            return frame;
        };
        let (title, icon, prompt) = match editing.kind {
            Kind::Setup => (
                "Setup",
                "setup",
                "Pick the bodies, then set the stock and the work zero",
            ),
            Kind::Operation => ("Operation", "operation", "Set the strategy and the tool"),
        };
        frame.hud.tool = Some(ToolHint {
            icon: icon.into(),
            name: node.name.clone(),
            prompt: prompt.into(),
            keys: vec![("Esc".into(), "cancel".into())],
            ..Default::default()
        });
        frame.editing = Some(editing.id.clone());
        frame.task = Some(Task {
            title: title.into(),
            icon: icon.into(),
            confirmable: true,
            ..Default::default()
        });
        frame.panel = match editing.kind {
            Kind::Setup => panel::setup_panel(self, &node),
            Kind::Operation => panel::operation_panel(self, &node),
        };
        frame
    }

    /// The setup `id` is, or belongs to.
    fn setup_of(&self, id: &str) -> Option<String> {
        if setup_node(id).is_some() {
            return Some(id.to_string());
        }
        operation_node(id)?.1.setup
    }

    fn status(&self, setup: &str) -> Value {
        if runs(setup) == Some(false) {
            return json!({"state": "off"});
        }
        let Some(c) = self.runs.get(setup) else {
            return json!({"state": "none"});
        };
        match &c.state {
            State::Queued => json!({"state": "queued"}),
            State::Running(_) => json!({"state": "running"}),
            State::Stopped => json!({"state": "stopped"}),
            State::Failed(e) => json!({"state": "failed", "error": e}),
            State::Ready(run) => json!({
                "state": "ready",
                "passed": run.passed,
                "seconds": run.seconds,
                "findings": run.findings.iter()
                    .filter(|f| f.level != cam_protocol::Level::Pass)
                    .map(|f| f.text.clone())
                    .collect::<Vec<_>>(),
                "operations": run.operations.iter().map(|o| json!({
                    "name": o.name,
                    "made": o.made,
                    "summary": o.summary,
                    "cut_mm": o.cut_mm,
                    "machining_seconds": o.seconds,
                    "lines": o.program.as_ref().map(|p| p.lines),
                })).collect::<Vec<_>>(),
                "lines": run.program.as_ref().map(|p| p.lines),
                "machining_seconds": run.machining.total(),
                "tool_changes": run.machining.tool_changes,
            }),
        }
    }
}

bench!(Cam);

/// A direction a script gives, `{x, y, z}`.
#[derive(serde::Deserialize)]
struct Direction3 {
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
    #[serde(default)]
    z: f64,
}

/// The probe results a script corrects a setup's depths from: a file the
/// helper reads each run, or its text, and how long a corrected piece is.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeMapArgs {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    segment: Option<f64>,
}

/// Tilts a script gives, `{x, y}` in degrees.
#[derive(serde::Deserialize)]
struct Tilt2 {
    #[serde(default)]
    x: f64,
    #[serde(default)]
    y: f64,
}

/// `now` with the fields of `given`, an object, over it: what a script command
/// sets of a group of numbers.
fn over<T: serde::Serialize + serde::de::DeserializeOwned>(
    now: &T,
    given: &Value,
) -> Result<T, String> {
    let given = given.as_object().ok_or("a table of its fields")?;
    let mut value = serde_json::to_value(now).map_err(|e| e.to_string())?;
    if let Some(fields) = value.as_object_mut() {
        fields.extend(given.clone());
    }
    serde_json::from_value(value).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_id_starts_with_the_package() {
        assert_eq!(suffix(SETUP_KIND), Some(SETUP));
        assert_eq!(suffix(OPERATION_KIND), Some(OPERATION));
        let registration = Cam::default().describe();
        for tool in &registration.tools {
            assert!(suffix(&tool.id).is_some(), "{}", tool.id);
        }
        for command in &registration.commands {
            assert!(suffix(&command.id).is_some(), "{}", command.id);
            for other in &command.see_also {
                assert!(
                    registration.commands.iter().any(|c| &c.id == other),
                    "{other}"
                );
            }
        }
        let manifest = include_str!("../bench.toml");
        assert!(manifest.contains(&format!("id = \"{PACKAGE}\"")));
        assert!(manifest.contains(&format!("\"{SETUP_KIND}\"")));
        assert!(manifest.contains(&format!("\"{OPERATION_KIND}\"")));
    }

    #[test]
    fn the_library_starts_as_camcraft_s() {
        let settings = Settings::default();
        assert_eq!(settings.tools.len(), 20);
        let back: Settings =
            serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
        assert_eq!(back, settings);
    }

    #[test]
    fn every_strategy_is_camcraft_s() {
        for (name, _) in STRATEGIES {
            assert!(StrategySpec::named(name).is_some(), "{name}");
        }
    }

    #[test]
    fn a_run_shows_in_the_tree_as_it_went() {
        use cam_protocol::{Finding, OperationRun, Run};
        let finding = |level, operation| Finding {
            level,
            code: "CC1".into(),
            text: format!("{level:?} at {operation:?}"),
            operation,
            at_move: None,
        };
        let op = |made: &str| OperationRun {
            made: made.into(),
            summary: "2 passes".into(),
            ..OperationRun::default()
        };
        let mut run = Run {
            passed: false,
            findings: vec![finding(Level::Fail, Some(1)), finding(Level::Warn, Some(0))],
            operations: vec![op("generated"), op("generated"), op("disabled")],
            ..Run::default()
        };
        assert_eq!(run_status(&run, None).state, FeatureState::Failed);
        assert_eq!(run_status(&run, Some(0)).state, FeatureState::Warnings);
        assert_eq!(run_status(&run, Some(1)).state, FeatureState::Failed);
        assert_eq!(run_status(&run, Some(2)).state, FeatureState::NotRun);
        run.findings.clear();
        run.passed = true;
        let done = run_status(&run, Some(0));
        assert_eq!(done.state, FeatureState::Done);
        assert_eq!(done.note.as_deref(), Some("2 passes"));
        assert_eq!(run_status(&run, None).state, FeatureState::Done);
    }

    #[test]
    fn each_operation_shows_its_own_checks() {
        use cam_protocol::{Finding, OperationRun, Run};
        // A tap the post refuses fails its own check, and only its own.
        let run = Run {
            passed: false,
            findings: vec![Finding {
                level: Level::Fail,
                code: "post".into(),
                text: "post GRBL Tap: tapping is posted only as a cycle".into(),
                operation: Some(1),
                at_move: None,
            }],
            operations: vec![
                OperationRun {
                    made: "generated".into(),
                    summary: "4 holes".into(),
                    ..OperationRun::default()
                },
                OperationRun {
                    made: "generated".into(),
                    ..OperationRun::default()
                },
            ],
            ..Run::default()
        };
        let computed = Computed::ready(run, vec!["chip".into(), "tap".into()]);
        let setup = computed_status(&computed, None);
        assert_eq!(setup.state, FeatureState::Failed);
        assert!(setup.note.unwrap().contains("Tap"));
        let chip = computed_status(&computed, Some("chip"));
        assert_eq!(chip.state, FeatureState::Done);
        assert_eq!(chip.note.as_deref(), Some("4 holes"));
        let tap = computed_status(&computed, Some("tap"));
        assert_eq!(tap.state, FeatureState::Failed);
        // One added since the run is not the setup's failure.
        let new = computed_status(&computed, Some("new"));
        assert_eq!(new.state, FeatureState::NotRun);
    }
}
