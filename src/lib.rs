//! CAM: a workbench package that mills pockets. A pocket is an operation,
//! a feature of its own kind that builds no solid: the outline it clears
//! (a sketch's closed loops, or loops typed into a command), the stock's
//! top and the depth below it, and the tool and how it cuts. Its toolpath
//! is worked out by a job away from the window, drawn over the model, and
//! written as G-code through the save dialog. Tools come from a table in
//! the bench's Preferences page; an operation keeps the numbers of the
//! tool it was given, so formulas drive them and the file never depends
//! on the machine it is opened on.

// Every literal of the contract's structs ends in `..Default::default()`,
// even one naming every field, so a field a later 0.1.x adds leaves the
// package building.
#![allow(clippy::needless_update)]

mod gcode;
mod path;

use std::collections::BTreeMap;

use printcad_bench_sdk::api::*;
use printcad_bench_sdk::{Bench, Value, bench, host, json, serde_json};

use path::{P, Toolpath};

/// The package's id: its feature kind, tools, commands and menu entries
/// all start with it.
const PACKAGE: &str = "io.github.gilbertorconde.cam";
const KIND: &str = "io.github.gilbertorconde.cam.pocket";
const NEW: &str = "new";
const POCKET: &str = "pocket";
const TOOLPATH: &str = "toolpath";
const GCODE: &str = "gcode";
const EDIT: &str = "edit";
const JOB: &str = "pocket";

/// The package's id for `suffix`.
fn id(suffix: &str) -> String {
    format!("{PACKAGE}.{suffix}")
}

/// The suffix of one of the package's ids.
fn suffix(id: &str) -> Option<&str> {
    id.strip_prefix(PACKAGE)?.strip_prefix('.')
}

/// The most points the view draws of one toolpath; a denser one draws its
/// outline only.
const DRAWN_POINTS: usize = 20_000;

/// The most rows a command works out on its own, within its time; a
/// larger pocket is worked out by the workbench's job.
const COMMAND_ROWS: f64 = 20_000.0;

const ACCENT: Rgb = [0.31, 0.64, 0.9];
const MUTED: Rgb = [0.55, 0.58, 0.62];
const OUTLINE: Rgb = [0.9, 0.62, 0.25];

/// An operation's data.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Pocket {
    /// The sketch whose closed loops it clears.
    #[serde(default)]
    sketch: Option<String>,
    /// Loops in world X and Y, when no sketch gives them.
    #[serde(default)]
    outline: Vec<Vec<P>>,
    /// The stock's top, world Z.
    top: f64,
    /// How far below `top` the floor is.
    depth: f64,
    /// The deepest one pass cuts.
    step_down: f64,
    /// The tool's name, as the tool table had it.
    #[serde(default)]
    tool: String,
    tool_diameter: f64,
    /// How far apart the rows are.
    stepover: f64,
    /// mm/min along the rows.
    feed: f64,
    /// mm/min going down.
    plunge: f64,
    /// Revolutions per minute.
    spindle: f64,
    /// How far over `top` the tool travels between cuts.
    clearance: f64,
}

impl Pocket {
    fn from(data: &Value) -> Option<Pocket> {
        serde_json::from_value(data.clone()).ok()
    }

    fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    fn problem(&self) -> Option<String> {
        if self.tool_diameter <= 0.0 {
            return Some("The tool's diameter must be more than 0.".into());
        }
        if self.stepover <= 0.0 || self.stepover > self.tool_diameter {
            return Some(
                "The stepover must be more than 0 and at most the tool's diameter.".into(),
            );
        }
        if self.depth <= 0.0 || self.step_down <= 0.0 {
            return Some("The depth and the depth per pass must be more than 0.".into());
        }
        if self.feed <= 0.0 || self.plunge <= 0.0 {
            return Some("The feed and plunge rates must be more than 0.".into());
        }
        if self.clearance <= 0.0 {
            return Some("The clearance must be more than 0.".into());
        }
        None
    }

    /// The loops it clears, from its sketch or as typed.
    fn loops(&self) -> Result<Vec<Vec<P>>, String> {
        match &self.sketch {
            Some(sketch) => outline_of(sketch).map(|(loops, _)| loops),
            None if self.outline.iter().any(|l| l.len() >= 3) => Ok(self.outline.clone()),
            None => Err("Pick the sketch whose outline the pocket clears.".into()),
        }
    }

    /// What its toolpath is worked out from.
    fn input(&self) -> Result<path::Input, String> {
        if let Some(problem) = self.problem() {
            return Err(problem);
        }
        Ok(path::Input {
            loops: self.loops()?,
            radius: self.tool_diameter / 2.0,
            stepover: self.stepover,
            top: self.top,
            depth: self.depth,
            step_down: self.step_down,
        })
    }
}

/// A sketch's closed loops in world X and Y, and the height it lies at.
fn outline_of(sketch: &str) -> Result<(Vec<Vec<P>>, f64), String> {
    let profile =
        host::profile(sketch).ok_or("The sketch has no closed outline to clear.".to_string())?;
    path::loops_of(&profile)
}

