//! What the document keeps of a setup and an operation, and the camcraft job
//! they make.
//!
//! A setup and an operation are features of the package's own kinds. Their
//! numbers are fields of their own, so a formula can drive any of them
//! (`/stepover`, `/margin_side`, `/heights/clearance`); the rest of an operation
//! is camcraft's own [`Operation`], kept whole. A job is made from them only to be sent to the helper: one
//! setup, its bodies as models, its stock, and its operations in the order the
//! document has them.

use std::collections::BTreeMap;

use camcraft_core::job::{
    Along, Boundary, BoxOf, Containment, Depths, FeatureRef, FixedBox, Height, HeightRef, Heights,
    Job, Leave, MachiningBoundary, ModelDef, Operation, Origin, Outline, Preset, Selection, Setup,
    StockDef, StockSpec, StockToLeave, StrategySpec, ToolSpec,
};
use camcraft_core::setup::frame::{Axis, Orientation};
use printcad_bench_sdk::serde_json;
use serde::{Deserialize, Serialize};

use crate::linking::LinkingData;
use crate::passes::PassesData;

/// Where a setup's stock comes from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StockKind {
    /// A box round the bodies, grown by margins.
    #[default]
    Around,
    /// A box of a fixed size, centred on the bodies, its top flush with theirs.
    Fixed,
}

/// Where a setup's work zero is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Zero {
    /// The stock's top, at its front-left corner: where Fusion's samples put it.
    #[default]
    StockCorner,
    /// The stock's top, in the middle.
    StockTopCentre,
    /// The stock's bottom, at its front-left corner.
    StockBottomCorner,
    /// The bodies' top, in the middle.
    ModelTopCentre,
    /// The document's origin.
    Document,
}

impl Zero {
    pub const ALL: [Zero; 5] = [
        Zero::StockCorner,
        Zero::StockTopCentre,
        Zero::StockBottomCorner,
        Zero::ModelTopCentre,
        Zero::Document,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Zero::StockCorner => "Stock top, front-left corner",
            Zero::StockTopCentre => "Stock top, centre",
            Zero::StockBottomCorner => "Stock bottom, front-left corner",
            Zero::ModelTopCentre => "Bodies' top, centre",
            Zero::Document => "Document origin",
        }
    }

    fn origin(self, offset: [f64; 3]) -> Origin {
        let on = |of, x, y, z| Origin::BoxPoint {
            of,
            x,
            y,
            z,
            offset,
        };
        match self {
            Zero::StockCorner => on(BoxOf::Stock, Along::Min, Along::Min, Along::Max),
            Zero::StockTopCentre => on(BoxOf::Stock, Along::Center, Along::Center, Along::Max),
            Zero::StockBottomCorner => on(BoxOf::Stock, Along::Min, Along::Min, Along::Min),
            Zero::ModelTopCentre => on(BoxOf::Model, Along::Center, Along::Center, Along::Max),
            Zero::Document => Origin::ModelOrigin { offset },
        }
    }
}

/// A setup: the bodies it machines, its stock, how the part sits and where its
/// work zero is, its post, and its own copies of the tools its operations use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetupData {
    /// The bodies, by id.
    #[serde(default)]
    pub bodies: Vec<String>,
    #[serde(default)]
    pub stock: StockKind,
    /// Extra material each side in X and Y, round the bodies.
    pub margin_side: f64,
    /// Extra material on top of the bodies.
    pub margin_top: f64,
    /// Extra material under the bodies.
    pub margin_below: f64,
    /// A fixed box's size.
    pub size_x: f64,
    pub size_y: f64,
    pub size_z: f64,
    /// The document direction the spindle points away from the part along.
    pub z_axis: Axis,
    /// The document direction that becomes the machine's X.
    pub x_axis: Axis,
    #[serde(default)]
    pub zero: Zero,
    /// The work zero moved along the setup's axes.
    pub offset_x: f64,
    pub offset_y: f64,
    pub offset_z: f64,
    /// The post's name: a built-in one's, or one of the user's.
    pub post: String,
    /// The user's machine profile it posts with, as camcraft's TOML: a copy,
    /// so it posts the same on a machine without it. `None` for a built-in
    /// post.
    #[serde(default)]
    pub post_profile: Option<String>,
    /// Work offset, 54 for G54.
    pub wcs: f64,
    /// Chordal tolerance, millimetres.
    pub tolerance: f64,
    /// The tools its operations cut with, by name: copies, never links to the
    /// library, so the setup cuts what was verified after the library changes.
    #[serde(default)]
    pub tools: BTreeMap<String, ToolSpec>,
}

