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
    Along, BarAxis, Boundary, BoxOf, Containment, Curve, Depths, FeatureRef, FixedBox,
    FixedCylinder, Height, HeightRef, Heights, Job, Leave, MachiningBoundary, ModelDef, Operation,
    Origin, Outline, Place, PointRef, Preset, Programs, Selection, Setup, Snap, StockDef,
    StockSpec, StockToLeave, StrategySpec, ToolSpec, ZCorrection,
};
use camcraft_core::setup::frame::{Axis, Direction, Orientation};
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
    /// A round bar: round the bodies, or of a fixed size.
    Cylinder,
    /// What an earlier setup leaves: the same material, after its last
    /// operation, turned as this setup sees it, as Fusion's "Continue from
    /// previous setup".
    Earlier,
}

/// Which way a cylinder of stock lies, as the part sits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CylinderAxis {
    /// Up the tool: a disc, or a bar stood on end.
    #[default]
    Tool,
    /// Lying along the machine's X.
    X,
    /// Lying along the machine's Y.
    Y,
}

impl CylinderAxis {
    pub const ALL: [CylinderAxis; 3] = [CylinderAxis::Tool, CylinderAxis::X, CylinderAxis::Y];

    pub fn label(self) -> &'static str {
        match self {
            CylinderAxis::Tool => "Along the tool axis",
            CylinderAxis::X => "Along machine X",
            CylinderAxis::Y => "Along machine Y",
        }
    }
}

/// Where a setup's tool axis comes from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolAxis {
    /// Along one of the document's axes, `z_axis`: the part square on the
    /// table.
    #[default]
    Square,
    /// Out of a flat face picked on the part, along its normal.
    Face,
    /// Along a straight edge picked on the part.
    Edge,
    /// The document's +Z tilted `tilt_x` about its X, then `tilt_y` about its
    /// Y.
    Tilt,
    /// Along a direction typed in, `axis_dx`, `axis_dy`, `axis_dz`.
    Direction,
}

impl ToolAxis {
    /// The ways that tilt the part, after the six axes, as the panel lists
    /// them.
    pub const TILTED: [ToolAxis; 4] = [
        ToolAxis::Face,
        ToolAxis::Edge,
        ToolAxis::Tilt,
        ToolAxis::Direction,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ToolAxis::Square => "One of the document's axes",
            ToolAxis::Face => "Out of a face picked on the part",
            ToolAxis::Edge => "Along an edge picked on the part",
            ToolAxis::Tilt => "Tilted by angles",
            ToolAxis::Direction => "Along a direction",
        }
    }
}

/// Where a setup's machine X comes from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MachineX {
    /// Along one of the document's axes, `x_axis`, made square to a tilted
    /// tool axis.
    #[default]
    Axis,
    /// Along a straight edge picked on the part, made square to the tool
    /// axis.
    Edge,
}

/// A face or an edge an axis is taken from, by a point on it in document
/// coordinates, as camcraft finds it again, and which way it points as the
/// view saw it: a face's outward normal, an edge's direction there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AxisPick {
    pub body: String,
    pub at: [f64; 3],
    pub toward: [f64; 3],
    /// The other way.
    #[serde(default)]
    pub reverse: bool,
}

impl AxisPick {
    /// Which way it points, as the view saw it, reversed when it is.
    pub fn up(&self) -> [f64; 3] {
        let s = if self.reverse { -1.0 } else { 1.0 };
        self.toward.map(|v| v * s)
    }
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
    /// A point picked on a body: a corner, or a round edge's centre.
    Picked,
}

/// A work zero picked on a body, by its point in document coordinates: what
/// camcraft finds it by again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZeroPoint {
    pub body: String,
    /// A corner, or a round edge's centre.
    pub snap: Snap,
    pub at: [f64; 3],
}

impl Zero {
    pub const ALL: [Zero; 6] = [
        Zero::StockCorner,
        Zero::StockTopCentre,
        Zero::StockBottomCorner,
        Zero::ModelTopCentre,
        Zero::Document,
        Zero::Picked,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Zero::StockCorner => "Stock top, front-left corner",
            Zero::StockTopCentre => "Stock top, centre",
            Zero::StockBottomCorner => "Stock bottom, front-left corner",
            Zero::ModelTopCentre => "Bodies' top, centre",
            Zero::Document => "Document origin",
            Zero::Picked => "A point picked on the part",
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
            // Until one is picked, where a new setup's is.
            Zero::Picked => Zero::StockCorner.origin(offset),
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
    /// The setup it carries on from, by id, for [`StockKind::Earlier`]: one
    /// before it in the document; `None` for the one just before it.
    #[serde(default)]
    pub stock_from: Option<String>,
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
    /// Which way a cylinder lies.
    #[serde(default)]
    pub cylinder_axis: CylinderAxis,
    /// Whether a cylinder is of a fixed size, rather than round the bodies.
    #[serde(default)]
    pub cylinder_fixed: bool,
    /// Extra material all round the bodies, and at each end, of a cylinder
    /// round them.
    #[serde(default = "two")]
    pub margin_radial: f64,
    #[serde(default = "two")]
    pub margin_ends: f64,
    /// A fixed cylinder's size.
    #[serde(default = "hundred")]
    pub diameter: f64,
    #[serde(default = "hundred")]
    pub length: f64,
    /// What the stock is, in words, for the setup sheet: `6061 aluminium`.
    #[serde(default)]
    pub material: String,
    /// The document direction the spindle points away from the part along.
    pub z_axis: Axis,
    /// The document direction that becomes the machine's X.
    pub x_axis: Axis,
    /// Where the tool axis comes from: `z_axis`, or tilted for 3+2
    /// machining.
    #[serde(default)]
    pub tool_axis: ToolAxis,
    /// The face or edge picked for [`ToolAxis::Face`] or [`ToolAxis::Edge`].
    #[serde(default)]
    pub axis_pick: Option<AxisPick>,
    /// [`ToolAxis::Tilt`]'s angles, degrees: about the document's X, then
    /// its Y.
    #[serde(default)]
    pub tilt_x: f64,
    #[serde(default)]
    pub tilt_y: f64,
    /// [`ToolAxis::Direction`]'s direction.
    #[serde(default)]
    pub axis_dx: f64,
    #[serde(default)]
    pub axis_dy: f64,
    #[serde(default = "one")]
    pub axis_dz: f64,
    /// Where machine X comes from.
    #[serde(default)]
    pub machine_x: MachineX,
    /// The edge picked for [`MachineX::Edge`].
    #[serde(default)]
    pub x_pick: Option<AxisPick>,
    #[serde(default)]
    pub zero: Zero,
    /// The point picked for [`Zero::Picked`].
    #[serde(default)]
    pub zero_point: Option<ZeroPoint>,
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
    /// Work offset, 54 for G54; from 60, the post's extended ones in its
    /// order (60 is LinuxCNC's G59.1).
    pub wcs: f64,
    /// Whether where the work zero is on the machine is given, in
    /// `wcs_x`, `wcs_y` and `wcs_z`: what the work offset holds.
    #[serde(default)]
    pub wcs_known: bool,
    /// Where the work zero is in machine coordinates, millimetres along the
    /// setup's axes: Z the gauge line's when the post takes tool length
    /// offsets, else the tool's tip's.
    #[serde(default)]
    pub wcs_x: f64,
    #[serde(default)]
    pub wcs_y: f64,
    #[serde(default)]
    pub wcs_z: f64,
    /// Whether it is posted as one program, or one an operation.
    #[serde(default)]
    pub programs: Programs,
    /// Its program's number, for a control that files programs by number;
    /// each program of a setup posted one an operation takes the next.
    #[serde(default = "program_number")]
    pub program_number: f64,
    /// Chordal tolerance, millimetres.
    pub tolerance: f64,
    /// The tools its operations cut with, by name: copies, never links to the
    /// library, so the setup cuts what was verified after the library changes.
    #[serde(default)]
    pub tools: BTreeMap<String, ToolSpec>,
    /// Whether it runs. One turned off keeps its operations, but none of
    /// them is worked out, simulated, checked or posted.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Whether every cut's depth is corrected to probe results.
    #[serde(default)]
    pub probe_correct: bool,
    /// The probe results, each x, y and z in the program's coordinates and
    /// units, as read from the file picked: kept in the document, so it
    /// cuts the same wherever it is opened.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub probe_points: Vec<[f64; 3]>,
    /// The file they came from, by name; with no points, a path the helper
    /// reads them from each run, as a script gives it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub probe_file: String,
    /// The longest piece a corrected move is split into, millimetres.
    #[serde(default = "one")]
    pub probe_segment: f64,
    /// Whether it is machined on a rotary turning its cylinder, lying
    /// along X or Y, about its axis: everything planned on it unrolled.
    #[serde(default)]
    pub wrap: bool,
    /// The diameter a flat path is wrapped round at, millimetres; the
    /// stock's when not above zero.
    #[serde(default)]
    pub wrap_diameter: f64,
}

impl SetupData {
    pub fn new(bodies: Vec<String>, post: &str, tolerance: f64) -> Self {
        Self {
            bodies,
            stock: StockKind::Around,
            stock_from: None,
            margin_side: 2.0,
            margin_top: 2.0,
            margin_below: 0.0,
            size_x: 100.0,
            size_y: 100.0,
            size_z: 20.0,
            cylinder_axis: CylinderAxis::Tool,
            cylinder_fixed: false,
            margin_radial: two(),
            margin_ends: two(),
            diameter: hundred(),
            length: hundred(),
            material: String::new(),
            z_axis: Axis::PosZ,
            x_axis: Axis::PosX,
            tool_axis: ToolAxis::Square,
            axis_pick: None,
            tilt_x: 0.0,
            tilt_y: 0.0,
            axis_dx: 0.0,
            axis_dy: 0.0,
            axis_dz: 1.0,
            machine_x: MachineX::Axis,
            x_pick: None,
            zero: Zero::StockCorner,
            zero_point: None,
            offset_x: 0.0,
            offset_y: 0.0,
            offset_z: 0.0,
            post: post.into(),
            post_profile: None,
            wcs: 54.0,
            wcs_known: false,
            wcs_x: 0.0,
            wcs_y: 0.0,
            wcs_z: 0.0,
            programs: Programs::Setup,
            program_number: program_number(),
            tolerance,
            tools: BTreeMap::new(),
            enabled: true,
            probe_correct: false,
            probe_points: Vec::new(),
            probe_file: String::new(),
            probe_segment: one(),
            wrap: false,
            wrap_diameter: 0.0,
        }
    }