/// A cutter of the tool table.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Cutter {
    name: String,
    diameter: f64,
    feed: f64,
    plunge: f64,
    spindle: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Settings {
    tools: Vec<Cutter>,
    clearance: f64,
}

impl Default for Settings {
    fn default() -> Self {
        let tool = |name: &str, diameter, feed, plunge, spindle| Cutter {
            name: name.into(),
            diameter,
            feed,
            plunge,
            spindle,
        };
        Self {
            tools: vec![
                tool("Flat 6 mm", 6.0, 800.0, 300.0, 12000.0),
                tool("Flat 3 mm", 3.0, 600.0, 200.0, 16000.0),
                tool("Flat 1 mm", 1.0, 300.0, 100.0, 20000.0),
            ],
            clearance: 5.0,
        }
    }
}

/// Where an operation's toolpath stands.
#[derive(Debug, Clone)]
enum State {
    Running(u64),
    Ready(Toolpath),
    Stopped,
    Failed(String),
}

#[derive(Debug, Clone)]
struct Computed {
    /// What it was worked out from, as sent to the job.
    input: String,
    state: State,
}

#[derive(Default)]
struct Cam {
    /// The operation whose task is open, and its data when it opened.
    editing: Option<(String, Pocket)>,
    /// The open operation is one the tool just made: Cancel removes it.
    fresh: bool,
    paths: BTreeMap<String, Computed>,
    /// What was selected when the frame was last drawn.
    pointer: Pointer,
    settings: Settings,
    /// The settings page's selected tool.
    chosen_tool: Option<usize>,
    /// A word for the task panel about the last thing tried.
    hint: Option<String>,
}

impl Cam {
    fn new_pocket(&self, sketch: Option<String>) -> Pocket {
        let tool = self.settings.tools.first().cloned().unwrap_or(Cutter {
            name: "Flat 6 mm".into(),
            diameter: 6.0,
            feed: 800.0,
            plunge: 300.0,
            spindle: 12000.0,
        });
        let top = sketch
            .as_deref()
            .and_then(|s| outline_of(s).ok())
            .map_or(0.0, |(_, z)| z);
        Pocket {
            sketch,
            outline: Vec::new(),
            top,
            depth: 3.0,
            step_down: 1.0,
            tool: tool.name,
            tool_diameter: tool.diameter,
            stepover: (tool.diameter * 0.4 * 1000.0).round() / 1000.0,
            feed: tool.feed,
            plunge: tool.plunge,
            spindle: tool.spindle,
            clearance: self.settings.clearance,
        }
    }

    /// Add `pocket` as an operation of the document; its id.
    fn make(&self, pocket: &Pocket, name: Option<&str>) -> Result<String, String> {
        let count = host::features().iter().filter(|n| n.kind == KIND).count();
        let name = name
            .map(str::to_string)
            .unwrap_or_else(|| format!("Pocket {}", count + 1));
        host::add_feature(KIND, &name, None, pocket.to_value())
    }

    fn open(&mut self, id: &str) {
        if let Some(node) = host::feature(id)
            && node.kind == KIND
            && let Some(pocket) = Pocket::from(&node.data)
        {
            self.refresh(id, &pocket, false);
            self.editing = Some((id.to_string(), pocket));
            self.fresh = false;
            self.hint = None;
        }
    }

    fn edited(&self) -> Option<(String, String, Pocket)> {
        let (id, _) = self.editing.as_ref()?;
        let node = host::feature(id)?;
        Some((id.clone(), node.name, Pocket::from(&node.data)?))
    }

    /// Start working out an operation's toolpath unless the one it has,
    /// or the one being worked out, is for the same input; `again` starts
    /// one over a stopped or failed one too.
    fn refresh(&mut self, id: &str, pocket: &Pocket, again: bool) {
        let input = match pocket.input() {
            Ok(input) => serde_json::to_string(&input).unwrap_or_default(),
            Err(e) => {
                self.stop(id);
                self.paths.insert(
                    id.to_string(),
                    Computed {
                        input: String::new(),
                        state: State::Failed(e),
                    },
                );
                return;
            }
        };
        if let Some(known) = self.paths.get(id)
            && known.input == input
            && (!again || matches!(known.state, State::Running(_) | State::Ready(_)))
        {
            return;
        }
        self.stop(id);
        let state = match host::start_job(JOB, &input) {
            Ok(job) => State::Running(job),
            Err(e) => State::Failed(e),
        };
        self.paths.insert(id.to_string(), Computed { input, state });
    }

    /// Stop the job working out an operation's toolpath, if one is.
    fn stop(&mut self, id: &str) {
        if let Some(Computed {
            state: State::Running(job),
            ..
        }) = self.paths.get(id)
        {
            host::cancel_job(*job);
        }
    }

