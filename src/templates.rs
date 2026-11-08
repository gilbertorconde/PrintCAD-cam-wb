//! Templates: a setup's rules and its operations, or some operations alone,
//! kept as a file to make again on another part, as Fusion's templates and
//! a job template are.
//!
//! A template keeps what does not depend on the part: a setup's stock rules,
//! work zero rule, orientation, post, tolerance and programs; each
//! operation's strategy, tool, heights, linking, passes and pattern. What was
//! picked on the part it was saved from is left out. Each operation says
//! instead how it is filled on the next part ([`Fill`]): with nothing, as it
//! needs no pick; waiting for a pick; on the part's silhouette; or every hole
//! of a diameter. Its tools go with it, and are matched to the document's
//! when it is applied ([`match_tool`]), else added.
//!
//! Templates are JSON files in the package's own folder, `/data/templates`,
//! which printCAD keeps for the package across updates; a few are built in.
//! Each file says its format and version, so a newer package can read an
//! older one, and an older package refuses a newer one rather than guess.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use camcraft_core::job::{Boundary, HeightRef, Outline, Preset, StrategySpec, ToolKind, ToolSpec};
use printcad_bench_sdk::serde_json::{self, Value};
use serde::{Deserialize, Serialize};

use printcad_bench_sdk::api::{
    ButtonStyle, FileKind, NoteKind, OpenedFile, PanelEvent, Request, Widget,
};

use crate::data::{OperationData, SetupData};

/// What a template file says it is.
pub const FORMAT: &str = "io.github.gilbertorconde.cam.template";
/// The version this package writes, and the newest it reads.
pub const VERSION: u64 = 1;
/// Where the user's templates are kept: the package's data folder, the
/// one part of the disk printCAD lets it reach.
pub const DIR: &str = "/data/templates";

/// A setup's rules and operations, or operations alone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Template {
    pub format: String,
    pub version: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// The setup's rules, its bodies, picks, probe results and tools left
    /// out; `None` for operations alone. An empty post keeps the setup's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<SetupData>,
    /// The tools its operations cut with, by the names they use.
    #[serde(default)]
    pub tools: BTreeMap<String, ToolSpec>,
    pub operations: Vec<TemplateOperation>,
}

/// One operation of a template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplateOperation {
    pub name: String,
    /// Its data, no setup and nothing picked.
    pub data: OperationData,
    #[serde(default)]
    pub fill: Fill,
}

/// How an operation is filled on the part a template is applied to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Fill {
    /// It picks nothing: it cuts the stock, the part's silhouette or the
    /// like, on any part.
    #[default]
    Nothing,
    /// It waits for a pick, saying what.
    Pick,
    /// Its boundary or outline is the part's silhouette, not picks.
    Silhouette,
    /// It takes every hole this wide, in millimetres, along the tool.
    Holes(f64),
}

impl Fill {
    /// Its name in a script or a file: `pick`, `silhouette`, `{holes = 6.8}`.
    pub fn named(given: &Value) -> Result<Self, String> {
        serde_json::from_value(given.clone()).map_err(|_| {
            "a fill is nothing, pick, silhouette or {holes = <diameter in mm>}".to_string()
        })
    }

    /// What it comes to, in words.
    pub fn words(self) -> String {
        match self {
            Fill::Nothing => "picks nothing".into(),
            Fill::Pick => "needs a pick".into(),
            Fill::Silhouette => "on the part's silhouette".into(),
            Fill::Holes(d) => format!("every hole Ø{}", crate::sheet::num(d)),
        }
    }
}

/// What an operation of `data`'s strategy is picked for, in words.
pub fn what_it_picks(data: &OperationData) -> &'static str {
    let uses = data.strategy().uses();
    if uses.holes {
        "a hole's wall or rim"
    } else if uses.traces {
        "the edges or sketches it follows"
    } else if uses.carves {
        "the sketches it carves"
    } else {
        "the faces, edges or sketches it machines"
    }
}

/// How `data`, as it is in the document, is filled on another part when no
/// rule is given: by its holes' diameter `hole` (the one picked, where it
/// takes every hole as wide), else waiting for a pick where it reads what is
/// picked, else nothing.
pub fn fill_of(data: &OperationData, hole: Option<f64>) -> Fill {
    let uses = data.strategy().uses();
    let picked = !(data.faces.is_empty() && data.edges.is_empty() && data.sketches.is_empty());
    if uses.holes
        && !picked
        && let Some(d) = data.operation.holes.diameter
    {
        return Fill::Holes(d);
    }
    if uses.holes
        && data.operation.holes.same_diameter
        && data.operation.holes.diameter.is_none()
        && let Some(d) = hole
    {
        return Fill::Holes(d);
    }
    if reads_picks(data) {
        Fill::Pick
    } else {
        Fill::Nothing
    }
}

/// `setup` without what belongs to its part: its bodies, what was picked
/// on them, its probe results, the setup it carries on from (by id, in its
/// document), the bodies its stock and its fixtures are, and its tools,
/// which the template keeps beside it.
fn rules_of(setup: &SetupData) -> SetupData {
    let mut rules = setup.clone();
    rules.bodies.clear();
    rules.stock_from = None;
    rules.stock_body = None;
    rules.fixtures.clear();
    rules.axis_pick = None;
    rules.x_pick = None;
    rules.zero_point = None;
    rules.probe_points.clear();
    rules.probe_file.clear();
    rules.tools.clear();
    rules
}

/// `data` without what was picked, and in no setup.
fn unpicked(data: &OperationData) -> OperationData {
    let mut data = data.clone();
    data.setup = None;
    data.faces.clear();
    data.edges.clear();
    data.sketches.clear();
    data.curves.clear();
    data.operation.selection = Default::default();
    data.needs_pick = None;
    data
}

/// A template named `name` of `operations` (their names, data and fills),
/// cutting with `setup`'s tools; with `setup`'s rules too when `whole`.
pub fn make(
    name: &str,
    description: &str,
    setup: &SetupData,
    whole: bool,
    operations: &[(String, OperationData, Fill)],
) -> Template {
    let mut tools = BTreeMap::new();
    for (_, data, _) in operations {
        if let Some(t) = &data.operation.tool
            && let Some(spec) = setup.tools.get(t)
        {
            tools.insert(t.clone(), spec.clone());
        }
    }
    Template {
        format: FORMAT.into(),
        version: VERSION,
        name: name.trim().to_string(),
        description: description.trim().to_string(),
        setup: whole.then(|| rules_of(setup)),
        tools,
        operations: operations
            .iter()
            .map(|(n, d, f)| TemplateOperation {
                name: n.clone(),
                data: unpicked(d),
                fill: *f,
            })
            .collect(),
    }
}

/// A template's file text.
pub fn text_of(t: &Template) -> String {
    serde_json::to_string_pretty(t).unwrap_or_default()
}

/// The template in `text`, a file's: refused, saying why, when it is no
/// template, or one a newer package wrote.
pub fn parse(text: &str) -> Result<Template, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| format!("it is not JSON: {e}"))?;
    if value.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err("it is not a CAM template".into());
    }
    let version = value
        .get("version")
        .and_then(Value::as_u64)
        .ok_or("it says no version")?;
    if version > VERSION {
        return Err(format!(
            "it is version {version}, from a newer CAM package; this one reads up to \
             version {VERSION}: update the package"
        ));
    }
    let value = upgrade(value, version)?;
    serde_json::from_value(value).map_err(|e| format!("it does not read: {e}"))
}