    /// Whether it is on a rotary, its cylinder lying along X or Y.
    pub fn wraps(&self) -> bool {
        self.wrap && self.stock == StockKind::Cylinder && self.cylinder_axis != CylinderAxis::Tool
    }

    /// The probe results in `text`, from the file `name`, as its points:
    /// refused, saying why, where they make no map.
    pub fn load_probe_results(&mut self, name: &str, text: &str) -> Result<String, String> {
        let points = camcraft_core::probe::parse(text).map_err(|e| e.to_string())?;
        let map =
            camcraft_core::probe::ProbeMap::from_points(&points, 1.0).map_err(|e| e.to_string())?;
        let (columns, rows) = map.size();
        self.probe_points = points;
        self.probe_file = name.to_string();
        self.probe_correct = true;
        Ok(format!(
            "Read {} point(s) from {name}: {columns} x {rows}.",
            columns * rows
        ))
    }

    /// The units its post writes, which its probe results are in.
    fn program_units(&self) -> camcraft_core::job::LengthUnit {
        match self.profile().map(|p| p.units) {
            Some(camcraft_post::profile::ProgramUnits::Inch) => {
                camcraft_core::job::LengthUnit::Inch
            }
            _ => camcraft_core::job::LengthUnit::Mm,
        }
    }

    /// Its depths corrected to its probe results, as camcraft has it; `None`
    /// when they are not.
    pub fn z_correction(&self) -> Option<ZCorrection> {
        self.probe_correct.then(|| ZCorrection {
            points: self.probe_points.clone(),
            file: (!self.probe_file.is_empty()).then(|| self.probe_file.clone()),
            units: self.program_units(),
            segment: self.probe_segment,
        })
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
            StockKind::Cylinder if self.cylinder_fixed => {
                StockSpec::FixedCylinder(FixedCylinder {
                    models: Vec::new(),
                    axis: self.cylinder_model_axis(),
                    diameter: self.diameter,
                    length: self.length,
                    // Up the tool, its top flush with the bodies', as a
                    // fixed box's; lying, centred on them. A tilted setup
                    // lays it out in its own axes, up the tool its Z.
                    place: match self.cylinder_axis {
                        CylinderAxis::Tool if !self.square() || is_positive(self.z_axis) => {
                            Place::Max { offset: 0.0 }
                        }
                        CylinderAxis::Tool => Place::Min { offset: 0.0 },
                        _ => Place::Center {},
                    },
                })
            }
            StockKind::Cylinder => StockSpec::CylinderAroundModels {
                models: Vec::new(),
                axis: self.cylinder_model_axis(),
                margin: self.margin_radial,
                ends: self.margin_ends,
            },
            // The helper starts from what the earlier setup left, which its
            // request carries, and lays the setup out round that; the job
            // still names a stock, the bodies' box.
            StockKind::Earlier => StockSpec::AroundModels {
                models: Vec::new(),
                margin: [0.0; 3],
                below: 0.0,
            },
        }
    }

    /// The axis a cylinder of stock lies along: the setup's axis it is set
    /// to lie along, as the part sits, as a document axis; for a tilted
    /// setup, which lays its stock out in its own axes, that axis itself.
    fn cylinder_model_axis(&self) -> BarAxis {
        if !self.square() {
            return match self.cylinder_axis {
                CylinderAxis::Tool => BarAxis::Z,
                CylinderAxis::X => BarAxis::X,
                CylinderAxis::Y => BarAxis::Y,
            };
        }
        let y = self.z_axis.vector().cross(self.x_axis.vector());
        let along = match self.cylinder_axis {
            CylinderAxis::Tool => self.z_axis.vector(),
            CylinderAxis::X => self.x_axis.vector(),
            CylinderAxis::Y => y,
        };
        if along.x != 0.0 {
            BarAxis::X
        } else if along.y != 0.0 {
            BarAxis::Y
        } else {
            BarAxis::Z
        }
    }

    /// Whether the part sits square on the table, its tool axis and machine
    /// X both along the document's axes; otherwise it is tilted.
    pub fn square(&self) -> bool {
        self.tool_axis == ToolAxis::Square && self.machine_x == MachineX::Axis
    }

    /// The tool axis [`ToolAxis::Tilt`] gives: the document's +Z turned
    /// `tilt_x` about its X, then `tilt_y` about its Y. What rounds to
    /// nothing is nothing, so no tilt is +Z exactly.
    pub fn tilted(&self) -> [f64; 3] {
        let (sa, ca) = self.tilt_x.to_radians().sin_cos();
        let (sb, cb) = self.tilt_y.to_radians().sin_cos();
        [ca * sb, -sa, ca * cb].map(|v| if v.abs() < 1e-12 { 0.0 } else { v })
    }

