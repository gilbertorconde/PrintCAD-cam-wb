//! The models a job machines and the blocks of material it starts from.

use glam::{DAffine3, DQuat, DVec3};
use serde::{Deserialize, Serialize};

use super::is_default;

/// Units for a file that does not say, as an STL does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LengthUnit {
    /// Millimetres.
    Mm,
    /// Inches.
    Inch,
}

/// One design file.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDef {
    /// Name, unique in the job; setups and stocks refer to it by this.
    pub name: String,
    /// The file, relative to the job file's folder, with `/` between folders. An
    /// absolute path is allowed.
    pub path: String,
    /// What an STL's numbers mean. Required for STL, which does not say; never
    /// guessed. STEP carries its own units.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub units: Option<LengthUnit>,
    /// Where it sits in the job, from where its file put it.
    #[serde(default, skip_serializing_if = "is_default")]
    pub transform: Transform,
}

/// Where a model sits in the job: its file's points scaled, then turned, then
/// moved, as Fusion's position, rotation and scale. Everything past loading sees
/// the model where this puts it; only picks keep the file's coordinates, so they
/// move with it.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(default)]
pub struct Transform {
    /// Where the file's origin goes, millimetres.
    pub position: [f64; 3],
    /// Turns about the job's X, then Y, then Z, in degrees, about the file's
    /// origin.
    pub rotation: [f64; 3],
    /// How much larger along the file's X, Y and Z. Never mirrored: a scale
    /// must be more than zero.
    pub scale: [f64; 3],
}

impl Default for Transform {
    /// Where the file put it.
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            rotation: [0.0; 3],
            scale: [1.0; 3],
        }
    }
}

impl Transform {
    /// From the file's coordinates to the job's.
    pub fn matrix(&self) -> DAffine3 {
        let [x, y, z] = self.rotation.map(f64::to_radians);
        let turn =
            DQuat::from_rotation_z(z) * DQuat::from_rotation_y(y) * DQuat::from_rotation_x(x);
        DAffine3::from_scale_rotation_translation(
            DVec3::from_array(self.scale),
            turn,
            DVec3::from_array(self.position),
        )
    }
}

/// One block of material.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct StockDef {
    /// Name, unique in the job; setups refer to it by this.
    pub name: String,
    /// Its shape.
    #[serde(flatten)]
    pub spec: StockSpec,
}

/// The shape of a block, in model coordinates, millimetres.
///
/// For a tilted setup, whose axes are not the model's, its X, Y and Z are the
/// setup's instead: a box round models is round them as the setup sees them,
/// its margins along the setup's axes, and a bar lies along one of them.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StockSpec {
    /// A box around models, grown by a margin.
    AroundModels {
        /// The models it is around, by name. Empty means the models of whichever
        /// setup uses it.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        models: Vec<String>,
        /// Extra material each side in X and Y, and on top in Z.
        #[serde(default)]
        margin: [f64; 3],
        /// Extra material under the models.
        ///
        /// Zero by default: for a part cut from the top only, material below the
        /// model is the part of the block the cutter never reaches.
        #[serde(default, skip_serializing_if = "is_default")]
        below: f64,
    },
    /// A box of a fixed size, placed against models.
    FixedBox(FixedBox),
    /// A round bar around models: its axis along `axis` through the middle of
    /// their box, its radius `margin` more than the farthest they reach from
    /// it, and `ends` longer than they are at each end.
    CylinderAroundModels {
        /// The models it is around, by name. Empty means the models of whichever
        /// setup uses it.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        models: Vec<String>,
        /// The model axis it lies along.
        axis: BarAxis,
        /// Extra material all round.
        #[serde(default)]
        margin: f64,
        /// Extra length at each end.
        #[serde(default)]
        ends: f64,
    },
    /// A round bar of a fixed size, placed against models.
    FixedCylinder(FixedCylinder),
    /// A body of the job's own, as Fusion's "From solid": a casting, a blank
    /// cut before, a printed part near its net shape. The setup starts from
    /// its material, each column of it from the lowest it comes to the highest
    /// along the tool, and its box is the stock's.
    Solid {
        /// The model it is, by name: never one the setup machines.
        model: String,
    },
}

impl StockSpec {
    /// The models it is placed against, by name. Empty means the models of
    /// whichever setup uses it.
    pub fn models(&self) -> &[String] {
        match self {
            StockSpec::AroundModels { models, .. }
            | StockSpec::FixedBox(FixedBox { models, .. })
            | StockSpec::CylinderAroundModels { models, .. }
            | StockSpec::FixedCylinder(FixedCylinder { models, .. }) => models,
            StockSpec::Solid { model } => std::slice::from_ref(model),
        }
    }
}

/// A model axis, as a round bar lies along it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BarAxis {
    /// Along X.
    X,
    /// Along Y.
    Y,
    /// Along Z.
    #[default]
    Z,
}