/// A template of version `version` as this version has it. Version 1 is
/// the first; a later version that changes the layout upgrades here, and a
/// field added since is its default where an older one has none.
fn upgrade(mut value: Value, version: u64) -> Result<Value, String> {
    if version == 0 {
        return Err("it is version 0, which no CAM package wrote".into());
    }
    if let Some(v) = value.get_mut("version") {
        *v = Value::from(VERSION);
    }
    Ok(value)
}

/// Where the tool a template's operation wants was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// In the setup it is applied to.
    Setup,
    /// In another setup of the document.
    Document,
    /// In the tool library.
    Library,
    /// Nowhere: the template's own is added.
    Template,
}

impl Found {
    pub fn words(self) -> &'static str {
        match self {
            Found::Setup => "this setup's",
            Found::Document => "another setup's",
            Found::Library => "the library's",
            Found::Template => "the template's, added",
        }
    }
}

/// Whether two numbers are one, as tools are measured.
fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-3
}

/// Whether `a` and `b` are the same shape of tool: kind, diameter, corner,
/// angle and tip, which is what it cuts; and where its kind has them, its
/// taper or lead, its neck, a tap's pitch and hand, and the flute length a
/// tapered mill tapers up, a face mill leads up and a slot cutter is thick.
pub fn same_shape(a: &ToolSpec, b: &ToolSpec) -> bool {
    let same = |x: Option<f64>, y: Option<f64>| match (x, y) {
        (Some(x), Some(y)) => near(x, y),
        (None, None) => true,
        _ => false,
    };
    let height = matches!(
        a.kind,
        ToolKind::Tapered | ToolKind::FaceMill | ToolKind::SlotCutter
    );
    a.kind == b.kind
        && near(a.diameter, b.diameter)
        && near(a.corner, b.corner)
        && near(a.tip_diameter, b.tip_diameter)
        && same(a.point_angle().or(a.angle), b.point_angle().or(b.angle))
        && same(a.taper_angle, b.taper_angle)
        && same(a.neck_diameter, b.neck_diameter)
        && same(a.pitch, b.pitch)
        && a.hand == b.hand
        && (!height || same(a.flute_length, b.flute_length))
}

/// The tool to cut with for a template's `want`: one of the setup's
/// (`setup`), another setup's (`document`) or the library's (`library`)
/// with the same number and shape, else with the same shape, looked for in
/// that order; `None` when there is none.
pub fn match_tool(
    want: &ToolSpec,
    setup: &BTreeMap<String, ToolSpec>,
    document: &[&BTreeMap<String, ToolSpec>],
    library: &BTreeMap<String, ToolSpec>,
) -> Option<(String, ToolSpec, Found)> {
    let places = std::iter::once((Found::Setup, setup))
        .chain(document.iter().map(|d| (Found::Document, *d)))
        .chain(std::iter::once((Found::Library, library)));
    let places: Vec<_> = places.collect();
    for numbered in [true, false] {
        for (found, tools) in &places {
            if let Some((name, spec)) = tools
                .iter()
                .find(|(_, t)| same_shape(t, want) && (!numbered || t.number == want.number))
            {
                return Some((name.clone(), spec.clone(), *found));
            }
        }
    }
    None
}

/// A tool of a template, and what it cuts as where it was applied.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolUsed {
    pub wanted: String,
    pub name: String,
    pub found: Found,
}

/// What applying a template made: the operations, named, for the setup;
/// the tools as matched; and the operations left waiting for a pick.
#[derive(Debug, Clone, Default)]
pub struct Applied {
    pub operations: Vec<(String, OperationData)>,
    pub tools: Vec<ToolUsed>,
    pub waiting: Vec<String>,
}

/// Take a template's rules into `setup`, keeping its bodies, tools, probe
/// results and whether it runs; an empty post keeps its post too.
pub fn take_rules(rules: &SetupData, setup: &mut SetupData) {
    let mut taken = rules.clone();
    taken.bodies = std::mem::take(&mut setup.bodies);
    taken.tools = std::mem::take(&mut setup.tools);
    taken.probe_points = std::mem::take(&mut setup.probe_points);
    taken.probe_file = std::mem::take(&mut setup.probe_file);
    taken.enabled = setup.enabled;
    if taken.post.is_empty() {
        taken.post = std::mem::take(&mut setup.post);
        taken.post_profile = setup.post_profile.take();
    }
    *setup = taken;
}

/// `t`'s operations for `setup` (whose id is `setup_id`): each tool matched
/// to the document's ([`match_tool`]) and taken into the setup, or the
/// template's own added; each operation filled as it says.
pub fn apply(
    t: &Template,
    setup_id: &str,
    setup: &mut SetupData,
    document: &[&BTreeMap<String, ToolSpec>],
    library: &BTreeMap<String, ToolSpec>,
) -> Applied {
    let mut applied = Applied::default();
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    for (wanted, spec) in &t.tools {
        let (name, spec, found) = match_tool(spec, &setup.tools, document, library)
            .unwrap_or_else(|| (wanted.clone(), spec.clone(), Found::Template));
        let name = setup.adopt(&name, &spec);
        names.insert(wanted.clone(), name.clone());
        applied.tools.push(ToolUsed {
            wanted: wanted.clone(),
            name,
            found,
        });
    }
    for op in &t.operations {
        let mut data = unpicked(&op.data);
        data.setup = Some(setup_id.to_string());
        if let Some(tool) = &data.operation.tool {
            // A tool the template does not carry is looked for by name.
            data.operation.tool = Some(names.get(tool).cloned().unwrap_or_else(|| tool.clone()));
        }
        if fill(&mut data, op.fill) {
            applied.waiting.push(op.name.clone());
        }
        applied.operations.push((op.name.clone(), data));
    }
    applied
}

/// Fill `data` as `fill` says; whether it is left waiting for a pick:
/// whatever the rule, one that still reads picks waits for them.
fn fill(data: &mut OperationData, fill: Fill) -> bool {
    let uses = data.strategy().uses();
    match fill {
        Fill::Nothing | Fill::Pick => {}
        Fill::Silhouette => {
            if uses.boundary && data.operation.boundary.kind == Boundary::Selection {
                data.operation.boundary.kind = Boundary::Silhouette;
            }
            if uses.outline {
                data.operation.outline = Outline::Silhouette;
            }
        }
        Fill::Holes(d) => {
            if uses.holes {
                data.operation.holes.diameter = Some(d);
            }
        }
    }
    if reads_picks(data) {
        data.needs_pick = Some(what_it_picks(data).into());
        return true;
    }
    false
}

/// Whether `data` reads what is picked: what it cuts, or a height measured
/// from it; not a hole operation that finds its holes by their diameter.
fn reads_picks(data: &OperationData) -> bool {
    let uses = data.strategy().uses();
    if uses.holes && data.operation.holes.diameter.is_some() {
        return false;
    }
    let measured =
        [data.heights.top_from, data.heights.bottom_from].contains(&Some(HeightRef::Selection));
    measured || data.picked("", |b| Some(b.to_string())).cuts_selection()
}

/// A file name for a template named `name`: its letters and digits, the
/// rest dashes.
pub fn file_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    format!("{}.json", if out.is_empty() { "template" } else { out })
}

/// Every template, built in then the user's, and each file that does not
/// read, with why.
pub type Shelf = std::rc::Rc<(Vec<Kept>, Vec<String>)>;

thread_local! {
    /// The built-in templates, made once: making them makes camcraft's
    /// tool library and their operations, too much for every frame.
    static BUILT_IN: Shelf = std::rc::Rc::new((built_in(), Vec::new()));
}