impl SetupData {
    pub fn new(bodies: Vec<String>, post: &str, tolerance: f64) -> Self {
        Self {
            bodies,
            stock: StockKind::Around,
            margin_side: 2.0,
            margin_top: 2.0,
            margin_below: 0.0,
            size_x: 100.0,
            size_y: 100.0,
            size_z: 20.0,
            z_axis: Axis::PosZ,
            x_axis: Axis::PosX,
            zero: Zero::StockCorner,
            offset_x: 0.0,
            offset_y: 0.0,
            offset_z: 0.0,
            post: post.into(),
            post_profile: None,
            wcs: 54.0,
            tolerance,
            tools: BTreeMap::new(),
        }
    }

    pub fn from(data: &serde_json::Value) -> Option<Self> {
        serde_json::from_value(data.clone()).ok()
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn stock_spec(&self) -> StockSpec {
        match self.stock {
            StockKind::Around => StockSpec::AroundModels {
                models: Vec::new(),
                margin: [self.margin_side, self.margin_side, self.margin_top],
                below: self.margin_below,
            },
            StockKind::Fixed => {
                StockSpec::FixedBox(FixedBox::centred([self.size_x, self.size_y, self.size_z]))
            }
        }
    }

    pub fn orientation(&self) -> Orientation {
        Orientation {
            z: self.z_axis,
            x: self.x_axis,
        }
    }

    /// Make the X axis square to Z again after Z changed.
    pub fn square_axes(&mut self) {
        if self.orientation().check().is_err() {
            self.x_axis = Axis::ALL
                .into_iter()
                .find(|&x| Orientation { z: self.z_axis, x }.check().is_ok())
                .unwrap_or(Axis::PosX);
        }
    }

    /// The name a tool copied from the library takes in this setup, and the T
    /// number it gets: its own when no other tool here has it.
    pub fn adopt(&mut self, name: &str, tool: &ToolSpec) -> String {
        if self.tools.get(name) == Some(tool) {
            return name.to_string();
        }
        let mut free = name.to_string();
        let mut n = 2;
        while self.tools.contains_key(&free) {
            free = format!("{name} ({n})");
            n += 1;
        }
        let mut copy = tool.clone();
        let taken = |t: u32| self.tools.values().any(|o| o.number == t);
        if taken(copy.number) {
            copy.number = (1..).find(|&t| !taken(t)).unwrap_or(1);
        }
        self.tools.insert(free.clone(), copy);
        free
    }

    pub fn problem(&self) -> Option<String> {
        if self.bodies.is_empty() {
            return Some("Pick the bodies the setup machines.".into());
        }
        if !positive(self.tolerance) {
            return Some("The tolerance must be more than 0.".into());
        }
        if self.stock == StockKind::Fixed
            && ![self.size_x, self.size_y, self.size_z]
                .into_iter()
                .all(positive)
        {
            return Some("The stock's size must be more than 0 every way.".into());
        }
        if !(1.0..=99.0).contains(&self.wcs) {
            return Some("The work offset must be 54 to 59, or another the machine has.".into());
        }
        if let Err(e) = self.orientation().check() {
            return Some(e.to_string());
        }
        None
    }
}

/// A face or an edge picked on a body, by a point on it in document
/// coordinates: what camcraft finds it by again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pick {
    pub body: String,
    pub at: [f64; 3],
}

/// An operation's heights, Fusion's Heights tab: each an offset in millimetres
/// from what it is measured from. A top or a bottom with nothing to measure from
/// is the strategy's own, and its offset waits for one.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HeightsData {
    pub clearance: f64,
    pub clearance_from: HeightRef,
    pub retract: f64,
    pub retract_from: HeightRef,
    pub top: f64,
    #[serde(default)]
    pub top_from: Option<HeightRef>,
    pub bottom: f64,
    #[serde(default)]
    pub bottom_from: Option<HeightRef>,
}

impl Default for HeightsData {
    fn default() -> Self {
        Self::of(&Heights::default())
    }
}