impl BarAxis {
    /// Which coordinate (0, 1 or 2).
    pub fn index(self) -> usize {
        match self {
            BarAxis::X => 0,
            BarAxis::Y => 1,
            BarAxis::Z => 2,
        }
    }

    /// The unit vector along it.
    pub fn vector(self) -> DVec3 {
        match self {
            BarAxis::X => DVec3::X,
            BarAxis::Y => DVec3::Y,
            BarAxis::Z => DVec3::Z,
        }
    }
}

/// A round bar of a fixed size, as one bought to size is: centred on the box of
/// the models across its axis, and placed against it along its axis as a fixed
/// box is on each of its, so it follows them when they move.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixedCylinder {
    /// The models it is placed against, by name. Empty means the models of
    /// whichever setup uses it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    /// The model axis it lies along.
    pub axis: BarAxis,
    /// Its diameter.
    pub diameter: f64,
    /// Its length.
    pub length: f64,
    /// Where it sits along its axis; centred when not said.
    #[serde(default = "centred")]
    pub place: Place,
}

fn centred() -> Place {
    Place::Center {}
}

impl Default for StockSpec {
    /// Around the setup's models, as the block a new job starts with.
    fn default() -> Self {
        StockSpec::AroundModels {
            models: Vec::new(),
            margin: [2.0, 2.0, 2.0],
            below: 0.0,
        }
    }
}

/// A block of a fixed size, as a board bought to size is: on each axis it is
/// placed against the box of the models, the way Fusion's fixed size box is, so
/// it follows them when they move.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(from = "FixedBoxFile", into = "FixedBoxFile")]
pub struct FixedBox {
    /// Its size along X, Y and Z.
    pub size: [f64; 3],
    /// The models it is placed against, by name. Empty means the models of
    /// whichever setup uses it.
    pub models: Vec<String>,
    /// Where it sits on X, Y and Z.
    pub place: [Place; 3],
}

impl FixedBox {
    /// A block of `size`, centred on the models in X and Y with its top flush with
    /// theirs: where Fusion puts a new one.
    pub fn centred(size: [f64; 3]) -> Self {
        Self {
            size,
            models: Vec::new(),
            place: [
                Place::Center {},
                Place::Center {},
                Place::Max { offset: 0.0 },
            ],
        }
    }

    /// A block of `size` with its low corner at `corner`, wherever the models are.
    pub fn at(corner: [f64; 3], size: [f64; 3]) -> Self {
        Self {
            size,
            models: Vec::new(),
            place: corner.map(|coordinate| Place::At { coordinate }),
        }
    }

    /// Whether it is placed against models, rather than only at a corner.
    pub fn placed(&self) -> bool {
        !self.models.is_empty() || self.place.iter().any(|p| !matches!(p, Place::At { .. }))
    }
}

/// Where a fixed box sits along one axis, against the models' box.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "from", rename_all = "kebab-case")]
pub enum Place {
    /// Centred on the models.
    Center {},
    /// Its low side (left, front or bottom) `offset` beyond the models'.
    Min {
        /// How far beyond; less than zero is inside.
        offset: f64,
    },
    /// Its high side (right, back or top) `offset` beyond the models'.
    Max {
        /// How far beyond; less than zero is inside.
        offset: f64,
    },
    /// Its low side at a coordinate, wherever the models are.
    At {
        /// The model coordinate.
        coordinate: f64,
    },
}

impl Place {
    /// Where a block `size` long goes, against the models' `lo..hi`: its low end.
    pub fn low(self, lo: f64, hi: f64, size: f64) -> f64 {
        match self {
            Place::Center {} => (lo + hi - size) * 0.5,
            Place::Min { offset } => lo - offset,
            Place::Max { offset } => hi + offset - size,
            Place::At { coordinate } => coordinate,
        }
    }

    /// Its offset or coordinate; none when centred.
    pub fn value(self) -> Option<f64> {
        match self {
            Place::Center {} => None,
            Place::Min { offset } | Place::Max { offset } => Some(offset),
            Place::At { coordinate } => Some(coordinate),
        }
    }
}

/// A fixed box as its file holds it: a place on each axis.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct FixedBoxFile {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    models: Vec<String>,
    size: [f64; 3],
    x: Place,
    y: Place,
    z: Place,
}

impl From<FixedBoxFile> for FixedBox {
    fn from(f: FixedBoxFile) -> Self {
        Self {
            size: f.size,
            models: f.models,
            place: [f.x, f.y, f.z],
        }
    }
}

impl From<FixedBox> for FixedBoxFile {
    fn from(b: FixedBox) -> Self {
        let [x, y, z] = b.place;
        Self {
            models: b.models,
            size: b.size,
            x,
            y,
            z,
        }
    }
}