/// A template kept as a file, or built in.
#[derive(Debug, Clone, PartialEq)]
pub struct Kept {
    pub template: Template,
    /// Its file, by name; `None` for a built-in one.
    pub file: Option<String>,
}

/// The user's templates in `dir`, by name, and each file that does not
/// read with why.
pub fn read_dir(dir: &Path) -> (Vec<Kept>, Vec<String>) {
    let mut kept = Vec::new();
    let mut bad = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (kept, bad);
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    for path in files {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| parse(&t))
        {
            Ok(template) => kept.push(Kept {
                template,
                file: Some(file),
            }),
            Err(e) => bad.push(format!("{file}: {e}")),
        }
    }
    kept.sort_by(|a, b| a.template.name.cmp(&b.template.name));
    (kept, bad)
}

/// Write `text` as the template file `file` in `dir`.
pub fn write(dir: &Path, file: &str, text: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    std::fs::write(dir.join(file), text).map_err(|e| format!("cannot write {file}: {e}"))
}

/// Take the template file `file` out of `dir`.
pub fn remove(dir: &Path, file: &str) -> Result<(), String> {
    std::fs::remove_file(dir.join(file)).map_err(|e| format!("cannot remove {file}: {e}"))
}

/// The job that reads and writes the template files: the disk can take
/// longer than a frame, or a command, may, when the machine is busy.
pub const JOB: &str = "templates";

/// What a [`JOB`] does with the template files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Work {
    /// Read them all: what comes back is a [`Read`].
    Read,
    Write {
        file: String,
        text: String,
    },
    Remove {
        file: String,
    },
}

/// The template files read, and each that does not read with why.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Read {
    pub templates: Vec<(String, Template)>,
    pub bad: Vec<String>,
}

/// Do `work` in `dir`, as a [`JOB`] does: a [`Read`] as JSON for
/// [`Work::Read`], nothing for the rest.
pub fn work(dir: &Path, work: &Work) -> Result<String, String> {
    match work {
        Work::Read => {
            let (kept, bad) = read_dir(dir);
            let read = Read {
                templates: kept
                    .into_iter()
                    .map(|k| (k.file.unwrap_or_default(), k.template))
                    .collect(),
                bad,
            };
            serde_json::to_string(&read).map_err(|e| e.to_string())
        }
        Work::Write { file, text } => write(dir, file, text).map(|()| String::new()),
        Work::Remove { file } => remove(dir, file).map(|()| String::new()),
    }
}

/// A tool of a built-in template, with one preset its operations take.
fn tool(spec: ToolSpec, preset: Preset) -> ToolSpec {
    ToolSpec {
        presets: vec![preset],
        ..spec
    }
}

/// An operation of a built-in template: `strategy` cutting with `tool`
/// (named `name`) from its first preset, then `adjust`ed.
fn built(
    strategy: &str,
    name: &str,
    tool: &ToolSpec,
    fill: Fill,
    label: &str,
    adjust: impl FnOnce(&mut OperationData),
) -> TemplateOperation {
    let mut data = OperationData::new(
        None,
        StrategySpec::named(strategy).unwrap_or(StrategySpec::CustomGcode {}),
    );
    data.use_tool(name, tool, tool.presets.first(), 0.02);
    adjust(&mut data);
    TemplateOperation {
        name: label.into(),
        data,
        fill,
    }
}

/// The templates that come with the package.
pub fn built_in() -> Vec<Kept> {
    let mut out = Vec::new();
    // Aluminium with a 6 mm three-flute carbide end mill and a 90° chamfer
    // mill, at a router's speeds: a hobby machine's numbers, light enough
    // for most.
    let flat = tool(
        ToolSpec {
            kind: ToolKind::Flat,
            diameter: 6.0,
            number: 1,
            rpm: 18_000,
            flutes: Some(3),
            flute_length: Some(16.0),
            comment: "6 mm 3-flute carbide end mill, for aluminium".into(),
            ..ToolSpec::default()
        },
        Preset {
            name: "6061 aluminium".into(),
            rpm: Some(18_000),
            feed: Some(1_000.0),
            plunge_feed: Some(300.0),
            ramp_feed: Some(600.0),
            stepdown: Some(3.0),
            stepover: Some(3.6),
            ..Preset::default()
        },
    );
    let chamfer = tool(
        ToolSpec {
            kind: ToolKind::Cone,
            diameter: 6.0,
            angle: Some(90.0),
            number: 2,
            rpm: 18_000,
            flutes: Some(2),
            comment: "6 mm 90° chamfer mill".into(),
            ..ToolSpec::default()
        },
        Preset {
            name: "6061 aluminium".into(),
            rpm: Some(18_000),
            feed: Some(600.0),
            plunge_feed: Some(200.0),
            ..Preset::default()
        },
    );
    let mut setup = SetupData::new(Vec::new(), "", 0.02);
    setup.material = "6061 aluminium".into();
    setup.margin_side = 3.0;
    setup.margin_top = 1.0;
    let (flat_name, chamfer_name) = ("6 mm flat", "6 mm 90° chamfer");
    let operations = vec![
        built("face", flat_name, &flat, Fill::Nothing, "Face", |_| {}),
        built(
            "adaptive",
            flat_name,
            &flat,
            Fill::Nothing,
            "Adaptive rough",
            |d| {
                d.stepover = 0.6;
                d.multiple_depths = true;
                d.stepdown = 6.0;
                d.radial = 0.2;
                d.axial = 0.2;
            },
        ),
        built(
            "contour",
            flat_name,
            &flat,
            Fill::Nothing,
            "Contour finish",
            |d| {
                d.stepdown = 0.5;
                d.feed = 800.0;
                d.radial = 0.0;
                d.axial = 0.0;
            },
        ),
        built(
            "chamfer",
            chamfer_name,
            &chamfer,
            Fill::Pick,
            "Chamfer",
            |_| {},
        ),
    ];
    out.push(Kept {
        template: Template {
            format: FORMAT.into(),
            version: VERSION,
            name: "Aluminium: face, rough, finish, chamfer".into(),
            description: "6061 aluminium with a 6 mm flat end mill: face the top, rough \
                          with adaptive clearing leaving 0.2 mm, finish the walls with \
                          contour, then chamfer the edges picked with a 90° chamfer mill."
                .into(),
            setup: Some(setup),
            tools: BTreeMap::from([
                (flat_name.to_string(), flat),
                (chamfer_name.to_string(), chamfer),
            ]),
            operations,
        },
        file: None,
    });
    // A part cut out of a sheet with the library's 1/4" upcut, at its
    // first preset's numbers.
    let mut root = camcraft_session::library::Folder::default();
    camcraft_session::library::system::add_system(&mut root);
    if let Some((_, name, upcut)) = root
        .walk()
        .into_iter()
        .find(|(_, name, _)| *name == "1/4 upcut")
    {
        let mut setup = SetupData::new(Vec::new(), "", 0.05);
        setup.material = "Plywood".into();
        setup.margin_side = 5.0;
        setup.margin_top = 0.0;
        let operations = vec![
            built(
                "adaptive",
                name,
                upcut,
                Fill::Nothing,
                "Adaptive clear",
                |d| {
                    d.radial = 0.3;
                    d.axial = 0.3;
                },
            ),
            built("profile", name, upcut, Fill::Nothing, "Profile out", |d| {
                d.passes.tabs = true;
            }),
        ];
        out.push(Kept {
            template: Template {
                format: FORMAT.into(),
                version: VERSION,
                name: "Sheet part: clear, profile on tabs".into(),
                description: "A part in a sheet with a 1/4\" upcut: clear down to it with \
                              adaptive clearing, then cut it out round its outline on tabs."
                    .into(),
                setup: Some(setup),
                tools: BTreeMap::from([(name.to_string(), upcut.clone())]),
                operations,
            },
            file: None,
        });
    }
    out
}