impl HeightsData {
    /// camcraft's heights as numbers.
    pub fn of(h: &Heights) -> Self {
        Self {
            clearance: h.clearance.offset,
            clearance_from: h.clearance.reference,
            retract: h.retract.offset,
            retract_from: h.retract.reference,
            top: h.top.map_or(0.0, |t| t.offset),
            top_from: h.top.map(|t| t.reference),
            bottom: h.bottom.map_or(0.0, |b| b.offset),
            bottom_from: h.bottom.map(|b| b.reference),
        }
    }

    /// The numbers as camcraft's heights.
    pub fn heights(&self) -> Heights {
        let at = |reference, offset| Height { reference, offset };
        Heights {
            clearance: at(self.clearance_from, self.clearance),
            retract: at(self.retract_from, self.retract),
            top: self.top_from.map(|r| at(r, self.top)),
            bottom: self.bottom_from.map(|r| at(r, self.bottom)),
        }
    }
}

/// An operation: which setup it belongs to, camcraft's operation, and its
/// numbers, which are what it cuts with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationData {
    /// The setup feature, by id.
    #[serde(default)]
    pub setup: Option<String>,
    /// camcraft's operation; its numbers below win over its own.
    pub operation: Operation,
    pub stepover: f64,
    /// Multiple depths, where the strategy may have them.
    pub multiple_depths: bool,
    pub stepdown: f64,
    /// Cutting feed, mm/min.
    pub feed: f64,
    /// Plunge feed, mm/min.
    pub plunge: f64,
    /// Spindle speed, rev/min; the tool's when 0.
    pub rpm: f64,
    /// Stock to leave on the walls.
    pub radial: f64,
    /// Stock to leave on the floors.
    pub axial: f64,
    /// Pass direction, degrees from +X, for parallel and facing.
    pub angle: f64,
    /// A 3D boundary's additional offset.
    pub margin: f64,
    /// Where it travels and the levels it cuts between. A document from before
    /// they could be set has camcraft's defaults, which it cut with.
    #[serde(default)]
    pub heights: HeightsData,
    /// How it moves between cuts; camcraft's defaults for a document from
    /// before they could be set.
    #[serde(default)]
    pub linking: LinkingData,
    /// The rest of its passes: its strategy's own numbers, the deepest step,
    /// tabs and finishing passes; camcraft's defaults for a document from
    /// before they could be set.
    #[serde(default)]
    pub passes: PassesData,
    #[serde(default)]
    pub faces: Vec<Pick>,
    #[serde(default)]
    pub edges: Vec<Pick>,
}

impl OperationData {
    /// A new operation of `strategy` in `setup`, as camcraft's app starts one:
    /// Fusion's defaults, on the part's silhouette.
    pub fn new(setup: Option<String>, strategy: StrategySpec) -> Self {
        let mut op = Operation::new("", strategy);
        start_on_silhouette(&mut op);
        let mut data = Self {
            setup,
            operation: op,
            stepover: 0.0,
            multiple_depths: false,
            stepdown: 0.0,
            feed: 0.0,
            plunge: 0.0,
            rpm: 0.0,
            radial: 0.0,
            axial: 0.0,
            angle: 0.0,
            margin: 0.0,
            heights: HeightsData::default(),
            linking: LinkingData::default(),
            passes: PassesData::default(),
            faces: Vec::new(),
            edges: Vec::new(),
        };
        data.read_numbers();
        data
    }

