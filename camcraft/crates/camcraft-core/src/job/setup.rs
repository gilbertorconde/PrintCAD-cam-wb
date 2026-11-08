//! A setup: how the part sits on the machine, where its work zero is, and what
//! it machines.

use serde::{Deserialize, Serialize};

use super::{LengthUnit, Operation, PointRef, is_default};
use crate::setup::frame::Orientation;

/// How a part is held and machined from one side.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    /// Name, unique in the job. Programs are named after it.
    pub name: String,
    /// The job's models this setup machines, by name.
    #[serde(default)]
    pub models: Vec<String>,
    /// The job's stocks this setup starts from, by name.
    #[serde(default)]
    pub stocks: Vec<String>,
    /// Work offset, 54 for `G54` to 59 for `G59`; from 60, the post's
    /// extended ones in its order (60 is LinuxCNC's `G59.1`).
    #[serde(default = "default_wcs")]
    pub wcs: u8,
    /// Where the work zero is in machine coordinates, in millimetres along
    /// the setup's axes: what its work offset holds, for checking a
    /// program against the machine's travel. Z is the spindle's gauge line
    /// at the work zero when the post takes tool length offsets, else the
    /// tool's tip. `None` when not known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_offset: Option<[f64; 3]>,
    /// The post processor: a built-in name, or a profile file relative to the job.
    #[serde(default = "default_post")]
    pub post: String,
    /// Which model directions become the machine's Z and X.
    #[serde(default, skip_serializing_if = "is_default")]
    pub orientation: Orientation,
    /// Where the work zero is.
    #[serde(default, skip_serializing_if = "is_default")]
    pub origin: Origin,
    /// Whether it is posted as one program, or one an operation.
    #[serde(default, skip_serializing_if = "is_default")]
    pub programs: Programs,
    /// Its program's number, for a control that files programs by number
    /// (`O1001`): each program of a setup posted one an operation takes the
    /// next.
    #[serde(default = "default_number", skip_serializing_if = "is_default_number")]
    pub number: u32,
    /// Every cut's depth corrected to a probed surface, when it is given:
    /// a warped board's, or a single point's, for the work zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z_correction: Option<ZCorrection>,
    /// Machined on a rotary that turns its round stock about its axis, when
    /// it is given: everything is planned on the stock's cylinder unrolled,
    /// and the program turns the rotary as it cuts ([`Wrap`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap: Option<Wrap>,
    /// Operations, in the order they run.
    #[serde(default, rename = "operation")]
    pub operations: Vec<Operation>,
}

/// A setup's cut depths corrected to a probed surface: every move of every
/// operation but a probe's moved up or down by the surface's height there,
/// as [`crate::probe::correct`] does, and the stock's top and the part taken
/// to follow it, so the simulation and the checks see the setup as it will
/// cut.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ZCorrection {
    /// The probe results: x, y and z of each point probed, in the program's
    /// coordinates and `units`, as the controller recorded them. Read from
    /// `file` when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<[f64; 3]>,
    /// The file the results were read from, or are to be: a LinuxCNC probe
    /// log or a CSV of x, y and z ([`crate::probe::parse`]). Relative to the
    /// job's folder, or absolute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// The units the results are in: the program's.
    #[serde(default = "millimetres", skip_serializing_if = "is_millimetres")]
    pub units: LengthUnit,
    /// The longest piece a corrected move is split into, seen from above, in
    /// millimetres: shorter follows the surface more closely between the
    /// probed points, in a longer program.
    #[serde(
        default = "default_segment",
        skip_serializing_if = "is_default_segment"
    )]
    pub segment: f64,
}

impl ZCorrection {
    /// The results as a map, in millimetres; `None` until they are read.
    pub fn map(&self) -> Option<Result<crate::probe::ProbeMap, crate::probe::ProbeMapError>> {
        if self.points.is_empty() {
            return None;
        }
        let scale = match self.units {
            LengthUnit::Mm => 1.0,
            LengthUnit::Inch => 25.4,
        };
        Some(crate::probe::ProbeMap::from_points(&self.points, scale))
    }
}

fn millimetres() -> LengthUnit {
    LengthUnit::Mm
}

// Serde's `skip_serializing_if` hands the field by reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_millimetres(u: &LengthUnit) -> bool {
    *u == LengthUnit::Mm
}

/// A millimetre: a board's warp is smooth over that.
pub const DEFAULT_SEGMENT: f64 = 1.0;

fn default_segment() -> f64 {
    DEFAULT_SEGMENT
}

// Serde's `skip_serializing_if` hands the field by reference.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_default_segment(v: &f64) -> bool {
    (*v - DEFAULT_SEGMENT).abs() < f64::EPSILON
}