/// What the setup panel's Templates group has chosen, between frames.
#[derive(Debug, Clone, Default)]
pub struct Ui {
    /// The template chosen, by its place in [`Cam::templates`].
    pub chosen: usize,
    /// The name to save as; the setup's when not typed.
    pub name: Option<String>,
    /// Whether to save its operations alone, not the setup's rules.
    pub alone: bool,
    /// Whether applying takes the template's setup rules too; when not
    /// chosen, only into a setup with no operations yet.
    pub rules: Option<bool>,
    /// Whether the group is shown first and open: the setup was made to
    /// apply one to.
    pub first: bool,
}

/// The dialog's id for a template file to import.
pub const OPEN_TEMPLATE: &str = "template";

/// Where a template is applied.
pub enum Target {
    /// A new setup on these bodies, named so or as the next.
    New {
        bodies: Vec<String>,
        name: Option<String>,
    },
    /// An existing setup, its operations appended; its rules too when
    /// `rules`.
    Setup { id: String, rules: bool },
}

/// What applying made, by feature id.
pub struct Made {
    pub setup: String,
    pub operations: Vec<String>,
    pub applied: Applied,
}

impl Made {
    /// What it made, in words.
    pub fn said(&self, t: &Template) -> String {
        let mut said = format!(
            "{} operation(s) from {}.",
            self.applied.operations.len(),
            t.name
        );
        for u in &self.applied.tools {
            said.push_str(&format!(
                " {} cuts with {} ({}).",
                u.wanted,
                u.name,
                u.found.words()
            ));
        }
        if !self.applied.waiting.is_empty() {
            said.push_str(&format!(
                " Needs a pick: {}.",
                self.applied.waiting.join(", ")
            ));
        }
        said
    }

    pub fn value(&self) -> Value {
        printcad_bench_sdk::json!({
            "setup": self.setup,
            "operations": self.operations,
            "tools": self.applied.tools.iter().map(|u| printcad_bench_sdk::json!({
                "template": u.wanted,
                "as": u.name,
                "from": match u.found {
                    Found::Setup => "setup",
                    Found::Document => "document",
                    Found::Library => "library",
                    Found::Template => "template",
                },
            })).collect::<Vec<_>>(),
            "needs_pick": self.applied.waiting,
        })
    }
}

/// The diameter of the hole `pick` is on (its wall, or its rim), as the
/// host finds the body's holes; `None` where it is on none.
fn hole_across(pick: &crate::data::Pick) -> Option<f64> {
    use printcad_bench_sdk::api::calls;
    let holes = printcad_bench_sdk::host::call(
        calls::GEOM_HOLES,
        printcad_bench_sdk::json!({ "body": pick.body }),
    )
    .ok()?;
    let three = |v: &Value| -> Option<[f64; 3]> {
        let a = v.as_array()?;
        Some([
            a.first()?.as_f64()?,
            a.get(1)?.as_f64()?,
            a.get(2)?.as_f64()?,
        ])
    };
    let mut best: Option<(f64, f64)> = None;
    for h in holes.as_array()? {
        let (Some(entry), Some(dir), Some(d)) = (
            h.get("entry").and_then(three),
            h.get("direction").and_then(three),
            h.get("diameter").and_then(Value::as_f64),
        ) else {
            continue;
        };
        let depth = h.get("depth").and_then(Value::as_f64).unwrap_or(f64::MAX);
        let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
        if len < 1e-9 {
            continue;
        }
        let rel = [0, 1, 2].map(|k| pick.at[k] - entry[k]);
        let along = (0..3).map(|k| rel[k] * dir[k]).sum::<f64>() / len;
        let off = ((0..3).map(|k| rel[k] * rel[k]).sum::<f64>() - along * along)
            .max(0.0)
            .sqrt();
        let miss = (off - d / 2.0).abs();
        if miss <= 0.05 && along.abs() <= depth + 0.05 && best.is_none_or(|(m, _)| miss < m) {
            best = Some((miss, d));
        }
    }
    best.map(|(_, d)| (d * 1000.0).round() / 1000.0)
}

impl crate::Cam {
    /// The templates: the built-in ones, then the user's once read
    /// ([`Self::load_templates`]); with each file that does not read. It
    /// reads nothing itself, so a frame may ask.
    pub(crate) fn templates(&self) -> Shelf {
        if let Some(shelf) = self.templates.borrow().clone() {
            return shelf;
        }
        BUILT_IN.with(|b| b.clone())
    }

    /// Whether the user's templates have been read.
    pub(crate) fn templates_read(&self) -> bool {
        self.templates.borrow().is_some()
    }

    /// Read the user's templates in a job, unless they are read or being
    /// read; now where no job starts.
    pub(crate) fn load_templates(&self) {
        if self.templates_read()
            || self
                .template_jobs
                .borrow()
                .iter()
                .any(|(_, w)| *w == Work::Read)
        {
            return;
        }
        self.file_work(Work::Read);
    }

    /// Read the user's templates now, unless they are read: for a script
    /// command, which waits for its answer.
    pub(crate) fn load_templates_now(&self) {
        if !self.templates_read() {
            self.read_templates(work(Path::new(DIR), &Work::Read));
        }
    }