    pub fn from(data: &serde_json::Value) -> Option<Self> {
        serde_json::from_value(data.clone()).ok()
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    pub fn strategy(&self) -> StrategySpec {
        self.operation.strategy
    }

    /// The operation's own numbers into the top-level fields.
    pub fn read_numbers(&mut self) {
        let op = &self.operation;
        self.stepover = op.passes.stepover;
        self.multiple_depths = op.passes.stepdown.is_some();
        self.stepdown = op.passes.stepdown.unwrap_or(self.stepdown.max(1.0));
        self.feed = op.passes.feed;
        self.plunge = op.linking.plunge_feed;
        self.rpm = op.passes.rpm.map_or(0.0, f64::from);
        self.radial = op.passes.stock_to_leave.radial;
        self.axial = op.passes.stock_to_leave.axial;
        self.angle = match op.strategy {
            StrategySpec::Parallel { angle } | StrategySpec::Face { angle, .. } => angle,
            _ => 0.0,
        };
        self.margin = op.boundary.margin;
        self.heights = HeightsData::of(&op.heights);
        self.linking = LinkingData::of(&op.linking);
        self.passes = PassesData::of(op);
    }

    /// The operation as it cuts: its numbers written in.
    pub fn operation(&self, name: &str) -> Operation {
        let mut op = self.operation.clone();
        op.name = name.to_string();
        let uses = op.strategy.uses();
        op.passes.stepover = self.stepover;
        op.passes.stepdown = match uses.depths {
            Depths::Required => Some(self.stepdown),
            Depths::Optional if self.multiple_depths => Some(self.stepdown),
            _ => None,
        };
        op.passes.feed = self.feed;
        self.linking.apply(&mut op.linking);
        op.linking.plunge_feed = self.plunge;
        op.passes.rpm = (self.rpm > 0.0).then(|| self.rpm.round() as u32);
        op.passes.stock_to_leave = match uses.stock_to_leave {
            Leave::None => StockToLeave::default(),
            Leave::Axial => StockToLeave {
                radial: 0.0,
                axial: self.axial,
            },
            Leave::Both => StockToLeave {
                radial: self.radial,
                axial: self.axial,
            },
        };
        match &mut op.strategy {
            StrategySpec::Parallel { angle } | StrategySpec::Face { angle, .. } => {
                *angle = self.angle
            }
            _ => {}
        }
        self.passes.apply(&mut op);
        op.boundary.margin = self.margin;
        op.heights = self.heights.heights();
        op
    }

    /// The operation as the job has it: [`Self::operation`] with its picks.
    /// camcraft finds a pick by its point when its number does not match, so
    /// any number does; printCAD's face names are not camcraft's numbers.
    pub fn picked(&self, name: &str, model_of: impl Fn(&str) -> Option<String>) -> Operation {
        let refs = |picks: &[Pick]| -> Vec<FeatureRef> {
            picks
                .iter()
                .filter_map(|p| {
                    Some(FeatureRef {
                        model: model_of(&p.body)?,
                        index: u32::MAX,
                        at: p.at,
                    })
                })
                .collect()
        };
        let mut op = self.operation(name);
        op.selection = Selection {
            faces: refs(&self.faces),
            edges: refs(&self.edges),
        };
        op
    }

    /// Whether its top or bottom may be measured from its picks: it has some,
    /// and is not a drill, whose holes each go to their own.
    pub fn can_measure_from_picks(&self) -> bool {
        self.picked("", |b| Some(b.to_string()))
            .can_measure_from_picks()
    }

    /// What its bottom is with none given: its picks for a 2D operation cut
    /// round them, else its strategy's own.
    pub fn bottom_unset(&self) -> Option<HeightRef> {
        let op = self.picked("", |b| Some(b.to_string()));
        let uses = op.strategy.uses();
        if uses.outline && !op.selection.is_empty() && op.outline == Outline::Selection {
            uses.bottom.map(|_| HeightRef::Selection)
        } else {
            uses.bottom
        }
    }

    /// Change the strategy, keeping the tool and the numbers that still apply.
    pub fn set_strategy(
        &mut self,
        strategy: StrategySpec,
        tool: Option<&ToolSpec>,
        tolerance: f64,
    ) {
        let mut op = Operation::new("", strategy);
        op.tool = self.operation.tool.clone();
        op.passes.feed = self.feed;
        op.passes.rpm = self.operation.passes.rpm;
        op.linking.plunge_feed = self.plunge;
        start_on_silhouette(&mut op);
        fit_stepover(&mut op, tool, None, tolerance);
        fit_channel(&mut op, tool);
        self.operation = op;
        self.read_numbers();
    }

    /// Cut with `name`, `tool`, from its preset `preset` if one is given, as
    /// camcraft's app does: the preset's feeds and speeds, Fusion's stepover
    /// for the tool, and room round the part for the first tool.
    pub fn use_tool(
        &mut self,
        name: &str,
        tool: &ToolSpec,
        preset: Option<&Preset>,
        tolerance: f64,
    ) {
        let mut op = self.operation(&self.operation.name);
        let first = op.tool.is_none();
        op.tool = Some(name.to_string());
        match preset {
            Some(p) => op.apply_preset(p),
            None => op.preset = None,
        }
        fit_stepover(&mut op, Some(tool), preset, tolerance);
        if first {
            fit_channel(&mut op, Some(tool));
        }
        self.operation = op;
        self.read_numbers();
    }

    pub fn problem(&self, tools: &BTreeMap<String, ToolSpec>) -> Option<String> {
        let Some(name) = &self.operation.tool else {
            return Some("Pick the tool it cuts with.".into());
        };
        let Some(tool) = tools.get(name) else {
            return Some(format!("The setup has no tool '{name}'."));
        };
        let uses = self.strategy().uses();
        if uses.stepover && !positive(self.stepover) {
            return Some("The stepover must be more than 0.".into());
        }
        if self
            .operation(&self.operation.name)
            .passes
            .stepdown
            .is_some()
            && !positive(self.stepdown)
        {
            return Some("The stepdown must be more than 0.".into());
        }
        if !(positive(self.feed) && positive(self.plunge)) {
            return Some("The feeds must be more than 0.".into());
        }
        if let Some(why) = self
            .picked("", |b| Some(b.to_string()))
            .cannot_measure_from_picks()
        {
            return Some(format!("The heights do not work: {why}."));
        }
        self.operation(&self.operation.name)
            .cannot_use(tool)
            .map(|why| format!("It cannot cut with {name}: {why}."))
    }
}

/// Whether `v` is a number more than zero (NaN is not).
fn positive(v: f64) -> bool {
    v > 0.0
}

/// The strategies a new operation may have, in the order the panel lists them,
/// with what the panel calls them.
pub const STRATEGIES: [(&str, &str); 13] = [
    ("adaptive", "Adaptive clearing (3D roughing)"),
    ("pocket-clearing", "Pocket clearing (3D roughing)"),
    ("parallel", "Parallel (3D finishing)"),
    ("scallop", "Scallop (3D finishing)"),
    ("contour", "Contour (3D finishing)"),
    ("flat", "Flat (3D finishing)"),
    ("spiral", "Spiral (3D finishing)"),
    ("radial", "Radial (3D finishing)"),
    ("morphed-spiral", "Morphed spiral (3D finishing)"),
    ("profile", "2D profile"),
    ("pocket", "2D pocket"),
    ("face", "Facing"),
    ("drill", "Drilling"),
];

/// The panel's word for `strategy`.
pub fn strategy_label(strategy: &StrategySpec) -> &'static str {
    STRATEGIES
        .iter()
        .find(|(name, _)| *name == strategy.name())
        .map_or("Operation", |(_, label)| label)
}