    /// An operation's toolpath now: the one it has when it is for its
    /// current input, otherwise worked out here (a small pocket's is
    /// quick; a large one waits for the job).
    fn toolpath_now(&mut self, id: &str, pocket: &Pocket) -> Result<Toolpath, String> {
        let input = pocket.input()?;
        let text = serde_json::to_string(&input).unwrap_or_default();
        if let Some(Computed {
            input: known,
            state,
        }) = self.paths.get(id)
            && *known == text
        {
            match state {
                State::Ready(path) => return Ok(path.clone()),
                State::Running(_) => {
                    return Err("Its toolpath is being worked out; ask again when it is.".into());
                }
                _ => {}
            }
        }
        let (low, high) = input
            .loops
            .iter()
            .flatten()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), p| {
                (l.min(p[1]), h.max(p[1]))
            });
        if (high - low) / input.stepover > COMMAND_ROWS {
            return Err(
                "The pocket is too large to work out in a command: open it in the CAM workbench."
                    .into(),
            );
        }
        let path = path::plan(&input, |_, _| true)?;
        self.paths.insert(
            id.to_string(),
            Computed {
                input: text,
                state: State::Ready(path.clone()),
            },
        );
        Ok(path)
    }

    fn operation(id: &str) -> Result<(String, Pocket), String> {
        let node = host::feature(id)
            .filter(|n| n.kind == KIND)
            .ok_or_else(|| format!("no pocket operation {id}"))?;
        let pocket = Pocket::from(&node.data).ok_or("the operation's data does not read")?;
        Ok((node.name, pocket))
    }

    fn write(&mut self, id: &str, pocket: &Pocket) {
        match host::set_feature_data(id, pocket.to_value()) {
            Ok(()) => self.refresh(id, pocket, false),
            Err(e) => host::error(&e),
        }
    }

    fn finished(&mut self, job: u64, result: &Result<String, String>) {
        let Some((id, known)) = self
            .paths
            .iter_mut()
            .find(|(_, c)| matches!(c.state, State::Running(j) if j == job))
        else {
            return;
        };
        known.state = match result {
            Ok(json) => match serde_json::from_str(json) {
                Ok(path) => State::Ready(path),
                Err(e) => State::Failed(e.to_string()),
            },
            Err(e) if e == "stopped" => {
                host::warn(&format!("The toolpath of {id} was stopped."));
                State::Stopped
            }
            Err(e) => {
                host::warn(&format!("No toolpath for {id}: {e}"));
                State::Failed(e.clone())
            }
        };
    }

    fn task_panel(&self, id: &str, name: &str, pocket: &Pocket) -> Vec<Widget> {
        let bind = |key: &str| {
            Some(Bind {
                feature: id.to_string(),
                key: format!("/{key}"),
                ..Default::default()
            })
        };
        let number =
            |key: &str, label: &str, value: f64, dim: Dim, decimals: usize| Widget::Number {
                id: key.into(),
                label: label.into(),
                value,
                dim,
                bind: bind(key),
                min: None,
                max: None,
                decimals,
                error: None,
            };
        let operations: Vec<Node> = host::features()
            .into_iter()
            .filter(|n| n.kind == KIND)
            .collect();
        let items = operations
            .iter()
            .map(|n| ListItem {
                label: n.name.clone(),
                detail: Pocket::from(&n.data)
                    .map(|p| format!("Ø{} · {} mm", p.tool_diameter, p.depth)),
                icon: Some("pocket".into()),
                ..Default::default()
            })
            .collect();
        let sketch_name = pocket
            .sketch
            .as_deref()
            .and_then(host::feature)
            .map(|n| n.name);
        let outline = match (&pocket.sketch, sketch_name) {
            (Some(_), Some(name)) => Some(name),
            (Some(_), None) => Some("A sketch no longer there".into()),
            (None, _) if !pocket.outline.is_empty() => {
                Some(format!("{} typed loops", pocket.outline.len()))
            }
            (None, _) => None,
        };
        let tool_rows = self
            .settings
            .tools
            .iter()
            .map(|t| vec![t.name.clone(), format!("{}", t.diameter)])
            .collect();
        let mut panel = vec![
            Widget::Group {
                title: "Operations".into(),
                open: true,
                children: vec![Widget::List {
                    id: "operations".into(),
                    items,
                    selected: operations.iter().position(|n| n.id == id),
                }],
            },
            Widget::Heading {
                text: name.to_string(),
            },
            Widget::Group {
                title: "Outline".into(),
                open: true,
                children: vec![
                    Widget::Pick {
                        id: "sketch".into(),
                        label: "Sketch".into(),
                        value: outline,
                        armed: false,
                    },
                    Widget::Pick {
                        id: "top_face".into(),
                        label: "Top from face".into(),
                        value: Some(format!("Z {:.2}", pocket.top)),
                        armed: false,
                    },
                    number("top", "Stock top", pocket.top, Dim::Length, 2),
                    number("depth", "Final depth", pocket.depth, Dim::Length, 2),
                ],
            },
            Widget::Group {
                title: "Tool".into(),
                open: true,
                children: vec![
                    Widget::Table {
                        id: "tools".into(),
                        columns: vec!["Tool".into(), "Ø mm".into()],
                        rows: tool_rows,
                        selected: self
                            .settings
                            .tools
                            .iter()
                            .position(|t| t.name == pocket.tool),
                        editable: Vec::new(),
                    },
                    number(
                        "tool_diameter",
                        "Diameter",
                        pocket.tool_diameter,
                        Dim::Length,
                        3,
                    ),
                    number("stepover", "Stepover", pocket.stepover, Dim::Length, 3),
                    number(
                        "step_down",
                        "Depth per pass",
                        pocket.step_down,
                        Dim::Length,
                        3,
                    ),
                ],
            },
            Widget::Group {
                title: "Feeds and speeds".into(),
                open: true,
                children: vec![
                    number("feed", "Feed, mm/min", pocket.feed, Dim::Number, 0),
                    number("plunge", "Plunge, mm/min", pocket.plunge, Dim::Number, 0),
                    number("spindle", "Spindle, rpm", pocket.spindle, Dim::Number, 0),
                    number("clearance", "Clearance", pocket.clearance, Dim::Length, 2),
                ],
            },
        ];
        if let Some(hint) = &self.hint {
            panel.push(Widget::Note {
                kind: NoteKind::Info,
                title: None,
                text: hint.clone(),
            });
        }
        panel.push(Widget::Separator);
        let button = |id: &str, label: &str, style: ButtonStyle| Widget::Button {
            id: id.into(),
            label: label.into(),
            style,
            enabled: true,
        };
        match self.paths.get(id).map(|c| &c.state) {
            Some(State::Running(job)) => {
                panel.push(Widget::Progress {
                    label: "Working out the toolpath".into(),
                    fraction: None,
                    job: Some(*job),
                });
                panel.push(button("stop", "Stop", ButtonStyle::Secondary));
            }
            Some(State::Ready(path)) => {
                panel.push(Widget::Text {
                    text: format!(
                        "{} passes, {} runs, {:.0} mm cut",
                        path.levels.len(),
                        path.runs.len(),
                        path.cut_length()
                    ),
                    mono: true,
                });
                panel.push(button("save", "Save G-code…", ButtonStyle::Primary));
            }
            Some(State::Stopped) => {
                panel.push(Widget::Note {
                    kind: NoteKind::Warning,
                    title: None,
                    text: "The toolpath was stopped before it was done.".into(),
                });
                panel.push(button("compute", "Work it out", ButtonStyle::Secondary));
            }
            Some(State::Failed(e)) => {
                panel.push(Widget::Note {
                    kind: NoteKind::Error,
                    title: None,
                    text: e.clone(),
                });
                panel.push(button("compute", "Try again", ButtonStyle::Secondary));
            }
            None => panel.push(button("compute", "Work it out", ButtonStyle::Secondary)),
        }
        panel
    }

    fn save(&mut self, id: &str, name: &str, pocket: &Pocket) {
        let Some(Computed {
            state: State::Ready(path),
            ..
        }) = self.paths.get(id)
        else {
            return;
        };
        let text = gcode::write(name, pocket, path);
        let file: String = name
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect();
        host::request(Request::SaveFile {
            name: format!("{}.nc", file.to_lowercase()),
            kind: "G-code".into(),
            extension: "nc".into(),
            contents: text.into_bytes(),
        });
    }

    fn settings_event(&mut self, event: PanelEvent) {
        let tools = &mut self.settings.tools;
        match event {
            PanelEvent::Cell {
                id,
                row,
                column,
                value,
            } if id == "tools" && row < tools.len() => {
                let tool = &mut tools[row];
                let number = value.trim().replace(',', ".").parse::<f64>().ok();
                match (column, number) {
                    (0, _) if !value.trim().is_empty() => tool.name = value.trim().to_string(),
                    (1, Some(v)) if v > 0.0 => tool.diameter = v,
                    (2, Some(v)) if v > 0.0 => tool.feed = v,
                    (3, Some(v)) if v > 0.0 => tool.plunge = v,
                    (4, Some(v)) if v >= 0.0 => tool.spindle = v,
                    _ => {}
                }
            }
            PanelEvent::Select { id, index } if id == "tools" => self.chosen_tool = Some(index),
            PanelEvent::Button { id } if id == "add_tool" => {
                let mut tool = tools.last().cloned().unwrap_or(Cutter {
                    name: String::new(),
                    diameter: 6.0,
                    feed: 800.0,
                    plunge: 300.0,
                    spindle: 12000.0,
                });
                tool.name = format!("Tool {}", tools.len() + 1);
                tools.push(tool);
                self.chosen_tool = Some(tools.len() - 1);
            }
            PanelEvent::Button { id } if id == "remove_tool" => {
                if let Some(i) = self.chosen_tool.take()
                    && i < tools.len()
                {
                    tools.remove(i);
                }
            }
            PanelEvent::Number { id, value } if id == "clearance" && value > 0.0 => {
                self.settings.clearance = value
            }
            _ => {}
        }
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
        let id_only = || vec![param("id", ParamKind::Id, true, "the pocket operation")];
        Registration {
            label: "CAM".into(),
            description: "Pocket toolpaths from a sketch's outline, written as G-code".into(),
            icon: "pocket".into(),
            tools: vec![Tool {
                id: id(NEW),
                label: "New pocket".into(),
                icon: "pocket".into(),
                behavior: ToolBehavior::Action,
                ..Default::default()
            }],
            commands: vec![
                Command {
                    id: id(POCKET),
                    summary: "Make a pocket operation clearing a sketch's outline".into(),
                    params: vec![
                        param("sketch", ParamKind::Id, false, "the sketch whose closed loops it clears"),
                        param(
                            "outline",
                            ParamKind::List,
                            false,
                            "loops of {x, y} points in world millimetres, when no sketch",
                        ),
                        param("depth", ParamKind::Number, true, "mm below the top"),
                        param("top", ParamKind::Number, false, "the stock's top, world Z; the sketch's height, or 0"),
                        param("tool_diameter", ParamKind::Number, false, "mm; the first tool's when left out"),
                        param("stepover", ParamKind::Number, false, "mm between rows; 40% of the tool"),
                        param("step_down", ParamKind::Number, false, "mm per pass; 1"),
                        param("feed", ParamKind::Number, false, "mm/min; the tool's"),
                        param("plunge", ParamKind::Number, false, "mm/min; the tool's"),
                        param("spindle", ParamKind::Number, false, "rpm; the tool's"),
                        param("clearance", ParamKind::Number, false, "mm over the top between cuts; 5"),
                        param("name", ParamKind::String, false, "the operation's name"),
                    ],
                    returns: "{feature}".into(),
                    notes: vec![
                        "The outline must lie square to Z: the tool comes down from above.".into(),
                        "The operation builds no solid and sits on no body; its toolpath is \
                         worked out when the CAM workbench shows it, or by io.github.gilbertorconde.cam.gcode."
                            .into(),
                        "Every number is the operation's own: a formula may drive any of them."
                            .into(),
                    ],
                    examples: vec![Example {
                        title: "A 40 by 30 pocket, 3 mm deep".into(),
                        script: "local op = pc.io.github.gilbertorconde.cam.pocket{outline = {{{0, 0}, {40, 0}, \
                                 {40, 30}, {0, 30}}}, depth = 3, tool_diameter = 6}\n\
                                 assert(op.feature, \"an operation\")\n\
                                 assert(not pcall(pc.io.github.gilbertorconde.cam.pocket, {depth = 3}), \
                                 \"an outline is needed\")\n"
                            .into(),
                        ..Default::default()
                    }],
                    see_also: vec![id(GCODE), id(TOOLPATH)],
                    ..Default::default()
                },
                Command {
                    id: id(TOOLPATH),
                    summary: "Where a pocket operation's toolpath stands, and the toolpath when ready"
                        .into(),
                    params: id_only(),
                    returns: "{state = none|running|stopped|failed|ready, passes?, levels?, runs?, \
                              cut_mm?, error?}"
                        .into(),
                    read_only: true,
                    notes: vec![
                        "It never works the toolpath out itself: io.github.gilbertorconde.cam.gcode does, or the \
                         workbench's job."
                            .into(),
                        "Runs are lines of {x, y} points cut at every one of the levels.".into(),
                    ],
                    examples: vec![Example {
                        title: "A pocket's passes".into(),
                        script: "local op = pc.io.github.gilbertorconde.cam.pocket{outline = {{{0, 0}, {20, 0}, \
                                 {20, 20}, {0, 20}}}, depth = 2, step_down = 1, tool_diameter = 4}\n\
                                 assert(pc.io.github.gilbertorconde.cam.toolpath{id = op.feature}.state == \"none\")\n\
                                 pc.io.github.gilbertorconde.cam.gcode{id = op.feature}\n\
                                 local path = pc.io.github.gilbertorconde.cam.toolpath{id = op.feature}\n\
                                 assert(path.state == \"ready\" and path.passes == 2, \"two passes\")\n"
                            .into(),
                        ..Default::default()
                    }],
                    see_also: vec![id(GCODE)],
                    ..Default::default()
                },
                Command {
                    id: id(GCODE),
                    summary: "A pocket operation's G-code, in millimetres".into(),
                    params: id_only(),
                    returns: "the program's text".into(),
                    read_only: true,
                    notes: vec![
                        "It works the toolpath out when the operation has none for its current \
                         numbers; a pocket of more than 20000 rows is refused, left to the \
                         workbench's job."
                            .into(),
                        "Between cuts the tool rises to the clearance over the top; it goes down \
                         at the plunge rate."
                            .into(),
                    ],
                    examples: vec![Example {
                        title: "G-code for a square pocket".into(),
                        script: "local op = pc.io.github.gilbertorconde.cam.pocket{outline = {{{0, 0}, {40, 0}, \
                                 {40, 40}, {0, 40}}}, depth = 3, tool_diameter = 6}\n\
                                 local program = pc.io.github.gilbertorconde.cam.gcode{id = op.feature}\n\
                                 assert(program:find(\"G21\"), \"millimetres\")\n\
                                 assert(program:find(\"Z%-3%.000\"), \"down to the floor\")\n"
                            .into(),
                        ..Default::default()
                    }],
                    see_also: vec![id(POCKET)],
                    ..Default::default()
                },
            ],
            length_keys: [
                "top",
                "depth",
                "step_down",
                "tool_diameter",
                "stepover",
                "clearance",
            ]
            .map(String::from)
            .to_vec(),
            ..Default::default()
        }
    }

    fn feature_info(&self, node: &Node) -> FeatureInfo {
        let label = match Pocket::from(&node.data) {
            Some(p) => format!("Pocket toolpath, Ø{} mm", p.tool_diameter),
            None => "Pocket toolpath".into(),
        };
        FeatureInfo {
            icon: "pocket".into(),
            kind_label: label,
            family_label: "CAM".into(),
            ..Default::default()
        }
    }

    fn parameters(&self, _node: &Node) -> Vec<Parameter> {
        let p = |key: &str, label: &str, dim: Dim| Parameter {
            key: format!("/{key}"),
            name: Some(key.into()),
            label: label.into(),
            dim,
            pointer: format!("/{key}"),
            ..Default::default()
        };
        vec![
            p("top", "Stock top", Dim::Length),
            p("depth", "Final depth", Dim::Length),
            p("step_down", "Depth per pass", Dim::Length),
            p("tool_diameter", "Tool diameter", Dim::Length),
            p("stepover", "Stepover", Dim::Length),
            p("feed", "Feed", Dim::Number),
            p("plunge", "Plunge", Dim::Number),
            p("spindle", "Spindle", Dim::Number),
            p("clearance", "Clearance", Dim::Length),
        ]
    }

    fn run_command(&mut self, command: &str, args: Value) -> Result<Value, String> {
        match suffix(command).unwrap_or_default() {
            POCKET => {
                let sketch = args.get("sketch").and_then(Value::as_str).map(String::from);
                let mut pocket = self.new_pocket(sketch.clone());
                if sketch.is_none() {
                    pocket.outline = args
                        .get("outline")
                        .cloned()
                        .and_then(|v| serde_json::from_value(v).ok())
                        .unwrap_or_default();
                }
                let number = |key: &str| args.get(key).and_then(Value::as_f64);
                if let Some(d) = number("tool_diameter") {
                    pocket.tool_diameter = d;
                    pocket.stepover = d * 0.4;
                    pocket.tool = format!("Ø{d} mm");
                }
                for (key, field) in [
                    ("top", &mut pocket.top),
                    ("depth", &mut pocket.depth),
                    ("stepover", &mut pocket.stepover),
                    ("step_down", &mut pocket.step_down),
                    ("feed", &mut pocket.feed),
                    ("plunge", &mut pocket.plunge),
                    ("spindle", &mut pocket.spindle),
                    ("clearance", &mut pocket.clearance),
                ] {
                    if let Some(v) = number(key) {
                        *field = v;
                    }
                }
                pocket.input()?;
                let name = args.get("name").and_then(Value::as_str);
                let feature = self.make(&pocket, name)?;
                Ok(json!({ "feature": feature }))
            }
            TOOLPATH => {
                let op = args.get("id").and_then(Value::as_str).unwrap_or_default();
                Cam::operation(op)?;
                Ok(match self.paths.get(op).map(|c| &c.state) {
                    None => json!({"state": "none"}),
                    Some(State::Running(_)) => json!({"state": "running"}),
                    Some(State::Stopped) => json!({"state": "stopped"}),
                    Some(State::Failed(e)) => json!({"state": "failed", "error": e}),
                    Some(State::Ready(path)) => json!({
                        "state": "ready",
                        "passes": path.levels.len(),
                        "levels": path.levels,
                        "runs": path.runs,
                        "cut_mm": path.cut_length(),
                    }),
                })
            }
            GCODE => {
                let op = args.get("id").and_then(Value::as_str).unwrap_or_default();
                let (name, pocket) = Cam::operation(op)?;
                let path = self.toolpath_now(op, &pocket)?;
                Ok(Value::String(gcode::write(&name, &pocket, &path)))
            }
            _ => Err(format!("no command `{command}`")),
        }
    }

    fn input(&mut self, input: &Input) -> bool {
        match &input.event {
            Event::ToolActivated if input.tool.as_deref().and_then(suffix) == Some(NEW) => {
                let sketch = input
                    .pointer
                    .active_feature
                    .clone()
                    .filter(|f| host::profile(f).is_some());
                let pocket = self.new_pocket(sketch.clone());
                match self.make(&pocket, None) {
                    Ok(id) => {
                        host::request(Request::JournalLabel {
                            label: "New pocket".into(),
                        });
                        self.refresh(&id, &pocket, false);
                        self.editing = Some((id, pocket));
                        self.fresh = true;
                        self.hint = sketch.is_none().then(|| {
                            "Select a sketch with a closed outline, then click Sketch.".into()
                        });
                    }
                    Err(e) => host::error(&format!("No pocket: {e}")),
                }
                true
            }
            Event::EditFeature { feature } => {
                self.open(feature);
                true
            }
            Event::Activated => {
                for node in host::features().into_iter().filter(|n| n.kind == KIND) {
                    if let Some(pocket) = Pocket::from(&node.data) {
                        self.refresh(&node.id, &pocket, false);
                    }
                }
                true
            }
            Event::ValuesMoved { features } => {
                for id in features {
                    if let Some(node) = host::feature(id)
                        && let Some(pocket) = Pocket::from(&node.data)
                    {
                        self.refresh(id, &pocket, false);
                    }
                }
                true
            }
            Event::JobFinished { job, result } => {
                self.finished(*job, result);
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
        let mut frame = Frame::default();
        let edited = self.edited();
        if edited.is_none() {
            self.editing = None;
        }
        let editing = edited.as_ref().map(|(id, _, _)| id.clone());
        for node in host::features().into_iter().filter(|n| n.kind == KIND) {
            let Some(pocket) = Pocket::from(&node.data) else {
                continue;
            };
            let mine = Some(&node.id) == editing.as_ref();
            if !node.visible && !mine {
                continue;
            }
            let color = if mine { ACCENT } else { MUTED };
            let at = |p: P, z: f64| [p[0] as f32, p[1] as f32, z as f32];
            if let Ok(loops) = pocket.loops() {
                for l in loops {
                    frame.lines.push(Polyline {
                        points: l.iter().map(|p| at(*p, pocket.top)).collect(),
                        color: OUTLINE,
                        dashed: true,
                        closed: true,
                        ..Default::default()
                    });
                }
            }
            let Some(Computed {
                state: State::Ready(path),
                ..
            }) = self.paths.get(&node.id)
            else {
                continue;
            };
            if path.points() > DRAWN_POINTS {
                if mine {
                    frame.hud.footer.push("Toolpath too dense to draw".into());
                }
                continue;
            }
            let floor = path.levels.last().copied().unwrap_or(pocket.top);
            for run in &path.runs {
                frame.lines.push(Polyline {
                    points: run.iter().map(|p| at(*p, floor)).collect(),
                    color,
                    width: if mine { 1.5 } else { 1.0 },
                    ..Default::default()
                });
            }
        }
        let Some((id, name, pocket)) = edited else {
            return frame;
        };
        frame.hud.tool = Some(ToolHint {
            icon: "pocket".into(),
            name: name.clone(),
            prompt: "Set the tool and the depths in the panel".into(),
            keys: vec![("Esc".into(), "cancel".into())],
            ..Default::default()
        });
        if let Some(Computed {
            state: State::Ready(path),
            ..
        }) = self.paths.get(&id)
        {
            frame.status.selection = Some(format!(
                "Toolpath: {} passes, {:.0} mm cut",
                path.levels.len(),
                path.cut_length()
            ));
        }
        frame.editing = Some(id.clone());
        frame.task = Some(Task {
            title: "Pocket".into(),
            icon: "pocket".into(),
            confirmable: true,
            ..Default::default()
        });
        frame.panel = self.task_panel(&id, &name, &pocket);
        frame
    }

    fn panel_event(&mut self, slot: PanelSlot, event: PanelEvent) {
        if slot == PanelSlot::Settings {
            self.settings_event(event);
            return;
        }
        let Some((id, name, mut pocket)) = self.edited() else {
            return;
        };
        self.hint = None;
        match event {
            PanelEvent::Number { id: field, value } => {
                match field.as_str() {
                    "top" => pocket.top = value,
                    "depth" => pocket.depth = value,
                    "step_down" => pocket.step_down = value,
                    "tool_diameter" => pocket.tool_diameter = value,
                    "stepover" => pocket.stepover = value,
                    "feed" => pocket.feed = value,
                    "plunge" => pocket.plunge = value,
                    "spindle" => pocket.spindle = value,
                    "clearance" => pocket.clearance = value,
                    _ => return,
                }
                self.write(&id, &pocket);
            }
            PanelEvent::Select { id: list, index } if list == "tools" => {
                if let Some(tool) = self.settings.tools.get(index) {
                    pocket.tool = tool.name.clone();
                    pocket.tool_diameter = tool.diameter;
                    pocket.stepover = pocket.stepover.min(tool.diameter);
                    pocket.feed = tool.feed;
                    pocket.plunge = tool.plunge;
                    pocket.spindle = tool.spindle;
                    self.write(&id, &pocket);
                }
            }
            PanelEvent::Select { id: list, index } if list == "operations" => {
                let other = host::features()
                    .into_iter()
                    .filter(|n| n.kind == KIND)
                    .nth(index);
                if let Some(other) = other
                    && other.id != id
                {
                    // The edits so far stay; the task goes on with the other.
                    self.open(&other.id);
                }
            }
            PanelEvent::Pick { id: pick } if pick == "sketch" => {
                let chosen = self.pointer.active_feature.clone().filter(|f| *f != id);
                match chosen.as_deref().map(outline_of) {
                    Some(Ok((_, z))) => {
                        pocket.sketch = chosen;
                        pocket.outline.clear();
                        pocket.top = z;
                        self.write(&id, &pocket);
                    }
                    Some(Err(e)) => self.hint = Some(e),
                    None => {
                        self.hint =
                            Some("Select a sketch with a closed outline in the tree first.".into())
                    }
                }
            }
            PanelEvent::Pick { id: pick } if pick == "top_face" => match self.pointer.face {
                Some(face) => {
                    pocket.top = f64::from(face.point[2]);
                    self.write(&id, &pocket);
                }
                None => self.hint = Some("Select the stock's top face first.".into()),
            },
            PanelEvent::Button { id: button } if button == "compute" => {
                self.refresh(&id, &pocket, true)
            }
            PanelEvent::Button { id: button } if button == "stop" => self.stop(&id),
            PanelEvent::Button { id: button } if button == "save" => self.save(&id, &name, &pocket),
            _ => {}
        }
    }

    fn task_close(&mut self, accept: bool) -> Option<String> {
        let (id, opened) = self.editing.take()?;
        self.hint = None;
        if accept {
            return Some("Edit pocket".into());
        }
        self.stop(&id);
        let result = if self.fresh {
            self.paths.remove(&id);
            host::remove_feature(&id)
        } else {
            host::set_feature_data(&id, opened.to_value())
        };
        if let Err(e) = result {
            host::error(&e);
        }
        None
    }

    fn delete_feature(&mut self, id: &str) -> bool {
        self.stop(id);
        self.paths.remove(id);
        false
    }

    fn menu_items(&mut self, scope: &MenuScope) -> Vec<MenuItem> {
        match scope {
            MenuScope::TreeFeature(feature)
                if host::feature(feature).is_some_and(|n| n.kind == KIND) =>
            {
                vec![MenuItem {
                    id: id(EDIT),
                    label: "Edit pocket".into(),
                    icon: Some("pocket".into()),
                    ..Default::default()
                }]
            }
            _ => Vec::new(),
        }
    }

    fn menu_command(&mut self, command: &str, scope: &MenuScope) -> bool {
        match (suffix(command), scope) {
            (Some(EDIT), MenuScope::TreeFeature(feature)) => {
                self.open(feature);
                true
            }
            _ => false,
        }
    }

    fn settings_panel(&mut self) -> Vec<Widget> {
        let rows = self
            .settings
            .tools
            .iter()
            .map(|t| {
                vec![
                    t.name.clone(),
                    format!("{}", t.diameter),
                    format!("{}", t.feed),
                    format!("{}", t.plunge),
                    format!("{}", t.spindle),
                ]
            })
            .collect();
        vec![
            Widget::Heading {
                text: "Tools".into(),
            },
            Widget::Text {
                text: "An operation takes a copy of its tool's numbers when the tool is picked."
                    .into(),
                mono: false,
            },
            Widget::Table {
                id: "tools".into(),
                columns: ["Name", "Ø mm", "Feed", "Plunge", "rpm"]
                    .map(String::from)
                    .to_vec(),
                rows,
                selected: self.chosen_tool,
                editable: vec![true; 5],
            },
            Widget::Button {
                id: "add_tool".into(),
                label: "Add a tool".into(),
                style: ButtonStyle::Secondary,
                enabled: true,
            },
            Widget::Button {
                id: "remove_tool".into(),
                label: "Remove the selected tool".into(),
                style: ButtonStyle::Destructive,
                enabled: self.chosen_tool.is_some(),
            },
            Widget::Separator,
            Widget::Number {
                id: "clearance".into(),
                label: "Clearance".into(),
                value: self.settings.clearance,
                dim: Dim::Length,
                bind: None,
                min: Some(0.1),
                max: None,
                decimals: 1,
                error: None,
            },
        ]
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
        let input: path::Input = serde_json::from_str(input).map_err(|e| e.to_string())?;
        let path = path::plan(&input, |done, of| {
            if done % 64 == 0 {
                host::progress(done as u64, of as u64);
            }
            !host::cancelled()
        })?;
        serde_json::to_string(&path).map_err(|e| e.to_string())
    }
}

bench!(Cam);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_id_starts_with_the_package() {
        assert_eq!(suffix(KIND), Some(POCKET));
        let registration = Cam::default().describe();
        for tool in &registration.tools {
            assert_eq!(suffix(&tool.id), Some(NEW), "{}", tool.id);
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
        assert!(manifest.contains(&format!("[\"{KIND}\"]")));
    }
}