    /// Which way the tool axis points, as near as the panel knows: a picked
    /// one as the view saw it. The helper finds it exactly.
    pub fn up(&self) -> [f64; 3] {
        let unit = |v: [f64; 3]| {
            let l = v.iter().map(|c| c * c).sum::<f64>().sqrt();
            if l > 0.0 && l.is_finite() {
                v.map(|c| c / l)
            } else {
                [0.0, 0.0, 1.0]
            }
        };
        let axis = self.z_axis.vector().to_array();
        match self.tool_axis {
            ToolAxis::Square => axis,
            ToolAxis::Face | ToolAxis::Edge => {
                self.axis_pick.as_ref().map_or(axis, |p| unit(p.up()))
            }
            ToolAxis::Tilt => self.tilted(),
            ToolAxis::Direction => unit([self.axis_dx, self.axis_dy, self.axis_dz]),
        }
    }

    /// The document axis machine X is taken from for a tilted tool axis:
    /// `x_axis`, or, where the tool points nearly along it, the first of X
    /// and Y that it does not, so X can be made square to the tool.
    fn level_x(&self) -> Axis {
        let up = self.up();
        let off = |a: Axis| {
            let v = a.vector();
            (v.x * up[0] + v.y * up[1] + v.z * up[2]).abs() < 0.999
        };
        [self.x_axis, Axis::PosX, Axis::PosY]
            .into_iter()
            .find(|&a| off(a))
            .unwrap_or(Axis::PosZ)
    }

    /// How the part sits, as camcraft has it: picks on the bodies by the
    /// models `model_of` names them as.
    pub fn orientation(&self, model_of: impl Fn(&str) -> Option<String>) -> Orientation {
        let pick = |p: &AxisPick| FeatureRef {
            model: model_of(&p.body).unwrap_or_else(|| p.body.clone()),
            // Found again by its point, as a pick is.
            index: u32::MAX,
            at: p.at,
        };
        let edge = |p: &AxisPick| Direction::Edge {
            edge: pick(p),
            toward: Some(p.toward),
            reverse: p.reverse,
        };
        let z = match (self.tool_axis, &self.axis_pick) {
            (ToolAxis::Face, Some(p)) => Direction::Face {
                face: pick(p),
                reverse: p.reverse,
            },
            (ToolAxis::Edge, Some(p)) => edge(p),
            (ToolAxis::Tilt, _) => Direction::Vector(self.tilted()),
            (ToolAxis::Direction, _) => {
                Direction::Vector([self.axis_dx, self.axis_dy, self.axis_dz])
            }
            _ => Direction::Axis(self.z_axis),
        };
        let x = match (self.machine_x, &self.x_pick) {
            (MachineX::Edge, Some(p)) => edge(p),
            _ if self.tool_axis == ToolAxis::Square => Direction::Axis(self.x_axis),
            _ => Direction::Axis(self.level_x()),
        };
        Orientation { z, x }
    }