/// A short word for `strategy`, for a new operation's name.
pub fn strategy_word(strategy: &StrategySpec) -> &'static str {
    match strategy {
        StrategySpec::Adaptive { .. } => "Adaptive",
        StrategySpec::PocketClearing { .. } => "Pocket clearing",
        StrategySpec::Parallel { .. } => "Parallel",
        StrategySpec::Scallop { .. } => "Scallop",
        StrategySpec::Contour { .. } => "Contour",
        StrategySpec::Flat { .. } => "Flat",
        StrategySpec::Spiral { .. } => "Spiral",
        StrategySpec::Radial { .. } => "Radial",
        StrategySpec::MorphedSpiral { .. } => "Morphed spiral",
        StrategySpec::Profile { .. } => "Profile",
        StrategySpec::Pocket { .. } => "2D pocket",
        StrategySpec::Face { .. } => "Face",
        StrategySpec::Drill { .. } => "Drill",
    }
}

/// A 3D strategy's boundary on the part's silhouette, as camcraft's app starts
/// one, the tool's centre on it for finishing and outside it for roughing.
fn start_on_silhouette(op: &mut Operation) {
    let containment = match op.strategy {
        StrategySpec::Parallel { .. }
        | StrategySpec::Scallop { .. }
        | StrategySpec::Spiral { .. }
        | StrategySpec::Radial { .. }
        | StrategySpec::MorphedSpiral { .. } => Containment::Center,
        StrategySpec::PocketClearing { .. }
        | StrategySpec::Adaptive { .. }
        | StrategySpec::Contour { .. }
        | StrategySpec::Flat { .. } => Containment::Outside,
        StrategySpec::Profile { .. }
        | StrategySpec::Pocket { .. }
        | StrategySpec::Face { .. }
        | StrategySpec::Drill { .. } => return,
    };
    op.boundary.kind = Boundary::Silhouette;
    op.boundary.containment = containment;
}

/// Whether `b` leaves the tool no room round the part: a boundary round it that
/// the tool reaches in from outside, with no additional offset.
fn needs_room(b: &MachiningBoundary) -> bool {
    matches!(b.kind, Boundary::BoundingBox | Boundary::Silhouette)
        && b.containment == Containment::Outside
        && b.margin == 0.0
}

