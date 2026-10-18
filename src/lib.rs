//! CAM: a workbench package for 3-axis CNC milling, after camcraft.
//!
//! A setup and its operations are features of the package's own kinds that
//! build no solid. A setup names the bodies it machines, its stock, how the
//! part sits and where its work zero is, and its post; each operation names
//! its setup, its strategy (adaptive or pocket clearing, the finishing
//! strategies, 2D profiles and pockets, facing, drilling) and its tool.
//!
//! None of the machining happens here. A setup runs as a job that hands it, as
//! a camcraft job with each body's B-rep, to the package's native helper
//! (`helper/`), which runs camcraft's engine on every core: the toolpaths, the
//! simulation against the material, the checks, and a G-code program for each
//! operation. The package draws what comes back, shows the checks, and saves
//! the programs through the save dialog, refusing one camcraft refuses.

// Every literal of the contract's structs ends in `..Default::default()`,
// even one naming every field, so a field a later 0.1.x adds leaves the
// package building.
#![allow(clippy::needless_update)]

mod data;
mod draw;
mod panel;
mod runs;

use std::collections::BTreeMap;

use cam_protocol::{HELPER, IDENTITY, Request as HelperRequest, Shape, VERSION};
use camcraft_core::job::{StrategySpec, ToolSpec};
use printcad_bench_sdk::api::*;
use printcad_bench_sdk::{Bench, Value, bench, host, json, serde_json};

use data::{OperationData, STRATEGIES, SetupData, models_of, strategy_word};
use runs::{JOB, Runs, State};

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

const SETUP: &str = "setup";
const OPERATION: &str = "operation";
const TOOLS: &str = "tools";
const RUN: &str = "run";
const STATUS: &str = "status";
const GCODE: &str = "gcode";

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
    /// A word for the task panel about the last thing tried.
    hint: Option<String>,
}

/// A feature of the document, if it is a setup.
fn setup_node(id: &str) -> Option<(Node, SetupData)> {
    let node = host::feature(id).filter(|n| n.kind == SETUP_KIND)?;
    let data = SetupData::from(&node.data)?;
    Some((node, data))
}

/// A feature of the document, if it is an operation.
fn operation_node(id: &str) -> Option<(Node, OperationData)> {
    let node = host::feature(id).filter(|n| n.kind == OPERATION_KIND)?;
    let data = OperationData::from(&node.data)?;
    Some((node, data))
}

fn setups() -> Vec<(Node, SetupData)> {
    host::features()
        .into_iter()
        .filter(|n| n.kind == SETUP_KIND)
        .filter_map(|n| {
            let data = SetupData::from(&n.data)?;
            Some((n, data))
        })
        .collect()
}

/// Every operation of setup `setup`, in the document's order.
fn operations_of(setup: &str) -> Vec<(Node, OperationData)> {
    host::features()
        .into_iter()
        .filter(|n| n.kind == OPERATION_KIND)
        .filter_map(|n| {
            let data = OperationData::from(&n.data)?;
            (data.setup.as_deref() == Some(setup)).then_some((n, data))
        })
        .collect()
}