    /// Make the X axis square to Z again after Z changed.
    pub fn square_axes(&mut self) {
        if Orientation::square(self.z_axis, self.x_axis)
            .check()
            .is_err()
        {
            self.x_axis = Axis::ALL
                .into_iter()
                .find(|&x| Orientation::square(self.z_axis, x).check().is_ok())
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

    /// Its post's machine profile: its own copy, or the built-in one it
    /// names; `None` when it does not load.
    pub fn profile(&self) -> Option<camcraft_post::profile::Profile> {
        match &self.post_profile {
            Some(text) => camcraft_post::profile::Profile::from_toml(text).ok(),
            None => camcraft_post::profile::Profile::builtin(&self.post)?.ok(),
        }
    }

    /// The work offsets its post has, by number (54 for G54) and word:
    /// G54 to G59, and its extended ones.
    pub fn work_offsets(&self) -> Vec<(u8, String)> {
        match self.profile() {
            Some(p) => p.work_offsets(),
            None => (54..=59).map(|n| (n, format!("G{n}"))).collect(),
        }
    }

    /// The word that selects its work offset, as its post writes it.
    pub fn wcs_word(&self) -> String {
        self.work_offsets()
            .into_iter()
            .find(|(n, _)| f64::from(*n) == self.wcs)
            .map_or_else(|| format!("G{}", self.wcs), |(_, w)| w)
    }

    pub fn problem(&self) -> Option<String> {
        if self.bodies.is_empty() {
            return Some("Pick the bodies the setup machines.".into());
        }
        // Its copy of a post of one's own, which a newer package may read
        // more strictly than the one it was kept with.
        if let Some(text) = &self.post_profile
            && let Err(e) = camcraft_post::profile::Profile::from_toml(text)
        {
            return Some(format!("Its post, {}, does not load: {e}", self.post));
        }
        if !positive(self.tolerance) {
            return Some("The tolerance must be more than 0.".into());
        }
        if self.probe_correct {
            if self.probe_points.is_empty() && self.probe_file.is_empty() {
                return Some(
                    "Load the probe results to correct Z from, or turn the correction off.".into(),
                );
            }
            if !positive(self.probe_segment) {
                return Some("The segment length must be more than 0.".into());
            }
            if !self.probe_points.is_empty()
                && let Err(e) = camcraft_core::probe::ProbeMap::from_points(&self.probe_points, 1.0)
            {
                return Some(format!("The probe results from {}: {e}.", self.probe_file));
            }
        }
        if self.zero == Zero::Picked && self.zero_point.is_none() {
            return Some("Pick the work zero on the part: a corner or a hole's centre.".into());
        }
        if self.wrap && !self.wraps() {
            return Some(
                "On a rotary the stock is a cylinder lying along X or Y, which it turns about."
                    .into(),
            );
        }
        if self.wraps() && self.probe_correct {
            return Some("On a rotary the depths are not corrected to probe results.".into());
        }
        if self.stock == StockKind::Fixed
            && ![self.size_x, self.size_y, self.size_z]
                .into_iter()
                .all(positive)
        {
            return Some("The stock's size must be more than 0 every way.".into());
        }
        if self.stock == StockKind::Cylinder {
            if self.cylinder_fixed && !(positive(self.diameter) && positive(self.length)) {
                return Some("The stock's diameter and length must be more than 0.".into());
            }
            if !self.cylinder_fixed && !(self.margin_radial >= 0.0 && self.margin_ends >= 0.0) {
                return Some("The stock's margins cannot be less than 0.".into());
            }
        }
        let offsets = self.work_offsets();
        if !offsets.iter().any(|(n, _)| f64::from(*n) == self.wcs) {
            let last = offsets.last().map_or("G59", |(_, w)| w.as_str());
            return Some(format!(
                "The work offset must be one {} has: G54 to {last}.",
                self.post
            ));
        }
        if self.wcs_known
            && ![self.wcs_x, self.wcs_y, self.wcs_z]
                .iter()
                .all(|v| v.is_finite())
        {
            return Some("Where the work offset is on the machine must be numbers.".into());
        }
        if !(1.0..=99_999.0).contains(&self.program_number) {
            return Some("The program number must be 1 to 99999.".into());
        }
        match (self.tool_axis, &self.axis_pick) {
            (ToolAxis::Face, None) => {
                return Some("Pick the flat face the tool axis comes out of.".into());
            }
            (ToolAxis::Edge, None) => {
                return Some("Pick the straight edge the tool axis lies along.".into());
            }
            _ => {}
        }
        if self.machine_x == MachineX::Edge && self.x_pick.is_none() {
            return Some("Pick the straight edge machine X lies along.".into());
        }
        if let Err(e) = self.orientation(|b| Some(b.to_string())).check() {
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
    /// The face's name, for a face picked in the view: what a second click
    /// on it takes it out by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<u64>,
    /// The names of the faces an edge runs between, for an edge picked in
    /// the view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faces: Option<[u64; 2]>,
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
    /// Its toolpath repeated in rows, round a centre, or mirrored; none for
    /// a document from before it could be.
    #[serde(default)]
    pub transform: crate::pattern::PatternData,
    #[serde(default)]
    pub faces: Vec<Pick>,
    #[serde(default)]
    pub edges: Vec<Pick>,
    /// Sketches whose curves it follows or cuts, by feature id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sketches: Vec<String>,
    /// The sketches' curves, read from the document when a job is made; not
    /// kept, since the sketches are.
    #[serde(skip)]
    pub curves: Vec<Curve>,
    /// What it waits to be given, made from a template whose picks were on
    /// another part: "the faces, edges or sketches it machines". Until
    /// something is picked it does not run, and says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_pick: Option<String>,
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
            transform: Default::default(),
            faces: Vec::new(),
            edges: Vec::new(),
            sketches: Vec::new(),
            curves: Vec::new(),
            needs_pick: None,
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

    /// Whether it is Custom G-code: lines for the program, with no tool.
    pub fn is_custom(&self) -> bool {
        matches!(self.strategy(), StrategySpec::CustomGcode {})
    }

    /// Its custom G-code's lines, one a line.
    pub fn gcode_lines(&self) -> Vec<String> {
        self.operation.gcode.lines().map(str::to_string).collect()
    }

    /// Set line `row` of its custom G-code to `text`: a row past the last
    /// adds a line, and an empty line is taken out.
    pub fn set_gcode_line(&mut self, row: usize, text: &str) {
        let mut lines = self.gcode_lines();
        let text = text.trim_end();
        match (row < lines.len(), text.is_empty()) {
            (true, true) => {
                lines.remove(row);
            }
            (true, false) => {
                if let Some(line) = lines.get_mut(row) {
                    *line = text.to_string();
                }
            }
            (false, false) => lines.push(text.to_string()),
            (false, true) => {}
        }
        self.operation.gcode = lines.join("\n");
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
        self.transform = crate::pattern::PatternData::of(&op.transform);
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
        op.transform = self.transform.pattern();
        op
    }

    /// The operation as the job has it: [`Self::operation`] with its picks.
    /// camcraft finds a pick by its point when its number does not match, so
    /// any number does; printCAD's face names are not camcraft's numbers.
    /// Its sketches are their curves, once read ([`Self::curves`]); until
    /// then each stands for some, enough to say what it picks.
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
        let curves = if self.curves.is_empty() {
            self.sketches.iter().map(|_| Curve::default()).collect()
        } else {
            self.curves.clone()
        };
        op.selection = Selection {
            faces: refs(&self.faces),
            edges: refs(&self.edges),
            curves,
        };
        op
    }

    /// What it still waits to be given, from a template: nothing once
    /// something is picked, or once its holes are found by their diameter.
    pub fn waits_for_pick(&self) -> Option<&str> {
        let picked = !(self.faces.is_empty() && self.edges.is_empty() && self.sketches.is_empty());
        let rule = self.strategy().uses().holes && self.operation.holes.diameter.is_some();
        self.needs_pick.as_deref().filter(|_| !picked && !rule)
    }

    /// Take sketches `ids`, as a script gives them: what their loops enclose
    /// is what it cuts. A 2D cut's outline is then its picks, as ticking
    /// them in its panel with the picks as its outline does; a 3D cut's
    /// boundary is, else it would go on clearing round the whole part and
    /// the sketches would change nothing. A boundary that had room for the
    /// tool round the part loses it: the sketches are the boundary now,
    /// and room round them would cut outside what they enclose.
    pub fn take_sketches(&mut self, ids: Vec<String>) {
        self.sketches = ids;
        if self.sketches.is_empty() {
            return;
        }
        let uses = self.strategy().uses();
        if uses.outline {
            self.operation.outline = Outline::Selection;
        }
        let b = &mut self.operation.boundary;
        if uses.boundary && b.kind != Boundary::Selection {
            if matches!(b.kind, Boundary::BoundingBox | Boundary::Silhouette)
                && b.containment == Containment::Outside
            {
                self.margin = 0.0;
            }
            b.kind = Boundary::Selection;
        }
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
        // Kept, should it be Custom G-code again.
        op.gcode.clone_from(&self.operation.gcode);
        op.passes.feed = self.feed;
        op.passes.rpm = self.operation.passes.rpm;
        op.linking.plunge_feed = self.plunge;
        // Repeated as it was, whatever it cuts.
        op.transform = self.transform.pattern();
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
        // Its lines are checked with the run, which says which and why.
        if self.is_custom() {
            return None;
        }
        if let Some(what) = self.waits_for_pick() {
            return Some(format!("Needs a pick: {what}."));
        }
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
fn yes() -> bool {
    true
}

fn two() -> f64 {
    2.0
}

fn hundred() -> f64 {
    100.0
}

fn one() -> f64 {
    1.0
}

fn program_number() -> f64 {
    f64::from(camcraft_core::job::DEFAULT_NUMBER)
}

/// Whether `a` points the way its document axis does.
fn is_positive(a: Axis) -> bool {
    matches!(a, Axis::PosX | Axis::PosY | Axis::PosZ)
}

fn positive(v: f64) -> bool {
    v > 0.0
}

/// The strategies a new operation may have, in the order the panel lists them,
/// with what the panel calls them.
pub const STRATEGIES: [(&str, &str); 22] = [
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
    ("bore", "Bore (holes, down a helix)"),
    ("circular", "Circular (holes and bosses)"),
    ("thread", "Thread milling"),
    ("engrave", "Engrave (along curves)"),
    ("slot", "Slot (along curves)"),
    ("chamfer", "Deburr / chamfer (along edges)"),
    ("v-carve", "V-carve"),
    ("custom-gcode", "Custom G-code"),
    ("probe", "Probe (a grid over the top)"),
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
        StrategySpec::Bore { .. } => "Bore",
        StrategySpec::Circular { .. } => "Circular",
        StrategySpec::Thread { .. } => "Thread",
        StrategySpec::Engrave { .. } => "Engrave",
        StrategySpec::VCarve { .. } => "V-carve",
        StrategySpec::Slot { .. } => "Slot",
        StrategySpec::Chamfer { .. } => "Chamfer",
        StrategySpec::CustomGcode {} => "Custom G-code",
        StrategySpec::Probe { .. } => "Probe",
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
        | StrategySpec::Drill { .. }
        | StrategySpec::Bore { .. }
        | StrategySpec::Circular { .. }
        | StrategySpec::Thread { .. }
        | StrategySpec::Engrave { .. }
        | StrategySpec::VCarve { .. }
        | StrategySpec::Slot { .. }
        | StrategySpec::Chamfer { .. }
        | StrategySpec::CustomGcode {}
        // Over the stock's top, all of it, unless kept to less.
        | StrategySpec::Probe { .. } => return,
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
    camcraft.wcs = setup
        .wcs
        .round()
        .clamp(54.0, f64::from(camcraft_core::job::LAST_WCS)) as u8;
    camcraft.work_offset = setup
        .wcs_known
        .then_some([setup.wcs_x, setup.wcs_y, setup.wcs_z]);
    camcraft.programs = setup.programs;
    camcraft.number = setup.program_number.round().clamp(1.0, 99_999.0) as u32;
    camcraft.post = setup.post.clone();
    camcraft.z_correction = setup.z_correction();
    camcraft.wrap = setup.wraps().then(|| camcraft_core::job::Wrap {
        diameter: (setup.wrap_diameter > 0.0).then_some(setup.wrap_diameter),
    });
    camcraft.orientation = setup.orientation(model_of);
    let offset = [setup.offset_x, setup.offset_y, setup.offset_z];
    camcraft.origin = match (&setup.zero, &setup.zero_point) {
        (Zero::Picked, Some(z)) if let Some(model) = model_of(&z.body) => Origin::Point {
            point: PointRef {
                model,
                snap: z.snap,
                // Found again by its point, as a pick is.
                index: u32::MAX,
                at: z.at,
            },
            offset,
        },
        (zero, _) => zero.origin(offset),
    };
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

    /// A setup carrying on from an earlier one keeps which, and one kept
    /// before there was the choice carries on from none.
    #[test]
    fn a_setup_carrying_on_survives_the_document() {
        let mut setup = SetupData::new(vec!["b1".into()], "grbl", 0.05);
        let mut value = setup.to_value();
        value.as_object_mut().unwrap().remove("stock_from");
        assert_eq!(SetupData::from(&value).unwrap().stock_from, None);
        setup.stock = StockKind::Earlier;
        setup.stock_from = Some("s1".into());
        assert_eq!(SetupData::from(&setup.to_value()).unwrap(), setup);
        assert_eq!(setup.to_value()["stock"], "earlier");
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
            name: None,
            faces: None,
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
    fn a_cylinder_of_stock_reads_back_as_camcraft_writes_it() {
        let mut setup = SetupData::new(vec!["b1".into()], "grbl", 0.05);
        setup.stock = StockKind::Cylinder;
        setup.margin_radial = 3.0;
        setup.margin_ends = 1.5;
        let models = models_of(&setup, |_| Some("Part".into()));
        let spec = |setup: &SetupData| {
            let job = job("S", setup, &models, &[]);
            let text = camcraft_session::job_text(&job).unwrap();
            let back = camcraft_session::parse_job(&text, "t").unwrap();
            assert_eq!(back, job);
            assert_eq!(SetupData::from(&setup.to_value()).unwrap(), *setup);
            back.stocks[0].spec.clone()
        };
        assert_eq!(
            spec(&setup),
            StockSpec::CylinderAroundModels {
                models: vec![],
                axis: BarAxis::Z,
                margin: 3.0,
                ends: 1.5,
            }
        );
        // The part on its side, the tool along the document's Y: a bar up
        // the tool lies along Y, and one along the machine's Y along the
        // document's Z (Y is Z × X: −Y × +X).
        setup.z_axis = Axis::NegY;
        setup.x_axis = Axis::PosX;
        let StockSpec::CylinderAroundModels { axis, .. } = spec(&setup) else {
            panic!();
        };
        assert_eq!(axis, BarAxis::Y);
        setup.cylinder_axis = CylinderAxis::Y;
        let StockSpec::CylinderAroundModels { axis, .. } = spec(&setup) else {
            panic!();
        };
        assert_eq!(axis, BarAxis::Z);
        // Of a fixed size up the tool, its top flush with the bodies': the
        // tool points down the document's Y, so the top is the low side.
        setup.cylinder_axis = CylinderAxis::Tool;
        setup.cylinder_fixed = true;
        setup.diameter = 80.0;
        setup.length = 25.0;
        assert_eq!(
            spec(&setup),
            StockSpec::FixedCylinder(FixedCylinder {
                models: vec![],
                axis: BarAxis::Y,
                diameter: 80.0,
                length: 25.0,
                place: Place::Min { offset: 0.0 },
            })
        );
        setup.cylinder_axis = CylinderAxis::X;
        let StockSpec::FixedCylinder(c) = spec(&setup) else {
            panic!();
        };
        assert_eq!((c.axis, c.place), (BarAxis::X, Place::Center {}));
        assert_eq!(setup.problem(), None);
        setup.diameter = 0.0;
        assert!(setup.problem().unwrap().contains("diameter"));
    }

    /// A sketch given to an adaptive clearing is its boundary, without the
    /// room round the part its tool gave it, so it clears what the sketch
    /// encloses and not the whole part: a plate's other pocket is left. A
    /// 2D pocket's outline is the sketch; an engraving follows it, as before.
    #[test]
    fn a_sketch_given_to_a_3d_cut_is_its_boundary() {
        let mut setup = SetupData::new(vec!["b1".into()], "grbl", 0.05);
        let flat = ToolSpec {
            diameter: 6.0,
            ..ToolSpec::default()
        };
        let name = setup.adopt("flat 6", &flat);
        let mut adaptive =
            OperationData::new(Some("s1".into()), StrategySpec::named("adaptive").unwrap());
        adaptive.use_tool(&name, &flat, None, 0.05);
        assert_eq!(adaptive.operation.boundary.kind, Boundary::Silhouette);
        assert_eq!(adaptive.margin, 6.0, "room round the part");
        adaptive.take_sketches(vec!["first".into()]);
        let square = Curve {
            start: [8.0, 12.0, 10.0],
            segments: [[24.0, 12.0], [24.0, 28.0], [8.0, 28.0], [8.0, 12.0]]
                .iter()
                .map(|p| camcraft_core::job::Segment::Line {
                    to: [p[0], p[1], 10.0],
                })
                .collect(),
        };
        adaptive.curves = vec![square.clone()];
        let op = adaptive.picked("Rough", |b| Some(b.to_string()));
        assert_eq!(op.boundary.kind, Boundary::Selection);
        assert_eq!(op.boundary.margin, 0.0);
        assert_eq!(op.selection.curves, vec![square]);
        assert!(
            adaptive.problem(&setup.tools).is_none(),
            "{:?}",
            adaptive.problem(&setup.tools)
        );

        let mut pocket =
            OperationData::new(Some("s1".into()), StrategySpec::named("pocket").unwrap());
        pocket.take_sketches(vec!["first".into()]);
        assert_eq!(pocket.operation.outline, Outline::Selection);
        let mut engrave =
            OperationData::new(Some("s1".into()), StrategySpec::named("engrave").unwrap());
        let before = engrave.operation.clone();
        engrave.take_sketches(vec!["first".into()]);
        assert_eq!(engrave.operation, before);
        assert_eq!(engrave.sketches, vec!["first".to_string()]);

        // None given leaves the boundary as it was.
        let mut other =
            OperationData::new(Some("s1".into()), StrategySpec::named("adaptive").unwrap());
        other.take_sketches(Vec::new());
        assert_eq!(other.operation.boundary.kind, Boundary::Silhouette);
    }

    /// An engraving of a sketch, and a V-carving: their numbers and choices
    /// survive the document, reach the job, and the job reads back; the
    /// sketch's curves are the job's.
    #[test]
    fn an_engraving_and_a_v_carving_read_back_as_camcraft_writes_them() {
        use camcraft_core::job::{Segment, ToolKind, TraceDirection};
        let mut setup = SetupData::new(vec!["b1".into()], "grbl", 0.05);
        let vbit = ToolSpec {
            kind: ToolKind::Cone,
            diameter: 6.0,
            angle: Some(60.0),
            ..ToolSpec::default()
        };
        let name = setup.adopt("V 60", &vbit);
        let mut engrave =
            OperationData::new(Some("s1".into()), StrategySpec::named("engrave").unwrap());
        engrave.use_tool(&name, &vbit, None, 0.05);
        engrave.sketches = vec!["sketch1".into()];
        engrave.passes.depth = 0.8;
        if let StrategySpec::Engrave {
            direction, reverse, ..
        } = &mut engrave.operation.strategy
        {
            *direction = TraceDirection::AsDrawn;
            *reverse = true;
        }
        let back = OperationData::from(&engrave.to_value()).unwrap();
        assert_eq!(back, engrave);
        assert!(
            engrave.problem(&setup.tools).is_none(),
            "{:?}",
            engrave.problem(&setup.tools)
        );
        let mut carve =
            OperationData::new(Some("s1".into()), StrategySpec::named("v-carve").unwrap());
        carve.use_tool(&name, &vbit, None, 0.05);
        carve.sketches = vec!["sketch1".into()];
        carve.passes.max_depth = crate::linking::Opt::of(Some(1.5));
        assert!(
            carve.problem(&setup.tools).is_none(),
            "{:?}",
            carve.problem(&setup.tools)
        );

        // The sketch's curves, as the document gives them when the job is made.
        let square = Curve {
            start: [0.0, 0.0, 5.0],
            segments: [[10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]]
                .iter()
                .map(|p| Segment::Line {
                    to: [p[0], p[1], 5.0],
                })
                .collect(),
        };
        engrave.curves = vec![square.clone()];
        carve.curves = vec![square.clone()];
        let models = models_of(&setup, |_| Some("Part".into()));
        let named = [("E".to_string(), &engrave), ("V".to_string(), &carve)];
        let job = job("S", &setup, &models, &named);
        let text = camcraft_session::job_text(&job).unwrap();
        let back = camcraft_session::parse_job(&text, "t").unwrap();
        assert_eq!(back, job);
        let [e, v] = back.setups[0].operations.as_slice() else {
            panic!("{:?}", back.setups[0].operations);
        };
        assert_eq!(
            e.strategy,
            StrategySpec::Engrave {
                depth: 0.8,
                direction: TraceDirection::AsDrawn,
                reverse: true,
            }
        );
        assert_eq!(e.selection.curves, vec![square.clone()]);
        assert_eq!(
            v.strategy,
            StrategySpec::VCarve {
                max_depth: Some(1.5)
            }
        );
        assert_eq!(v.selection.curves, vec![square]);
        // Neither has a stepover; both cut to size.
        assert_eq!(e.passes.stepdown, None);
        assert!(back.check_setup(0).is_ok(), "{:?}", back.check_setup(0));

        // A flat end mill is no V-bit.
        let flat = ToolSpec {
            diameter: 6.0,
            ..ToolSpec::default()
        };
        let flat_name = setup.adopt("Flat 6", &flat);
        carve.use_tool(&flat_name, &flat, None, 0.05);
        assert!(carve.problem(&setup.tools).unwrap().contains("V-bit"));
    }

    /// A slot along a sketch: its depth, pattern and stepdown survive the
    /// document, reach the job, and the job reads back.
    #[test]
    fn a_slot_reads_back_as_camcraft_writes_it() {
        use camcraft_core::job::{Segment, SlotPattern};
        let mut setup = SetupData::new(vec!["b1".into()], "grbl", 0.05);
        let flat = ToolSpec {
            diameter: 6.0,
            ..ToolSpec::default()
        };
        let name = setup.adopt("Flat 6", &flat);
        let mut slot = OperationData::new(Some("s1".into()), StrategySpec::named("slot").unwrap());
        assert_eq!(slot.passes.depth, 3.0, "a slot's own depth");
        assert!(slot.multiple_depths);
        assert_eq!(slot.stepdown, 1.0);
        slot.use_tool(&name, &flat, None, 0.05);
        slot.sketches = vec!["sketch1".into()];
        slot.passes.depth = 4.0;
        let directional = printcad_bench_sdk::api::PanelEvent::Choice {
            id: "passes/slot_pattern".into(),
            index: 1,
        };
        assert!(crate::passes::passes_event(
            &mut slot,
            "slot_pattern",
            &directional,
            None
        ));
        let back = OperationData::from(&slot.to_value()).unwrap();
        assert_eq!(back, slot);
        assert!(
            slot.problem(&setup.tools).is_none(),
            "{:?}",
            slot.problem(&setup.tools)
        );
        slot.curves = vec![Curve {
            start: [0.0, 0.0, 5.0],
            segments: vec![
                Segment::Line {
                    to: [10.0, 0.0, 5.0],
                },
                Segment::Arc {
                    via: [15.0, 5.0, 5.0],
                    to: [10.0, 10.0, 5.0],
                },
            ],
        }];
        let models = models_of(&setup, |_| Some("Part".into()));
        let job = job("S", &setup, &models, &[("Slot".to_string(), &slot)]);
        let text = camcraft_session::job_text(&job).unwrap();
        let back = camcraft_session::parse_job(&text, "t").unwrap();
        assert_eq!(back, job);
        let op = &back.setups[0].operations[0];
        assert_eq!(
            op.strategy,
            StrategySpec::Slot {
                depth: 4.0,
                pattern: SlotPattern::Directional,
                reverse: false,
            }
        );
        assert_eq!(op.passes.stepdown, Some(1.0));
        assert!(back.check_setup(0).is_ok(), "{:?}", back.check_setup(0));

        // A V-bit is no slot cutter.
        let vbit = ToolSpec {
            kind: camcraft_core::job::ToolKind::Cone,
            diameter: 6.0,
            angle: Some(90.0),
            ..ToolSpec::default()
        };
        let v = setup.adopt("V 90", &vbit);
        slot.use_tool(&v, &vbit, None, 0.05);
        assert!(
            slot.problem(&setup.tools)
                .unwrap()
                .contains("end mill its width")
        );
    }

    /// Custom G-code: its lines, edited a row at a time, survive the
    /// document, reach the job with no tool, and read back; it is no
    /// problem without a tool, and changing its strategy and back keeps them.
    #[test]
    fn custom_gcode_reads_back_as_camcraft_writes_it() {
        let setup = SetupData::new(vec!["b1".into()], "grbl", 0.05);
        let mut op = OperationData::new(
            Some("s1".into()),
            StrategySpec::named("custom-gcode").unwrap(),
        );
        assert!(op.is_custom());
        op.set_gcode_line(0, "M0 (check the clamps)");
        op.set_gcode_line(1, "M7  ");
        op.set_gcode_line(5, "G4 P@rpm@");
        op.set_gcode_line(1, "M8");
        op.set_gcode_line(9, "");
        assert_eq!(
            op.gcode_lines(),
            ["M0 (check the clamps)", "M8", "G4 P@rpm@"]
        );
        op.set_gcode_line(1, "");
        assert_eq!(op.gcode_lines(), ["M0 (check the clamps)", "G4 P@rpm@"]);
        assert_eq!(op.problem(&setup.tools), None, "it has no tool");
        let back = OperationData::from(&op.to_value()).unwrap();
        assert_eq!(back, op);
        let models = models_of(&setup, |_| Some("Part".into()));
        let job = job("S", &setup, &models, &[("Clamps".to_string(), &op)]);
        let text = camcraft_session::job_text(&job).unwrap();
        let back = camcraft_session::parse_job(&text, "t").unwrap();
        assert_eq!(back, job);
        let read = &back.setups[0].operations[0];
        assert_eq!(read.strategy, StrategySpec::CustomGcode {});
        assert_eq!(read.gcode, "M0 (check the clamps)\nG4 P@rpm@");
        assert_eq!(read.tool, None);
        assert_eq!(strategy_word(&read.strategy), "Custom G-code");
        let mut changed = op.clone();
        changed.set_strategy(StrategySpec::named("face").unwrap(), None, 0.05);
        changed.set_strategy(StrategySpec::named("custom-gcode").unwrap(), None, 0.05);
        assert_eq!(changed.gcode_lines(), op.gcode_lines());
    }

    /// A setup's copy of a post of one's own that does not load says why in
    /// its panel, before it is run.
    #[test]
    fn a_setups_post_that_does_not_load_is_its_problem() {
        let mut setup = SetupData::new(vec!["b1".into()], "My mill", 0.05);
        let grbl = include_str!("../camcraft/crates/camcraft-post/profiles/grbl.toml");
        setup.post_profile = Some(grbl.replacen("\"GRBL\"", "\"My mill\"", 1));
        assert_eq!(setup.problem(), None);
        setup.post_profile = Some(grbl.replacen("\"({name})\"", "\"(@nmae@)\"", 1));
        let said = setup.problem().unwrap();
        assert!(
            said.starts_with(
                "Its post, My mill, does not load: header is not usable: line 1: @nmae@"
            ),
            "{said}"
        );
    }

    /// A setup picks its work offset from those its post has, LinuxCNC's
    /// past G59 among them, and says where it is on the machine only when
    /// asked to; the job carries both.
    #[test]
    fn a_setups_work_offset_is_its_posts_and_says_where_it_is() {
        let mut setup = SetupData::new(vec!["b1".into()], "linuxcnc", 0.05);
        let words: Vec<String> = setup.work_offsets().into_iter().map(|(_, w)| w).collect();
        assert_eq!(words.len(), 9);
        assert_eq!(words[6], "G59.1");
        setup.wcs = 61.0;
        assert_eq!(setup.wcs_word(), "G59.2");
        assert_eq!(setup.problem(), None);
        (setup.wcs_x, setup.wcs_y, setup.wcs_z) = (120.0, -80.0, -95.5);
        let models = models_of(&setup, |_| Some("Part".into()));
        let unknown = job("S", &setup, &models, &[]);
        assert_eq!(
            (unknown.setups[0].wcs, unknown.setups[0].work_offset),
            (61, None)
        );
        setup.wcs_known = true;
        let known = job("S", &setup, &models, &[]);
        assert_eq!(known.setups[0].work_offset, Some([120.0, -80.0, -95.5]));
        let text = camcraft_session::job_text(&known).unwrap();
        assert_eq!(camcraft_session::parse_job(&text, "t").unwrap(), known);
        // GRBL has none past G59.
        setup.post = "grbl".into();
        assert_eq!(setup.work_offsets().len(), 6);
        assert!(
            setup
                .problem()
                .unwrap()
                .starts_with("The work offset must be one grbl has")
        );
        // A document from before keeps G54 and no position.
        let mut old = SetupData::new(vec!["b1".into()], "grbl", 0.05).to_value();
        for key in ["wcs_known", "wcs_x", "wcs_y", "wcs_z"] {
            old.as_object_mut().unwrap().remove(key);
        }
        let old = SetupData::from(&old).unwrap();
        assert!(!old.wcs_known && old.wcs == 54.0);
    }

    /// A chamfer of picked edges: its width, tip offset and side survive
    /// the document, reach the job and read back; a cone too small for it,
    /// or no cone, is a problem the panel says.
    #[test]
    fn a_chamfer_reads_back_as_camcraft_writes_it() {
        use camcraft_core::job::{ChamferSide, ToolKind};
        let mut setup = SetupData::new(vec!["b1".into()], "grbl", 0.05);
        let vbit = ToolSpec {
            kind: ToolKind::Cone,
            diameter: 6.0,
            angle: Some(90.0),
            ..ToolSpec::default()
        };
        let name = setup.adopt("V 90", &vbit);
        let mut chamfer =
            OperationData::new(Some("s1".into()), StrategySpec::named("chamfer").unwrap());
        assert_eq!(
            (chamfer.passes.chamfer_width, chamfer.passes.tip_offset),
            (0.5, 0.2)
        );
        chamfer.use_tool(&name, &vbit, None, 0.05);
        chamfer.edges = vec![Pick {
            body: "b1".into(),
            at: [4.0, 0.0, 5.0],
            name: None,
            faces: None,
        }];
        chamfer.passes.chamfer_width = 1.0;
        chamfer.passes.tip_offset = 0.3;
        let flip = printcad_bench_sdk::api::PanelEvent::Choice {
            id: "passes/chamfer_side".into(),
            index: 1,
        };
        assert!(crate::passes::passes_event(
            &mut chamfer,
            "chamfer_side",
            &flip,
            None
        ));
        let back = OperationData::from(&chamfer.to_value()).unwrap();
        assert_eq!(back, chamfer);
        assert!(
            chamfer.problem(&setup.tools).is_none(),
            "{:?}",
            chamfer.problem(&setup.tools)
        );
        let names: Vec<_> = chamfer
            .passes
            .parameters(&chamfer.operation(""))
            .into_iter()
            .map(|(_, n, ..)| n)
            .collect();
        assert!(names.contains(&"chamfer_width") && names.contains(&"tip_offset"));
        let models = models_of(&setup, |_| Some("Part".into()));
        let job = job("S", &setup, &models, &[("Deburr".to_string(), &chamfer)]);
        let text = camcraft_session::job_text(&job).unwrap();
        let back = camcraft_session::parse_job(&text, "t").unwrap();
        assert_eq!(back, job);
        assert_eq!(
            back.setups[0].operations[0].strategy,
            StrategySpec::Chamfer {
                width: 1.0,
                tip_offset: 0.3,
                side: ChamferSide::Flipped,
            }
        );
        assert!(back.check_setup(0).is_ok(), "{:?}", back.check_setup(0));

        // Three millimetres of chamfer is more cone than a 6 mm V-bit has.
        chamfer.passes.chamfer_width = 3.0;
        let why = chamfer.problem(&setup.tools).unwrap();
        assert!(why.contains("more of the cone"), "{why}");
        let flat = ToolSpec {
            diameter: 6.0,
            ..ToolSpec::default()
        };
        let f = setup.adopt("Flat 6", &flat);
        chamfer.use_tool(&f, &flat, None, 0.05);
        assert!(chamfer.problem(&setup.tools).unwrap().contains("V-bit"));
    }

    #[test]
    fn a_setup_from_before_cylinders_reads() {
        let setup = SetupData::new(vec!["b1".into()], "grbl", 0.05);
        let mut value = setup.to_value();
        for key in [
            "cylinder_axis",
            "cylinder_fixed",
            "margin_radial",
            "margin_ends",
            "diameter",
            "length",
        ] {
            value.as_object_mut().unwrap().remove(key);
        }
        assert_eq!(SetupData::from(&value).unwrap(), setup);
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
            name: None,
            faces: None,
        });
        assert!(op.can_measure_from_picks());
        assert_eq!(op.problem(&tools), None);
        // A drill's holes each go to their own bottom.
        let mut drill = OperationData::new(None, StrategySpec::named("drill").unwrap());
        drill.edges.push(Pick {
            body: "b".into(),
            at: [0.0; 3],
            name: None,
            faces: None,
        });
        assert!(!drill.can_measure_from_picks());
    }

    #[test]
    fn a_picked_zero_is_camcrafts_point() {
        let mut setup = SetupData::new(vec!["b1".into()], "grbl", 0.05);
        setup.zero = Zero::Picked;
        assert!(setup.problem().unwrap().contains("Pick the work zero"));
        setup.zero_point = Some(ZeroPoint {
            body: "b1".into(),
            snap: Snap::Vertex,
            at: [0.0, 0.0, 15.0],
        });
        assert_eq!(setup.problem(), None);
        let models = models_of(&setup, |_| Some("Part".into()));
        let job = job("S", &setup, &models, &[]);
        let Origin::Point { point, .. } = &job.setups[0].origin else {
            panic!("{:?}", job.setups[0].origin);
        };
        assert_eq!(
            (point.model.as_str(), point.snap, point.at),
            ("Part", Snap::Vertex, [0.0, 0.0, 15.0])
        );
        let text = camcraft_session::job_text(&job).unwrap();
        assert_eq!(camcraft_session::parse_job(&text, "t").unwrap(), job);
        assert_eq!(SetupData::from(&setup.to_value()).unwrap(), setup);
    }

    /// A tilted setup survives the document and camcraft's job file: its
    /// tool axis picked on a face, by its body's model, and machine X along
    /// a picked edge, the way the view saw it.
    #[test]
    fn a_tilted_setup_reads_back_as_camcraft_writes_it() {
        use camcraft_core::setup::frame::Direction;
        let mut setup = SetupData::new(vec!["b1".into()], "linuxcnc", 0.05);
        let models = models_of(&setup, |_| Some("Wedge".into()));
        let round_trip = |setup: &SetupData| {
            assert_eq!(SetupData::from(&setup.to_value()).unwrap(), *setup);
            let job = job("S", setup, &models, &[]);
            let text = camcraft_session::job_text(&job).unwrap();
            let back = camcraft_session::parse_job(&text, "t").unwrap();
            assert_eq!(back, job);
            back.setups[0].orientation.clone()
        };
        assert_eq!(round_trip(&setup), Orientation::default());
        assert!(setup.square());

        setup.tool_axis = ToolAxis::Face;
        assert!(setup.problem().unwrap().contains("Pick the flat face"));
        setup.axis_pick = Some(AxisPick {
            body: "b1".into(),
            at: [5.0, 10.0, 5.0],
            toward: [0.0, -0.447, 0.894],
            reverse: false,
        });
        assert_eq!(setup.problem(), None);
        setup.machine_x = MachineX::Edge;
        assert!(setup.problem().unwrap().contains("machine X"));
        setup.x_pick = Some(AxisPick {
            body: "b1".into(),
            at: [5.0, 0.0, 0.0],
            toward: [-1.0, 0.0, 0.0],
            reverse: true,
        });
        let o = round_trip(&setup);
        let Direction::Face { face, reverse } = &o.z else {
            panic!("{o:?}");
        };
        assert_eq!(
            (face.model.as_str(), face.at, *reverse),
            ("Wedge", [5.0, 10.0, 5.0], false)
        );
        let Direction::Edge {
            toward, reverse, ..
        } = &o.x
        else {
            panic!("{o:?}");
        };
        assert_eq!((*toward, *reverse), (Some([-1.0, 0.0, 0.0]), true));
        let up = setup.up();
        let l = (0.447f64 * 0.447 + 0.894 * 0.894).sqrt();
        assert!((up[1] + 0.447 / l).abs() < 1e-12 && (up[2] - 0.894 / l).abs() < 1e-12);
        assert!(!setup.square());

        // Tilted by angles: 30° about X leans the tool toward -Y; machine X
        // stays the document's, made square to it.
        setup.tool_axis = ToolAxis::Tilt;
        setup.machine_x = MachineX::Axis;
        setup.tilt_x = 30.0;
        let o = round_trip(&setup);
        let (s, c) = 30f64.to_radians().sin_cos();
        let Direction::Vector(v) = o.z else {
            panic!("{o:?}");
        };
        assert!((v[0]).abs() < 1e-15 && (v[1] + s).abs() < 1e-15 && (v[2] - c).abs() < 1e-15);
        assert_eq!(o.x, Direction::Axis(Axis::PosX));
        // Tilted on its side, nearly along X: machine X is taken from Y.
        setup.tilt_x = 0.0;
        setup.tilt_y = 90.0;
        assert_eq!(round_trip(&setup).x, Direction::Axis(Axis::PosY));
        assert_eq!(setup.tilted(), [1.0, 0.0, 0.0]);

        // A direction of no length is said to be.
        setup.tool_axis = ToolAxis::Direction;
        (setup.axis_dx, setup.axis_dy, setup.axis_dz) = (0.0, 0.0, 0.0);
        assert!(setup.problem().unwrap().contains("no length"));
        setup.axis_dz = 1.0;
        assert_eq!(setup.problem(), None);
    }

    /// A tilted setup lays its stock out in its own axes: a cylinder up the
    /// tool is along its Z, and a fixed one's top flush with the bodies'.
    #[test]
    fn a_tilted_setups_cylinder_lies_along_its_own_axes() {
        let mut setup = SetupData::new(vec!["b1".into()], "grbl", 0.05);
        setup.tool_axis = ToolAxis::Tilt;
        setup.tilt_x = 20.0;
        setup.z_axis = Axis::NegY;
        setup.stock = StockKind::Cylinder;
        setup.cylinder_fixed = true;
        let StockSpec::FixedCylinder(c) = setup.stock_spec() else {
            panic!();
        };
        assert_eq!((c.axis, c.place), (BarAxis::Z, Place::Max { offset: 0.0 }));
        setup.cylinder_axis = CylinderAxis::Y;
        let StockSpec::FixedCylinder(c) = setup.stock_spec() else {
            panic!();
        };
        assert_eq!((c.axis, c.place), (BarAxis::Y, Place::Center {}));
    }

    /// Probe results loaded from a file are kept in the setup and reach its
    /// job in the post's units; results that are no grid are refused, and a
    /// correction with none loaded is said.
    #[test]
    fn probe_results_reach_the_job() {
        let mut setup = SetupData::new(vec!["b1".into()], "linuxcnc-inch", 0.05);
        setup.probe_correct = true;
        assert!(setup.problem().unwrap().contains("Load the probe results"));
        let log = "0 0 0.01 0 0 0 0 0 0\n2 0 0.02 0 0 0 0 0 0\n0 1 0.0 0 0 0 0 0 0\n2 1 0.0 0 0 0 0 0 0\n";
        let said = setup.load_probe_results("probe.txt", log).unwrap();
        assert!(said.contains("4 point(s) from probe.txt: 2 x 2"), "{said}");
        assert_eq!(setup.problem(), None);
        let back = SetupData::from(&setup.to_value()).unwrap();
        assert_eq!(back, setup);
        let job = job("s", &setup, &[], &[]);
        let z = job.setups[0].z_correction.clone().unwrap();
        assert_eq!(z.points.len(), 4);
        assert_eq!(z.units, camcraft_core::job::LengthUnit::Inch);
        assert_eq!(z.file.as_deref(), Some("probe.txt"));
        let e = setup
            .load_probe_results("bad.csv", "0,0,0\n1,0,0\n0,1,0\n")
            .unwrap_err();
        assert!(e.contains("not a grid"), "{e}");
        setup.probe_correct = false;
        assert!(job_of(&setup).setups[0].z_correction.is_none());
    }

    fn job_of(setup: &SetupData) -> Job {
        job("s", setup, &[], &[])
    }
}