/// Room for `tool` round the part: a boundary that [`needs_room`] offset by the
/// tool's diameter, a channel two tools wide.
fn fit_channel(op: &mut Operation, tool: Option<&ToolSpec>) {
    if op.strategy.uses().boundary
        && needs_room(&op.boundary)
        && let Some(t) = tool
    {
        op.boundary.margin = t.diameter;
    }
}

/// Fusion's stepover for the strategy with `tool`, unless `preset` set one.
fn fit_stepover(
    op: &mut Operation,
    tool: Option<&ToolSpec>,
    preset: Option<&Preset>,
    tolerance: f64,
) {
    if preset.is_some_and(|p| p.stepover.is_some()) && !op.strategy.uses().load {
        return;
    }
    if let Some(over) = tool.and_then(|t| op.default_stepover(t, tolerance)) {
        op.passes.stepover = over;
    }
}

/// A body of the document as one of a job's models.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyModel {
    /// The body's id.
    pub body: String,
    /// Its model's name in the job.
    pub name: String,
}

/// The models a setup's bodies make: each body once, named after it, the names
/// made unique.
pub fn models_of(setup: &SetupData, name_of: impl Fn(&str) -> Option<String>) -> Vec<BodyModel> {
    let mut out: Vec<BodyModel> = Vec::new();
    for body in &setup.bodies {
        if out.iter().any(|m| &m.body == body) {
            continue;
        }
        let base = name_of(body).unwrap_or_else(|| "Body".into());
        let mut name = base.clone();
        let mut n = 2;
        while out.iter().any(|m| m.name == name) {
            name = format!("{base} ({n})");
            n += 1;
        }
        out.push(BodyModel {
            body: body.clone(),
            name,
        });
    }
    out
}