/// A setup machined on a rotary, as Fusion's Wrap toolpath and rotary
/// operations: its stock a round bar lying along its X (turned by an A axis)
/// or its Y (by a B axis), the tool pointing at the bar's axis. Its work
/// zero is on the axis, across it. Its operations are planned on the bar
/// unrolled ([`crate::setup::frame::Unrolled`]): a sketch is laid flat on it
/// as drawn, its Y (or X) a distance round the bar at the wrap diameter, and
/// Z is how far out from the axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Wrap {
    /// The diameter a flat path is wrapped round at, in millimetres: a
    /// millimetre round there is a millimetre of the flat. The stock's when
    /// not given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diameter: Option<f64>,
}

/// How a setup's operations are posted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Programs {
    /// One program for the whole setup, as Fusion posts one: a header, each
    /// operation in order, the tool changed only where it changes, and a
    /// footer.
    #[default]
    Setup,
    /// One program an operation, each with its own header and footer.
    Operation,
}

/// The number a setup's program takes unless it is given one.
pub const DEFAULT_NUMBER: u32 = 1001;

impl Setup {
    /// An empty setup with the usual defaults.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            models: Vec::new(),
            stocks: Vec::new(),
            wcs: default_wcs(),
            work_offset: None,
            post: default_post(),
            orientation: Orientation::default(),
            origin: Origin::default(),
            programs: Programs::default(),
            number: DEFAULT_NUMBER,
            z_correction: None,
            wrap: None,
            operations: Vec::new(),
        }
    }
}

/// The last work offset a setup may use: G59 and 99 extended ones past it,
/// as many as Haas's `G154 P1` to `P99`.
pub const LAST_WCS: u8 = 59 + 99;

fn default_wcs() -> u8 {
    54
}

fn default_post() -> String {
    "masso".into()
}

fn default_number() -> u32 {
    DEFAULT_NUMBER
}

fn is_default_number(n: &u32) -> bool {
    *n == DEFAULT_NUMBER
}

/// Where a setup's work zero is.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Origin {
    /// The design's own zero, turned with the part, moved by `offset` along the
    /// setup's axes.
    ModelOrigin {
        /// Millimetres along the setup's X, Y and Z.
        #[serde(default, skip_serializing_if = "is_default")]
        offset: [f64; 3],
    },
    /// A point on the box around the setup's stock or models, as the part sits on
    /// the machine, moved by `offset`.
    BoxPoint {
        /// Which box.
        of: BoxOf,
        /// Where along X.
        x: Along,
        /// Where along Y.
        y: Along,
        /// Where along Z.
        z: Along,
        /// Millimetres along the setup's X, Y and Z.
        #[serde(default, skip_serializing_if = "is_default")]
        offset: [f64; 3],
    },
    /// A point picked on a model: a corner, the centre of a round edge, or the
    /// middle of an edge, moved by `offset`.
    Point {
        /// The point.
        point: PointRef,
        /// Millimetres along the setup's X, Y and Z.
        #[serde(default, skip_serializing_if = "is_default")]
        offset: [f64; 3],
    },
}

impl Origin {
    /// Millimetres the work zero is moved by, along the setup's axes.
    pub fn offset(&self) -> [f64; 3] {
        match self {
            Origin::ModelOrigin { offset }
            | Origin::BoxPoint { offset, .. }
            | Origin::Point { offset, .. } => *offset,
        }
    }

    /// The same work zero, moved by `to` instead.
    pub fn with_offset(mut self, to: [f64; 3]) -> Self {
        match &mut self {
            Origin::ModelOrigin { offset }
            | Origin::BoxPoint { offset, .. }
            | Origin::Point { offset, .. } => *offset = to,
        }
        self
    }

    /// The stock's front-left corner on its top, so X and Y run into the part
    /// and Z is zero where the tool is touched off: the corner Autodesk's own
    /// Fusion tutorials and setup sample pick (`TOP_XMIN_YMIN`).
    pub fn stock_corner() -> Self {
        Origin::BoxPoint {
            of: BoxOf::Stock,
            x: Along::Min,
            y: Along::Min,
            z: Along::Max,
            offset: [0.0; 3],
        }
    }
}

impl Default for Origin {
    /// The design's own zero, as models are drawn.
    fn default() -> Self {
        Origin::ModelOrigin { offset: [0.0; 3] }
    }
}

/// Which box an origin is placed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BoxOf {
    /// The setup's stock.
    Stock,
    /// The setup's models.
    Model,
}

/// Where along one side of a box.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Along {
    /// The low end: left, front or bottom.
    Min,
    /// The middle.
    Center,
    /// The high end: right, back or top.
    Max,
}

impl Along {
    /// The point along `lo..hi`.
    pub fn pick(self, lo: f64, hi: f64) -> f64 {
        match self {
            Along::Min => lo,
            Along::Center => (lo + hi) * 0.5,
            Along::Max => hi,
        }
    }
}