    /// Take a [`Work::Read`]'s answer as the user's templates.
    fn read_templates(&self, answer: Result<String, String>) {
        let read: Read =
            match answer.and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
                Ok(r) => r,
                Err(e) => Read {
                    bad: vec![format!("the folder: {e}")],
                    ..Read::default()
                },
            };
        let mut all = BUILT_IN.with(|b| b.0.clone());
        all.extend(read.templates.into_iter().map(|(file, template)| Kept {
            template,
            file: Some(file),
        }));
        *self.templates.borrow_mut() = Some(std::rc::Rc::new((all, read.bad)));
    }

    /// Do `work` in a job; now where no job starts.
    fn file_work(&self, w: Work) {
        let input = serde_json::to_string(&w).unwrap_or_default();
        match printcad_bench_sdk::host::start_job(JOB, &input) {
            Ok(job) => self.template_jobs.borrow_mut().push((job, w)),
            Err(_) => {
                let answer = work(Path::new(DIR), &w);
                self.file_work_done(&w, answer);
            }
        }
    }

    /// What came of `w`: the templates read, or a write that failed said.
    fn file_work_done(&self, w: &Work, answer: Result<String, String>) {
        match (w, answer) {
            (Work::Read, answer) => {
                if !self.templates_read() {
                    self.read_templates(answer);
                }
            }
            (_, Ok(_)) => {}
            (_, Err(e)) => printcad_bench_sdk::host::warn(&format!("A template file: {e}")),
        }
    }

    /// Job `job`'s end, if it is a template file job: whether it was.
    pub(crate) fn template_job_finished(&self, job: u64, answer: &Result<String, String>) -> bool {
        let mut jobs = self.template_jobs.borrow_mut();
        let Some(at) = jobs.iter().position(|(j, _)| *j == job) else {
            return false;
        };
        let (_, w) = jobs.remove(at);
        drop(jobs);
        self.file_work_done(&w, answer.clone());
        true
    }

    /// The template named `name`, built in or the user's.
    pub(crate) fn template_named(&self, name: &str) -> Result<Template, String> {
        let all = self.templates();
        all.0
            .iter()
            .rev()
            .find(|k| k.template.name == name)
            .map(|k| k.template.clone())
            .ok_or_else(|| {
                let names: Vec<&str> = all.0.iter().map(|k| k.template.name.as_str()).collect();
                format!("no template '{name}': {}", names.join("; "))
            })
    }

    /// A template named `name` of the operations `ops` (feature ids) of
    /// setup `setup`, every one of it when empty, with its rules too when
    /// `whole`; each filled as `fills` says by its name, else as it is.
    pub(crate) fn template_of(
        &self,
        name: &str,
        description: &str,
        setup: &str,
        ops: &[String],
        whole: bool,
        fills: &BTreeMap<String, Fill>,
    ) -> Result<Template, String> {
        let (_, data) = crate::setup_node(setup).ok_or("no such setup")?;
        let chosen: Vec<_> = crate::operations_of(setup)
            .into_iter()
            .filter(|(n, _)| ops.is_empty() || ops.contains(&n.id))
            .map(|(n, d)| {
                let fill = fills.get(&n.name).copied().unwrap_or_else(|| {
                    let hole = (d.strategy().uses().holes && d.operation.holes.same_diameter)
                        .then(|| d.faces.iter().chain(&d.edges).find_map(hole_across))
                        .flatten();
                    fill_of(&d, hole)
                });
                (n.name, d, fill)
            })
            .collect();
        if chosen.is_empty() && !whole {
            return Err("no operations to keep".into());
        }
        Ok(make(name, description, &data, whole, &chosen))
    }

    /// Keep `t` as a file ([`file_name`]), written by a job: refused when
    /// one of the user's has that file and `replace` is not asked. The
    /// file's name, and whether it replaced one.
    pub(crate) fn keep_template(
        &self,
        t: &Template,
        replace: bool,
    ) -> Result<(String, bool), String> {
        if t.name.is_empty() {
            return Err("a template needs a name".into());
        }
        self.load_templates_now();
        let file = file_name(&t.name);
        let shelf = self.templates();
        let mut all = shelf.0.clone();
        let there = all.iter().position(|k| k.file.as_deref() == Some(&file));
        let kept = Kept {
            template: t.clone(),
            file: Some(file.clone()),
        };
        match there {
            Some(_) if !replace => {
                return Err(format!("there is a template '{}' already", t.name));
            }
            Some(i) => all[i] = kept,
            None => {
                // After the built-in ones, by name.
                let at = all
                    .iter()
                    .position(|k| k.file.is_some() && k.template.name > t.name)
                    .unwrap_or(all.len());
                all.insert(at, kept);
            }
        }
        *self.templates.borrow_mut() = Some(std::rc::Rc::new((all, shelf.1.clone())));
        self.file_work(Work::Write {
            file: file.clone(),
            text: text_of(t),
        });
        Ok((file, there.is_some()))
    }

    /// Take the user's template file `file` out, by a job.
    pub(crate) fn drop_template(&self, file: &str) {
        let shelf = self.templates();
        let all = shelf
            .0
            .iter()
            .filter(|k| k.file.as_deref() != Some(file))
            .cloned()
            .collect();
        *self.templates.borrow_mut() = Some(std::rc::Rc::new((all, shelf.1.clone())));
        self.file_work(Work::Remove {
            file: file.to_string(),
        });
    }

    /// Save the setup or the operation `feature` as a template named after
    /// it, as its menu in the tree does: a name of its own among the
    /// user's.
    pub(crate) fn save_from_tree(&mut self, feature: &str) -> Result<String, String> {
        let (setup, name, ops, whole) = if let Some((n, _)) = crate::setup_node(feature) {
            (n.id, n.name, Vec::new(), true)
        } else if let Some((n, d)) = crate::operation_node(feature) {
            (
                d.setup.ok_or("it is in no setup")?,
                n.name,
                vec![n.id],
                false,
            )
        } else {
            return Err("no setup or operation by that id".into());
        };
        let all = self.templates();
        let taken = |n: &str| {
            all.0
                .iter()
                .any(|k| k.file.is_some() && file_name(&k.template.name) == file_name(n))
        };
        let mut free = name.clone();
        let mut k = 2;
        while taken(&free) {
            free = format!("{name} ({k})");
            k += 1;
        }
        let t = self.template_of(&free, "", &setup, &ops, whole, &BTreeMap::new())?;
        let (file, _) = self.keep_template(&t, false)?;
        Ok(format!("Saved the template {} ({file}).", t.name))
    }

    /// Apply `t` to `target`: the setup made or chosen, its rules taken
    /// where asked, and the operations added after its own.
    pub(crate) fn apply_template(&mut self, t: &Template, target: Target) -> Result<Made, String> {
        let (setup_id, rules) = match target {
            Target::New { bodies, name } => (self.make_setup(bodies, name.as_deref())?, true),
            Target::Setup { id, rules } => (id, rules),
        };
        let (_, mut setup) = crate::setup_node(&setup_id).ok_or("no such setup")?;
        if rules && let Some(r) = &t.setup {
            take_rules(r, &mut setup);
            // A post of the user's named and not carried is the library's.
            if setup.post_profile.is_none() {
                setup.post_profile = self.settings.posts.get(&setup.post).cloned();
            }
        }
        let others: Vec<SetupData> = crate::setups()
            .into_iter()
            .filter(|(n, _)| n.id != setup_id)
            .map(|(_, d)| d)
            .collect();
        let document: Vec<&BTreeMap<String, ToolSpec>> = others.iter().map(|s| &s.tools).collect();
        let applied = apply(t, &setup_id, &mut setup, &document, &self.settings.tools);
        printcad_bench_sdk::host::set_feature_data(&setup_id, setup.to_value())?;
        // Each operation's name its own in the setup, as a job needs.
        let mut taken: Vec<String> = crate::operations_of(&setup_id)
            .into_iter()
            .map(|(n, _)| n.name)
            .collect();
        let mut operations = Vec::new();
        for (name, data) in &applied.operations {
            let mut free = name.clone();
            let mut n = 2;
            while taken.contains(&free) {
                free = format!("{name} ({n})");
                n += 1;
            }
            taken.push(free.clone());
            operations.push(printcad_bench_sdk::host::add_feature(
                crate::OPERATION_KIND,
                &free,
                None,
                data.to_value(),
            )?);
        }
        self.refresh(&setup_id, false);
        Ok(Made {
            setup: setup_id,
            operations,
            applied,
        })
    }
}