/// One setup's camcraft job: its bodies as `models`, its stock, and
/// `operations` (each with its name) in order.
pub fn job(
    name: &str,
    setup: &SetupData,
    models: &[BodyModel],
    operations: &[(String, &OperationData)],
) -> Job {
    let model_of = |body: &str| {
        models
            .iter()
            .find(|m| m.body == body)
            .map(|m| m.name.clone())
    };
    let mut camcraft = Setup::new(name);
    camcraft.models = models.iter().map(|m| m.name.clone()).collect();
    camcraft.stocks = vec!["stock".into()];
    camcraft.wcs = setup.wcs.round().clamp(1.0, 99.0) as u8;
    camcraft.post = setup.post.clone();
    camcraft.orientation = setup.orientation();
    camcraft.origin = setup
        .zero
        .origin([setup.offset_x, setup.offset_y, setup.offset_z]);
    camcraft.operations = operations
        .iter()
        .map(|(name, data)| data.picked(name, model_of))
        .collect();
    Job {
        tolerance: setup.tolerance,
        tools: setup.tools.clone(),
        models: models
            .iter()
            .map(|m| ModelDef {
                name: m.name.clone(),
                path: format!("printcad:{}", m.body),
                units: None,
                transform: Default::default(),
            })
            .collect(),
        stocks: vec![StockDef {
            name: "stock".into(),
            spec: setup.stock_spec(),
        }],
        setups: vec![camcraft],
        ..Job::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_survives_the_document() {
        let mut setup = SetupData::new(vec!["b1".into()], "grbl", 0.05);
        let tool = ToolSpec {
            diameter: 6.0,
            ..ToolSpec::default()
        };
        let name = setup.adopt("Flat 6", &tool);
        let mut op =
            OperationData::new(Some("s1".into()), StrategySpec::named("adaptive").unwrap());
        op.use_tool(&name, &tool, None, 0.05);
        let back = SetupData::from(&setup.to_value()).unwrap();
        assert_eq!(back, setup);
        let back = OperationData::from(&op.to_value()).unwrap();
        assert_eq!(back, op);
        assert!(
            op.problem(&setup.tools).is_none(),
            "{:?}",
            op.problem(&setup.tools)
        );
    }

    #[test]
    fn the_numbers_are_what_it_cuts_with() {
        let mut op = OperationData::new(None, StrategySpec::named("pocket").unwrap());
        op.stepover = 1.25;
        op.multiple_depths = true;
        op.stepdown = 2.5;
        let cut = op.operation("p");
        assert_eq!(cut.passes.stepover, 1.25);
        assert_eq!(cut.passes.stepdown, Some(2.5));
        op.multiple_depths = false;
        assert_eq!(op.operation("p").passes.stepdown, None);
    }

    #[test]
    fn a_job_reads_back_as_camcraft_writes_it() {
        let mut setup = SetupData::new(vec!["b1".into(), "b2".into()], "grbl", 0.05);
        let tool = ToolSpec {
            diameter: 6.0,
            ..ToolSpec::default()
        };
        let name = setup.adopt("Flat 6", &tool);
        let mut op = OperationData::new(None, StrategySpec::named("pocket-clearing").unwrap());
        op.use_tool(&name, &tool, None, 0.05);
        op.faces.push(Pick {
            body: "b2".into(),
            at: [1.0, 2.0, 3.0],
        });
        let models = models_of(&setup, |_| Some("Part".into()));
        assert_eq!(models[1].name, "Part (2)");
        let job = job("Setup 1", &setup, &models, &[("Rough".into(), &op)]);
        let text = camcraft_session::job_text(&job).unwrap();
        let back = camcraft_session::parse_job(&text, "test").unwrap();
        assert_eq!(back, job);
        assert_eq!(
            back.setups[0].operations[0].selection.faces[0].model,
            "Part (2)"
        );
    }

    #[test]
    fn a_tool_adopted_twice_is_one_copy() {
        let mut setup = SetupData::new(Vec::new(), "grbl", 0.05);
        let tool = ToolSpec::default();
        assert_eq!(setup.adopt("A", &tool), "A");
        assert_eq!(setup.adopt("A", &tool), "A");
        let other = ToolSpec {
            diameter: 3.0,
            ..ToolSpec::default()
        };
        assert_eq!(setup.adopt("A", &other), "A (2)");
        assert_ne!(setup.tools["A"].number, setup.tools["A (2)"].number);
    }

    #[test]
    fn the_heights_are_what_it_cuts_between() {
        let mut op = OperationData::new(None, StrategySpec::named("pocket-clearing").unwrap());
        op.heights.clearance_from = HeightRef::ModelTop;
        op.heights.clearance = 7.0;
        op.heights.top_from = Some(HeightRef::StockTop);
        op.heights.top = -1.0;
        op.heights.bottom_from = Some(HeightRef::Absolute);
        op.heights.bottom = -5.0;
        let cut = op.operation("p");
        assert_eq!(
            cut.heights.clearance,
            Height {
                reference: HeightRef::ModelTop,
                offset: 7.0
            }
        );
        assert_eq!(cut.heights.top.unwrap().offset, -1.0);
        assert_eq!(cut.heights.bottom.unwrap().reference, HeightRef::Absolute);
        // Not given, the strategy's own; the offset waits.
        op.heights.top_from = None;
        assert_eq!(op.operation("p").heights.top, None);
        assert_eq!(OperationData::from(&op.to_value()).unwrap(), op);
    }

    #[test]
    fn an_operation_from_before_heights_cuts_with_camcrafts() {
        let op = OperationData::new(None, StrategySpec::named("adaptive").unwrap());
        let mut value = op.to_value();
        value.as_object_mut().unwrap().remove("heights");
        let old = OperationData::from(&value).unwrap();
        assert_eq!(old.operation("a").heights, Heights::default());
    }

    #[test]
    fn a_height_from_the_picks_needs_some() {
        let tools = BTreeMap::from([(
            "Flat 6".to_string(),
            ToolSpec {
                diameter: 6.0,
                ..ToolSpec::default()
            },
        )]);
        let mut op = OperationData::new(None, StrategySpec::named("pocket").unwrap());
        op.use_tool("Flat 6", &tools["Flat 6"], None, 0.05);
        assert!(!op.can_measure_from_picks());
        op.heights.bottom_from = Some(HeightRef::Selection);
        assert!(op.problem(&tools).unwrap().contains("nothing is picked"));
        op.faces.push(Pick {
            body: "b".into(),
            at: [0.0; 3],
        });
        assert!(op.can_measure_from_picks());
        assert_eq!(op.problem(&tools), None);
        // A drill's holes each go to their own bottom.
        let mut drill = OperationData::new(None, StrategySpec::named("drill").unwrap());
        drill.edges.push(Pick {
            body: "b".into(),
            at: [0.0; 3],
        });
        assert!(!drill.can_measure_from_picks());
    }
}