/// The request that runs setup `id`, or why it cannot run.
fn request_for(node: &Node, setup: &SetupData) -> Result<String, String> {
    if let Some(problem) = setup.problem() {
        return Err(problem);
    }
    let bodies = host::bodies();
    let models = models_of(setup, |b| {
        bodies.iter().find(|x| x.id == b).map(|x| x.name.clone())
    });
    let operations = operations_of(&node.id);
    if operations.is_empty() {
        return Err("The setup has no operations yet: add one.".into());
    }
    for (op, data) in &operations {
        if data.operation.enabled
            && let Some(problem) = data.problem(&setup.tools)
        {
            return Err(format!("{}: {problem}", op.name));
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
    let request = HelperRequest {
        version: VERSION,
        job: camcraft_session::job_text(&job).map_err(|e| e.to_string())?,
        shapes,
        max_points: DRAWN_POINTS,
    };
    serde_json::to_string(&request).map_err(|e| e.to_string())
}

impl Cam {
    /// Run setup `id` unless what it is run from is unchanged; `again` runs a
    /// stopped or failed one again too.
    fn refresh(&mut self, id: &str, again: bool) {
        let Some((node, setup)) = setup_node(id) else {
            self.runs.forget(id);
            return;
        };
        match request_for(&node, &setup) {
            Ok(request) => self.runs.want(id, request, again),
            Err(why) => self.runs.refuse(id, why),
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
        let data = SetupData::new(bodies, &self.settings.post, self.settings.tolerance);
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
        let last = ops.last().and_then(|(_, d)| d.operation.tool.clone());
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
        self.refresh_for(id, false);
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

    /// The program of operation `op`, when its setup has run.
    fn program_of(&self, op: &str, force: bool) -> Result<cam_protocol::Program, String> {
        let (node, data) = operation_node(op).ok_or_else(|| format!("no operation {op}"))?;
        let setup = data.setup.ok_or("The operation is in no setup.")?;
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
                            "mm of stock round the bodies and on top; 2",
                        ),
                        param(
                            "post",
                            ParamKind::String,
                            false,
                            "grbl, masso, masso-inch, centroid-avid or centroid-avid-inch",
                        ),
                        param(
                            "tolerance",
                            ParamKind::Number,
                            false,
                            "mm; the preferences' (0.05)",
                        ),
                        param("name", ParamKind::String, false, "the setup's name"),
                    ],
                    returns: "{feature}".into(),
                    notes: vec![
                        "The work zero is the stock's top, front-left corner.".into(),
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
                        param("name", ParamKind::String, false, "the operation's name"),
                    ],
                    returns: "{feature}".into(),
                    notes: vec![
                        "Without a tool it takes the one the setup's last operation used.".into(),
                        "A library tool's first preset sets its feeds and speeds.".into(),
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
                              findings?, operations?}"
                        .into(),
                    read_only: true,
                    see_also: vec![id(GCODE)],
                    ..Default::default()
                },
                Command {
                    id: id(GCODE),
                    summary: "An operation's G-code, once its setup has run and passed its checks"
                        .into(),
                    params: vec![
                        param("id", ParamKind::Id, true, "the operation"),
                        param(
                            "force",
                            ParamKind::Bool,
                            false,
                            "give it even though a check failed",
                        ),
                    ],
                    returns: "the program's text".into(),
                    read_only: true,
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
                "offset_x",
                "offset_y",
                "offset_z",
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
        if node.kind == SETUP_KIND {
            return FeatureInfo {
                icon: "setup".into(),
                kind_label: "CAM setup".into(),
                family_label: "CAM".into(),
                ..Default::default()
            };
        }
        let label = OperationData::from(&node.data)
            .map_or("CAM operation", |d| data::strategy_label(&d.strategy()));
        FeatureInfo {
            icon: "operation".into(),
            kind_label: label.into(),
            family_label: "CAM".into(),
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
                p("offset_x", "Work zero X offset", Dim::Length),
                p("offset_y", "Work zero Y offset", Dim::Length),
                p("offset_z", "Work zero Z offset", Dim::Length),
                p("tolerance", "Tolerance", Dim::Length),
            ];
        }
        vec![
            p("stepover", "Stepover", Dim::Length),
            p("stepdown", "Stepdown", Dim::Length),
            p("feed", "Feed", Dim::Number),
            p("plunge", "Plunge feed", Dim::Number),
            p("rpm", "Spindle speed", Dim::Number),
            p("radial", "Stock to leave, walls", Dim::Length),
            p("axial", "Stock to leave, floors", Dim::Length),
            p("angle", "Pass angle", Dim::Angle),
            p("margin", "Boundary offset", Dim::Length),
        ]
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
                let id = self.make_setup(bodies, text("name").as_deref())?;
                if let Some((_, mut data)) = setup_node(&id) {
                    if let Some(m) = number("margin") {
                        data.margin_side = m;
                        data.margin_top = m;
                    }
                    if let Some(post) = text("post") {
                        data.post = post;
                    }
                    if let Some(t) = number("tolerance") {
                        data.tolerance = t;
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
                host::set_feature_data(&id, data.to_value())?;
                self.refresh(&setup, false);
                Ok(json!({ "feature": id }))
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
            GCODE => {
                let id = text("id").unwrap_or_default();
                let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
                Ok(Value::String(self.program_of(&id, force)?.text))
            }
            _ => Err(format!("no command `{command}`")),
        }
    }

    fn input(&mut self, input: &Input) -> bool {
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
                    _ => return false,
                }
                true
            }
            Event::EditFeature { feature } => {
                self.open(feature, false);
                true
            }
            Event::Activated => {
                self.refresh_all(false);
                true
            }
            Event::ValuesMoved { features } => {
                for id in features {
                    self.refresh_for(id, false);
                }
                true
            }
            Event::JobFinished { job, result } => {
                if let Some(setup) = self.runs.finished(*job, result) {
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
            Event::Key { key, down: true } if key == "Escape" && self.editing.is_some() => {
                self.task_close(false);
                true
            }
            _ => false,
        }
    }

    fn frame(&mut self, pointer: &Pointer) -> Frame {
        self.pointer = pointer.clone();
        if let Some(e) = &self.editing
            && host::feature(&e.id).is_none()
        {
            self.editing = None;
        }
        let mut frame = Frame::default();
        draw::draw(self, &mut frame);
        let Some(editing) = self.editing.clone() else {
            return frame;
        };
        let Some(node) = host::feature(&editing.id) else {
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

    fn panel_event(&mut self, slot: PanelSlot, event: PanelEvent) {
        if slot == PanelSlot::Settings {
            panel::settings_event(self, event);
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
        match scope {
            MenuScope::TreeFeature(f) if setup_node(f).is_some() => vec![
                item(EDIT, "Edit setup", "setup"),
                item(ADD_OPERATION, "Add operation", "operation"),
                item(GENERATE, "Generate", "generate"),
            ],
            MenuScope::TreeFeature(f) if operation_node(f).is_some() => vec![
                item(EDIT, "Edit operation", "operation"),
                item(GENERATE, "Generate its setup", "generate"),
            ],
            _ => Vec::new(),
        }
    }

    fn menu_command(&mut self, command: &str, scope: &MenuScope) -> bool {
        let MenuScope::TreeFeature(feature) = scope else {
            return false;
        };
        match suffix(command) {
            Some(EDIT) => self.open(feature, false),
            Some(GENERATE) => self.refresh_for(feature, true),
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
        if entry != JOB {
            return Err(format!("no job `{entry}`"));
        }
        let out = host::helper(HELPER, input.as_bytes())?;
        String::from_utf8(out).map_err(|_| "the helper's answer is not text".into())
    }
}

impl Cam {
    /// The setup `id` is, or belongs to.
    fn setup_of(&self, id: &str) -> Option<String> {
        if setup_node(id).is_some() {
            return Some(id.to_string());
        }
        operation_node(id)?.1.setup
    }

    fn status(&self, setup: &str) -> Value {
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
                    "lines": o.program.as_ref().map(|p| p.lines),
                })).collect::<Vec<_>>(),
            }),
        }
    }
}

bench!(Cam);

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
}