/// The setup panel's Templates group, for setup `name` with `ops` operations.
pub fn group(cam: &crate::Cam, name: &str, ops: usize) -> Widget {
    use crate::panel::{button, choice};
    let all = cam.templates();
    let mut rows = vec![Widget::Text {
        text: "A template keeps the setup's stock, work zero, orientation, post and \
               tolerance, and its operations with their tools and passes, but not what \
               is picked on the part."
            .into(),
        mono: false,
    }];
    rows.push(Widget::TextField {
        id: "template_name".into(),
        label: "Save as".into(),
        value: cam
            .template_ui
            .name
            .clone()
            .unwrap_or_else(|| name.to_string()),
    });
    rows.push(choice(
        "template_what",
        "Keep",
        vec![
            "The setup and its operations".into(),
            "Its operations only".into(),
        ],
        usize::from(cam.template_ui.alone),
    ));
    rows.push(button(
        "save_template",
        "Save as template",
        ButtonStyle::Secondary,
        ops > 0 || !cam.template_ui.alone,
    ));
    if !cam.templates_read() {
        rows.push(Widget::Text {
            text: "Reading the templates saved…".into(),
            mono: false,
        });
    }
    let names: Vec<String> = all
        .0
        .iter()
        .map(|k| match k.file {
            None => format!("{} (built in)", k.template.name),
            Some(_) => k.template.name.clone(),
        })
        .collect();
    let chosen = cam.template_ui.chosen.min(names.len().saturating_sub(1));
    rows.push(choice("template", "Template", names, chosen));
    if let Some(k) = all.0.get(chosen) {
        let t = &k.template;
        if !t.description.is_empty() {
            rows.push(Widget::Text {
                text: t.description.clone(),
                mono: false,
            });
        }
        rows.extend(t.operations.iter().map(|o| Widget::Value {
            label: o.name.clone(),
            value: format!(
                "{}, {}",
                crate::data::strategy_label(&o.data.strategy()),
                o.fill.words()
            ),
            mono: false,
        }));
        let rules = t.setup.is_some();
        if rules {
            rows.push(Widget::Toggle {
                id: "template_rules".into(),
                label: "Take its stock, work zero and post too".into(),
                on: cam.template_ui.rules.unwrap_or(ops == 0),
            });
        }
        rows.push(button(
            "apply_template",
            "Add its operations here",
            ButtonStyle::Primary,
            true,
        ));
        rows.push(button(
            "export_template",
            "Export…",
            ButtonStyle::Small,
            true,
        ));
        if k.file.is_some() {
            rows.push(button(
                "delete_template",
                "Delete this template",
                ButtonStyle::Small,
                true,
            ));
        }
    }
    rows.push(button(
        "import_template",
        "Import a template…",
        ButtonStyle::Small,
        true,
    ));
    rows.extend(all.1.iter().map(|bad| Widget::Note {
        kind: NoteKind::Warning,
        title: None,
        text: format!("Not read: {bad}"),
    }));
    Widget::Group {
        title: "Templates".into(),
        open: cam.template_ui.first,
        children: rows,
    }
}

/// What the Templates group's widgets do, in setup `id`'s panel; whether
/// the event was one of its own.
pub fn event(cam: &mut crate::Cam, id: &str, name: &str, event: &PanelEvent) -> bool {
    match event {
        PanelEvent::Text { id: t, value } if t == "template_name" => {
            cam.template_ui.name = Some(value.clone());
        }
        PanelEvent::Choice { id: c, index } if c == "template_what" => {
            cam.template_ui.alone = *index == 1;
        }
        PanelEvent::Choice { id: c, index } if c == "template" => {
            cam.template_ui.chosen = *index;
            cam.template_ui.rules = None;
        }
        PanelEvent::Toggle { id: t, on } if t == "template_rules" => {
            cam.template_ui.rules = Some(*on);
        }
        PanelEvent::Button { id: b } if b == "save_template" => {
            let name = cam
                .template_ui
                .name
                .clone()
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| name.to_string());
            let whole = !cam.template_ui.alone;
            let kept = cam
                .template_of(&name, "", id, &[], whole, &BTreeMap::new())
                .and_then(|t| Ok((cam.keep_template(&t, true)?, t)));
            cam.hint = Some(match kept {
                Ok(((file, replaced), t)) => {
                    let all = cam.templates();
                    if let Some(i) = all.0.iter().position(|k| k.file.as_deref() == Some(&file)) {
                        cam.template_ui.chosen = i;
                    }
                    let waiting: Vec<&str> = t
                        .operations
                        .iter()
                        .filter(|o| o.fill == Fill::Pick)
                        .map(|o| o.name.as_str())
                        .collect();
                    format!(
                        "{} template {} ({file}){}",
                        if replaced { "Replaced the" } else { "Saved as" },
                        t.name,
                        if waiting.is_empty() {
                            ".".to_string()
                        } else {
                            format!("; {} will need a pick.", waiting.join(", "))
                        }
                    )
                }
                Err(e) => format!("Not saved: {e}"),
            });
        }
        PanelEvent::Button { id: b } if b == "apply_template" => {
            let Some(k) = cam_chosen(cam) else {
                return true;
            };
            let ops = crate::operations_of(id).len();
            let rules = cam.template_ui.rules.unwrap_or(ops == 0);
            cam.hint = Some(
                match cam.apply_template(
                    &k.template,
                    Target::Setup {
                        id: id.to_string(),
                        rules,
                    },
                ) {
                    Ok(made) => {
                        printcad_bench_sdk::host::request(Request::JournalLabel {
                            label: "Apply template".into(),
                        });
                        cam.template_ui.first = false;
                        made.said(&k.template)
                    }
                    Err(e) => format!("Not applied: {e}"),
                },
            );
        }
        PanelEvent::Button { id: b } if b == "export_template" => {
            if let Some(k) = cam_chosen(cam) {
                printcad_bench_sdk::host::request(Request::SaveFile {
                    name: file_name(&k.template.name),
                    kind: "CAM template".into(),
                    extension: "json".into(),
                    contents: text_of(&k.template).into_bytes(),
                });
            }
        }
        PanelEvent::Button { id: b } if b == "import_template" => {
            printcad_bench_sdk::host::request(Request::OpenFile {
                id: OPEN_TEMPLATE.into(),
                kinds: vec![FileKind {
                    label: "CAM template (JSON)".into(),
                    extensions: vec!["json".into()],
                }],
                multiple: true,
            });
        }
        PanelEvent::Button { id: b } if b == "delete_template" => {
            if let Some(Kept {
                file: Some(file),
                template,
            }) = cam_chosen(cam)
            {
                cam.drop_template(&file);
                cam.template_ui.chosen = 0;
                cam.hint = Some(format!("Deleted the template {}.", template.name));
            }
        }
        _ => return false,
    }
    true
}

/// The template the group has chosen.
fn cam_chosen(cam: &crate::Cam) -> Option<Kept> {
    cam.templates().0.get(cam.template_ui.chosen).cloned()
}

/// Template files the user picked to import, kept beside the others: what
/// was kept, and what not, in words.
pub fn imported(cam: &crate::Cam, files: &[OpenedFile]) -> String {
    let mut said = Vec::new();
    for f in files {
        let read = std::str::from_utf8(&f.contents)
            .map_err(|_| "it is not text".to_string())
            .and_then(parse)
            .and_then(|t| Ok((cam.keep_template(&t, true)?, t)));
        said.push(match read {
            Ok(((_, replaced), t)) => format!(
                "{} {}.",
                if replaced { "Replaced" } else { "Imported" },
                t.name
            ),
            Err(e) => format!("{}: {e}.", f.name),
        });
    }
    said.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Pick;
    use camcraft_core::job::{HolePicking, ToolKind};

    fn flat(d: f64, number: u32) -> ToolSpec {
        ToolSpec {
            kind: ToolKind::Flat,
            diameter: d,
            number,
            ..ToolSpec::default()
        }
    }

    fn pick(body: &str) -> Pick {
        Pick {
            body: body.into(),
            at: [1.0, 2.0, 3.0],
            name: None,
            faces: None,
        }
    }

    /// A setup on body `a` of a fixed box, its work zero moved, posted to
    /// LinuxCNC, with an adaptive clearing on its silhouette, a pocket of
    /// faces picked and a drill of every hole as wide as one picked.
    fn source() -> (SetupData, Vec<(String, OperationData)>) {
        let mut setup = SetupData::new(vec!["a".into()], "linuxcnc", 0.01);
        setup.stock = crate::data::StockKind::Fixed;
        setup.size_x = 80.0;
        setup.offset_z = -1.5;
        setup.material = "6061 aluminium".into();
        setup.tools.insert("Flat 6".into(), flat(6.0, 4));
        setup.tools.insert(
            "Drill 5".into(),
            ToolSpec {
                kind: ToolKind::Drill,
                diameter: 5.0,
                number: 7,
                ..ToolSpec::default()
            },
        );
        let mut rough =
            OperationData::new(Some("S".into()), StrategySpec::named("adaptive").unwrap());
        rough.use_tool("Flat 6", &setup.tools["Flat 6"], None, 0.01);
        rough.stepdown = 2.5;
        rough.multiple_depths = true;
        let mut pocket =
            OperationData::new(Some("S".into()), StrategySpec::named("pocket").unwrap());
        pocket.use_tool("Flat 6", &setup.tools["Flat 6"], None, 0.01);
        pocket.operation.outline = Outline::Selection;
        pocket.faces.push(pick("a"));
        let mut drill = OperationData::new(Some("S".into()), StrategySpec::named("drill").unwrap());
        drill.use_tool("Drill 5", &setup.tools["Drill 5"], None, 0.01);
        drill.operation.holes = HolePicking {
            same_diameter: true,
            ..HolePicking::default()
        };
        drill.edges.push(pick("a"));
        let ops = vec![
            ("Rough".to_string(), rough),
            ("Pocket".to_string(), pocket),
            ("Holes".to_string(), drill),
        ];
        (setup, ops)
    }

    fn saved(setup: &SetupData, ops: &[(String, OperationData)]) -> Template {
        let ops: Vec<_> = ops
            .iter()
            .map(|(n, d)| {
                let hole = (n == "Holes").then_some(5.0);
                (n.clone(), d.clone(), fill_of(d, hole))
            })
            .collect();
        make("Plate", "a plate", setup, true, &ops)
    }

    /// Saved, written out, read back and applied to another body, it makes
    /// the same setup and operations, but for the body and the picks: the
    /// pocket waits for one, the drill takes every 5 mm hole.
    #[test]
    fn a_template_round_trips_onto_another_body() {
        let (setup, ops) = source();
        let t = saved(&setup, &ops);
        assert_eq!(
            t.operations.iter().map(|o| o.fill).collect::<Vec<_>>(),
            [Fill::Nothing, Fill::Pick, Fill::Holes(5.0)]
        );
        let t = parse(&text_of(&t)).unwrap();
        let mut other = SetupData::new(vec!["b".into()], "grbl", 0.05);
        take_rules(t.setup.as_ref().unwrap(), &mut other);
        let applied = apply(&t, "T", &mut other, &[], &BTreeMap::new());

        let mut expected = setup.clone();
        expected.bodies = vec!["b".into()];
        assert_eq!(other, expected, "the same setup, on b, with the same tools");
        assert_eq!(applied.waiting, ["Pocket"]);
        assert_eq!(applied.operations.len(), 3);
        for ((name, made), (was_name, was)) in applied.operations.iter().zip(&ops) {
            assert_eq!(name, was_name);
            let mut want = was.clone();
            want.setup = Some("T".into());
            want.faces.clear();
            want.edges.clear();
            if name == "Pocket" {
                want.needs_pick = Some("the faces, edges or sketches it machines".into());
            }
            if name == "Holes" {
                want.operation.holes.diameter = Some(5.0);
            }
            assert_eq!(made, &want, "{name}");
        }
        // The pocket says what it waits for, until it is given it.
        let pocket = &applied.operations[1].1;
        let why = pocket.problem(&other.tools).unwrap();
        assert!(why.starts_with("Needs a pick: the faces"), "{why}");
        let mut given = pocket.clone();
        given.faces.push(pick("b"));
        assert_eq!(given.problem(&other.tools), None);
        // The drill needs no pick to find its holes.
        assert_eq!(applied.operations[2].1.problem(&other.tools), None);
    }

    /// Operations alone carry no setup, and appended to a setup keep its
    /// rules and tools.
    #[test]
    fn operations_alone_keep_the_setup_they_join() {
        let (setup, ops) = source();
        let t = make(
            "Rough",
            "",
            &setup,
            false,
            &[(ops[0].0.clone(), ops[0].1.clone(), Fill::Nothing)],
        );
        assert!(t.setup.is_none());
        assert_eq!(t.tools.keys().collect::<Vec<_>>(), ["Flat 6"]);
        let mut target = SetupData::new(vec!["b".into()], "grbl", 0.05);
        target.tools.insert("My 6 mm".into(), flat(6.0, 4));
        let before = target.clone();
        let applied = apply(&t, "T", &mut target, &[], &BTreeMap::new());
        assert_eq!(target, before, "its own 6 mm flat, so nothing added");
        assert_eq!(
            applied.operations[0].1.operation.tool.as_deref(),
            Some("My 6 mm")
        );
        assert_eq!(applied.tools[0].found, Found::Setup);
    }

    /// A tool is matched by number and shape, then by shape, in the setup,
    /// the document and the library in turn; else the template's is added,
    /// under a free name and number.
    /// The kinds past the end mills match only their own shape: a tap its
    /// pitch and hand, a tapered mill its taper and flutes, a spot drill
    /// its point, 90° given or not.
    #[test]
    fn the_other_kinds_match_by_their_shape() {
        let tap = ToolSpec {
            kind: ToolKind::Tap,
            diameter: 6.0,
            pitch: Some(1.0),
            ..ToolSpec::default()
        };
        assert!(same_shape(&tap, &tap.clone()));
        let fine = ToolSpec {
            pitch: Some(0.75),
            ..tap.clone()
        };
        assert!(!same_shape(&tap, &fine));
        let left = ToolSpec {
            hand: camcraft_core::job::ThreadHand::Left,
            ..tap.clone()
        };
        assert!(!same_shape(&tap, &left));
        let tapered = ToolSpec {
            kind: ToolKind::Tapered,
            diameter: 1.0,
            corner: 0.5,
            taper_angle: Some(3.0),
            flute_length: Some(20.0),
            ..ToolSpec::default()
        };
        let longer = ToolSpec {
            flute_length: Some(30.0),
            ..tapered.clone()
        };
        let steeper = ToolSpec {
            taper_angle: Some(5.0),
            ..tapered.clone()
        };
        assert!(!same_shape(&tapered, &longer) && !same_shape(&tapered, &steeper));
        let spot = ToolSpec {
            kind: ToolKind::SpotDrill,
            diameter: 6.0,
            ..ToolSpec::default()
        };
        let ninety = ToolSpec {
            angle: Some(90.0),
            ..spot.clone()
        };
        assert!(same_shape(&spot, &ninety));
        // A flat end mill's flute length is not its shape.
        let flat = ToolSpec::default();
        let long = ToolSpec {
            flute_length: Some(30.0),
            ..flat.clone()
        };
        assert!(same_shape(&flat, &long));
    }

    #[test]
    fn tools_are_matched_by_number_kind_and_diameter() {
        let want = flat(6.0, 4);
        let setup = BTreeMap::from([("Six".to_string(), flat(6.0, 9))]);
        let doc = BTreeMap::from([("Doc six".to_string(), flat(6.0, 4))]);
        let library = BTreeMap::from([
            ("Lib six".to_string(), flat(6.0, 4)),
            ("Lib eight".to_string(), flat(8.0, 5)),
        ]);
        let (name, _, found) = match_tool(&want, &setup, &[&doc], &library).unwrap();
        assert_eq!(
            (name.as_str(), found),
            ("Doc six", Found::Document),
            "number first"
        );
        let (name, _, found) = match_tool(&want, &setup, &[], &BTreeMap::new()).unwrap();
        assert_eq!((name.as_str(), found), ("Six", Found::Setup), "then shape");
        let (name, _, found) = match_tool(&flat(8.0, 1), &setup, &[&doc], &library).unwrap();
        assert_eq!((name.as_str(), found), ("Lib eight", Found::Library));
        // A ball is not a flat, nor a 90° cone a 60° one.
        let ball = ToolSpec {
            kind: ToolKind::Ball,
            ..flat(6.0, 4)
        };
        assert!(match_tool(&ball, &setup, &[&doc], &library).is_none());
        let cone = |a: f64| ToolSpec {
            kind: ToolKind::Cone,
            angle: Some(a),
            ..flat(6.0, 2)
        };
        let vs = BTreeMap::from([("V60".to_string(), cone(60.0))]);
        assert!(match_tool(&cone(90.0), &vs, &[], &BTreeMap::new()).is_none());

        // None found: the template's own, renumbered past the setup's T4.
        let (setup_data, ops) = source();
        let t = make(
            "R",
            "",
            &setup_data,
            false,
            &[(ops[0].0.clone(), ops[0].1.clone(), Fill::Nothing)],
        );
        let mut target = SetupData::new(vec!["b".into()], "grbl", 0.05);
        target.tools.insert("Flat 6".into(), flat(10.0, 4));
        let applied = apply(&t, "T", &mut target, &[], &BTreeMap::new());
        assert_eq!(applied.tools[0].found, Found::Template);
        assert_eq!(applied.tools[0].name, "Flat 6 (2)");
        assert_eq!(target.tools["Flat 6 (2)"].diameter, 6.0);
        assert_ne!(target.tools["Flat 6 (2)"].number, 4);
        assert_eq!(
            applied.operations[0].1.operation.tool.as_deref(),
            Some("Flat 6 (2)")
        );
    }

    /// A file says its format and version: a newer one is refused, saying
    /// so, as is one that is no template; an older layout missing fields
    /// added since reads with their defaults.
    #[test]
    fn versions_are_checked() {
        let (setup, ops) = source();
        let text = text_of(&saved(&setup, &ops));
        let mut v: Value = serde_json::from_str(&text).unwrap();
        v["version"] = Value::from(VERSION + 1);
        let why = parse(&v.to_string()).unwrap_err();
        assert!(why.contains("newer CAM package"), "{why}");
        v["version"] = Value::from(0);
        assert!(parse(&v.to_string()).is_err());
        v["version"] = Value::from(VERSION);
        v["format"] = Value::from("something else");
        assert_eq!(
            parse(&v.to_string()).unwrap_err(),
            "it is not a CAM template"
        );
        assert!(parse("not json").unwrap_err().contains("not JSON"));
        // Without the fill or the description, or an operation's linking.
        let mut v: Value = serde_json::from_str(&text).unwrap();
        v.as_object_mut().unwrap().remove("description");
        for op in v["operations"].as_array_mut().unwrap() {
            op.as_object_mut().unwrap().remove("fill");
            op["data"].as_object_mut().unwrap().remove("linking");
        }
        let t = parse(&v.to_string()).unwrap();
        assert_eq!(t.description, "");
        assert!(t.operations.iter().all(|o| o.fill == Fill::Nothing));
    }

    /// A fill of nothing on an operation that reads picks still waits for
    /// them; the silhouette takes its place where it can.
    #[test]
    fn fills_say_what_waits() {
        let (setup, ops) = source();
        let pocket = &ops[1];
        for (fill, waits, outline) in [
            (Fill::Nothing, true, Outline::Selection),
            (Fill::Pick, true, Outline::Selection),
            (Fill::Silhouette, false, Outline::Silhouette),
        ] {
            let t = make(
                "P",
                "",
                &setup,
                false,
                &[(pocket.0.clone(), pocket.1.clone(), fill)],
            );
            let mut target = setup.clone();
            let applied = apply(&t, "T", &mut target, &[], &BTreeMap::new());
            assert_eq!(!applied.waiting.is_empty(), waits, "{fill:?}");
            assert_eq!(applied.operations[0].1.operation.outline, outline);
        }
        // A drill finding its holes by a diameter keeps that as its rule.
        let mut drill = ops[2].1.clone();
        drill.edges.clear();
        drill.operation.holes.diameter = Some(6.8);
        assert_eq!(fill_of(&drill, None), Fill::Holes(6.8));
        // An engraving has no silhouette to follow.
        let mut engrave = OperationData::new(None, StrategySpec::named("engrave").unwrap());
        engrave.sketches.push("s".into());
        let t = make(
            "E",
            "",
            &setup,
            false,
            &[("E".into(), engrave, Fill::Silhouette)],
        );
        let applied = apply(&t, "T", &mut setup.clone(), &[], &BTreeMap::new());
        assert_eq!(applied.waiting, ["E"]);
        assert_eq!(
            applied.operations[0].1.needs_pick.as_deref(),
            Some("the edges or sketches it follows")
        );
    }

    /// Templates are kept as files by name, as a job writes, reads and
    /// removes them, and a file that does not read is said, not dropped
    /// quietly.
    #[test]
    fn templates_are_kept_as_files() {
        let dir = std::env::temp_dir().join(format!("cam-templates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (setup, ops) = source();
        let t = saved(&setup, &ops);
        let file = file_name(&t.name);
        assert_eq!(file, "plate.json");
        let job = |w: &Work| {
            let input = serde_json::to_string(w).unwrap();
            work(&dir, &serde_json::from_str(&input).unwrap())
        };
        assert_eq!(job(&Work::Read).unwrap(), r#"{"templates":[],"bad":[]}"#);
        job(&Work::Write {
            file: file.clone(),
            text: text_of(&t),
        })
        .unwrap();
        std::fs::write(dir.join("broken.json"), "{").unwrap();
        let read: Read = serde_json::from_str(&job(&Work::Read).unwrap()).unwrap();
        assert_eq!(read.templates, [(file.clone(), t)]);
        assert!(
            read.bad[0].starts_with("broken.json: it is not JSON"),
            "{:?}",
            read.bad
        );
        job(&Work::Remove { file }).unwrap();
        assert!(read_dir(&dir).0.is_empty());
        assert!(
            job(&Work::Remove {
                file: "gone.json".into()
            })
            .is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            file_name("Face, rough & chamfer!"),
            "face-rough-chamfer.json"
        );
        assert_eq!(file_name("  "), "template.json");
    }

    /// The built-in templates read back as written, and their operations
    /// cut with their own tools, the chamfer waiting for its edges.
    #[test]
    fn the_built_in_templates_work() {
        let kept = built_in();
        assert_eq!(kept.len(), 2);
        for k in &kept {
            let t = parse(&text_of(&k.template)).unwrap();
            assert_eq!(t, k.template);
            let mut setup = SetupData::new(vec!["b".into()], "grbl", 0.05);
            take_rules(t.setup.as_ref().unwrap(), &mut setup);
            assert_eq!(setup.post, "grbl", "its post kept");
            let applied = apply(&t, "T", &mut setup, &[], &BTreeMap::new());
            for (name, op) in &applied.operations {
                if applied.waiting.contains(name) {
                    continue;
                }
                assert_eq!(op.problem(&setup.tools), None, "{}: {name}", t.name);
            }
        }
        let aluminium = &kept[0].template;
        assert_eq!(
            built_in()[0]
                .template
                .operations
                .iter()
                .map(|o| o.name.as_str())
                .collect::<Vec<_>>(),
            ["Face", "Adaptive rough", "Contour finish", "Chamfer"]
        );
        let mut setup = SetupData::new(vec!["b".into()], "grbl", 0.05);
        let applied = apply(aluminium, "T", &mut setup, &[], &BTreeMap::new());
        assert_eq!(applied.waiting, ["Chamfer"]);
    }
}
