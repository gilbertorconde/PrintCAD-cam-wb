//! How a setup holds the part: its orientation on the table, and where its work
//! zero is.
//!
//! Models are drawn in their own (design) coordinates. A setup turns them so that
//! a chosen model direction points up the spindle, then measures everything from
//! its work zero. Everything past that point (strategies, the linker, the post)
//! works in the setup frame and never sees a rotation.
//!
//! Most setups take one of the 24 orientations that map axes onto axes, given
//! as model axes, the ways a block can sit square on a 3-axis table. Then a box
//! stays a box, exactly, and turning a point only swaps and negates its
//! coordinates: no rounding, and the identity orientation changes nothing bit
//! for bit ([`Turn::Square`]).
//!
//! An axis given any other way, as a vector, a flat face's normal or a straight
//! edge's direction, tilts the part for 3+2 machining ([`Turn::Tilted`]): the
//! setup's axes are then unit vectors in the model, and a point is turned by
//! rotating it. Its X is the one given made square to Z, so a model axis given
//! for X is its projection onto the plane square to the tool. Which of the two
//! a setup is goes by how it is given, not by where its axes come out, so a
//! picked face that happens to lie along an axis lays its stock out as any
//! other picked face does ([`crate::setup`]).

use glam::{DMat4, DVec3, DVec4};
use serde::{Deserialize, Serialize};

use crate::job::FeatureRef;
use crate::setup::Bodies;
use crate::setup::features::{self, FeatureError};
use camcraft_geom::topology::{EdgeKind, FaceKind};

/// A model axis, either way along it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Axis {
    /// +X.
    #[serde(rename = "+x")]
    PosX,
    /// −X.
    #[serde(rename = "-x")]
    NegX,
    /// +Y.
    #[serde(rename = "+y")]
    PosY,
    /// −Y.
    #[serde(rename = "-y")]
    NegY,
    /// +Z.
    #[serde(rename = "+z")]
    PosZ,
    /// −Z.
    #[serde(rename = "-z")]
    NegZ,
}

impl Axis {
    /// Every axis, in the order the app lists them.
    pub const ALL: [Axis; 6] = [
        Axis::PosX,
        Axis::NegX,
        Axis::PosY,
        Axis::NegY,
        Axis::PosZ,
        Axis::NegZ,
    ];

    /// Which coordinate (0, 1 or 2) and which way.
    fn parts(self) -> (usize, f64) {
        match self {
            Axis::PosX => (0, 1.0),
            Axis::NegX => (0, -1.0),
            Axis::PosY => (1, 1.0),
            Axis::NegY => (1, -1.0),
            Axis::PosZ => (2, 1.0),
            Axis::NegZ => (2, -1.0),
        }
    }

    fn from_parts(i: usize, sign: f64) -> Axis {
        match (i, sign > 0.0) {
            (0, true) => Axis::PosX,
            (0, false) => Axis::NegX,
            (1, true) => Axis::PosY,
            (1, false) => Axis::NegY,
            (2, true) => Axis::PosZ,
            _ => Axis::NegZ,
        }
    }

    /// The unit vector.
    pub fn vector(self) -> DVec3 {
        let (i, s) = self.parts();
        let mut v = [0.0; 3];
        if let Some(slot) = v.get_mut(i) {
            *slot = s;
        }
        DVec3::from_array(v)
    }

    /// The same axis, the other way.
    pub fn flipped(self) -> Axis {
        let (i, s) = self.parts();
        Axis::from_parts(i, -s)
    }

    /// How the app names it.
    pub fn label(self) -> &'static str {
        match self {
            Axis::PosX => "+X",
            Axis::NegX => "−X",
            Axis::PosY => "+Y",
            Axis::NegY => "−Y",
            Axis::PosZ => "+Z",
            Axis::NegZ => "−Z",
        }
    }
}

/// A model direction one of a setup's axes is given as.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Direction {
    /// Along a model axis.
    Axis(Axis),
    /// Along a vector, in model coordinates, of any length but zero.
    Vector([f64; 3]),
    /// Out of a flat face picked on a model, along its outward normal; into
    /// it, `reverse`.
    Face {
        /// The face.
        face: FeatureRef,
        /// The other way.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        reverse: bool,
    },
    /// Along a straight edge picked on a model: the way of `toward`, a rough
    /// direction such as a view gives, or without one from the end its
    /// topology puts first to the other; the other way, `reverse`.
    Edge {
        /// The edge.
        edge: FeatureRef,
        /// Which way along it, roughly, in model coordinates.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        toward: Option<[f64; 3]>,
        /// The other way.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        reverse: bool,
    },
}

impl From<Axis> for Direction {
    fn from(a: Axis) -> Self {
        Direction::Axis(a)
    }
}

impl std::fmt::Display for Direction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Direction::Axis(a) => f.write_str(a.label()),
            Direction::Vector([x, y, z]) => write!(f, "({x}, {y}, {z})"),
            Direction::Face { face, reverse } => write!(
                f,
                "{} the face picked on '{}'",
                if *reverse { "into" } else { "out of" },
                face.model
            ),
            Direction::Edge { edge, reverse, .. } => write!(
                f,
                "along the edge picked on '{}'{}",
                edge.model,
                if *reverse { ", reversed" } else { "" }
            ),
        }
    }
}

/// A direction worked out: a unit vector, and the model axis it was given
/// as, if it was.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Aim {
    unit: DVec3,
    axis: Option<Axis>,
}

impl Aim {
    fn of(v: DVec3, given: &Direction) -> Result<Self, BadOrientation> {
        let unit = v.normalize_or_zero();
        if unit == DVec3::ZERO || !unit.is_finite() {
            return Err(BadOrientation::NoDirection {
                given: given.to_string(),
            });
        }
        Ok(Self { unit, axis: None })
    }
}

impl Direction {
    /// The model axis it is given as, if it is.
    pub fn axis(&self) -> Option<Axis> {
        match self {
            Direction::Axis(a) => Some(*a),
            Direction::Vector(_) | Direction::Face { .. } | Direction::Edge { .. } => None,
        }
    }

    /// Whether it is picked on a model, so only found with the models.
    pub fn picked(&self) -> bool {
        matches!(self, Direction::Face { .. } | Direction::Edge { .. })
    }

    /// The same direction, the other way.
    pub fn flipped(&self) -> Direction {
        match self {
            Direction::Axis(a) => Direction::Axis(a.flipped()),
            Direction::Vector(v) => Direction::Vector(v.map(|c| -c)),
            Direction::Face { face, reverse } => Direction::Face {
                face: face.clone(),
                reverse: !reverse,
            },
            Direction::Edge {
                edge,
                toward,
                reverse,
            } => Direction::Edge {
                edge: edge.clone(),
                toward: *toward,
                reverse: !reverse,
            },
        }
    }

    /// Worked out without the models: `None` for a pick.
    fn aim(&self) -> Option<Result<Aim, BadOrientation>> {
        match self {
            Direction::Axis(a) => Some(Ok(Aim {
                unit: a.vector(),
                axis: Some(*a),
            })),
            Direction::Vector(v) => Some(Aim::of(DVec3::from_array(*v), self)),
            Direction::Face { .. } | Direction::Edge { .. } => None,
        }
    }

    /// Worked out, a pick found on its model as it is now.
    fn find(&self, bodies: &Bodies<'_>, tolerance: f64) -> Result<Aim, BadOrientation> {
        let body_of = |pick: &FeatureRef| {
            let body = bodies
                .get(pick.model.as_str())
                .ok_or_else(|| FeatureError::NotLoaded(pick.model.clone()))?;
            Ok::<_, BadOrientation>((*body, features::tolerance(tolerance, body)))
        };
        let (v, reverse) = match self {
            Direction::Axis(_) | Direction::Vector(_) => {
                return self.aim().unwrap_or_else(|| {
                    Err(BadOrientation::NoDirection {
                        given: self.to_string(),
                    })
                });
            }
            Direction::Face { face, reverse } => {
                let (body, tol) = body_of(face)?;
                let found = features::find_face(body, face, tol)?;
                match body.topology().face(found.index).map(|f| f.kind) {
                    Some(FaceKind::Plane { normal, .. }) => (normal, *reverse),
                    _ => {
                        return Err(BadOrientation::NotFlat {
                            given: self.to_string(),
                        });
                    }
                }
            }
            Direction::Edge {
                edge,
                toward,
                reverse,
            } => {
                let (body, tol) = body_of(edge)?;
                let found = features::find_edge(body, edge, tol)?;
                let t = body.topology();
                let ends = t
                    .edge(found.index)
                    .filter(|e| matches!(e.kind, EdgeKind::Line))
                    .and_then(|e| e.ends)
                    .and_then(|[a, b]| Some((t.vertex(a)?.position, t.vertex(b)?.position)));
                let toward = toward.map(DVec3::from_array);
                match ends {
                    Some((a, b)) if toward.is_some_and(|t| t.dot(b - a) < 0.0) => (a - b, *reverse),
                    Some((a, b)) => (b - a, *reverse),
                    None => {
                        return Err(BadOrientation::NotStraight {
                            given: self.to_string(),
                        });
                    }
                }
            }
        };
        Aim::of(if reverse { -v } else { v }, self)
    }
}

/// The two setup axes given as model directions: `z` points up the spindle, away
/// from the table, and `x` along the machine's X. Y follows, as `z × x`.
///
/// `x` need not be square to `z`: the setup's X is it made square, so the
/// default, model X, is model X projected square to the tool. It only may not
/// lie along `z`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Orientation {
    /// The model direction that becomes the setup's +Z.
    pub z: Direction,
    /// The model direction that becomes the setup's +X, made square to Z.
    pub x: Direction,
}

impl Default for Orientation {
    /// As modelled.
    fn default() -> Self {
        Self::square(Axis::PosZ, Axis::PosX)
    }
}

/// An orientation that cannot be worked out.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BadOrientation {
    /// Its X lies along its Z, so no X can be made square to Z from it.
    #[error("the X axis ({x}) must not lie along the Z axis ({z})")]
    Parallel {
        /// The Z axis given.
        z: String,
        /// The X axis given.
        x: String,
    },
    /// A vector of no length.
    #[error("the direction {given} has no length")]
    NoDirection {
        /// The direction given.
        given: String,
    },
    /// A tool axis from a face that is not flat.
    #[error("{given} is not flat, so has no one normal to take an axis from")]
    NotFlat {
        /// The direction given.
        given: String,
    },
    /// An axis from an edge that is not straight.
    #[error("{given} is not straight, so has no one direction to take an axis from")]
    NotStraight {
        /// The direction given.
        given: String,
    },
    /// The face or edge picked for an axis is not there.
    #[error("the setup's orientation: {0}")]
    Pick(#[from] FeatureError),
}

impl Orientation {
    /// `z` and `x` along model axes.
    pub fn square(z: Axis, x: Axis) -> Self {
        Self {
            z: Direction::Axis(z),
            x: Direction::Axis(x),
        }
    }

    /// The part turned over about X, for machining its underside.
    pub fn flipped(&self) -> Self {
        Self {
            z: self.z.flipped(),
            x: self.x.clone(),
        }
    }

    /// Whether it can be worked out, as far as it can be without the models:
    /// a pick is checked once found.
    pub fn check(&self) -> Result<(), BadOrientation> {
        match (self.z.aim(), self.x.aim()) {
            (Some(z), Some(x)) => Turn::between(z?, x?, self).map(|_| ()),
            (Some(a), None) | (None, Some(a)) => a.map(|_| ()),
            (None, None) => Ok(()),
        }
    }

    /// The turn, for an orientation that needs no model to work out: `None`
    /// when it has a pick, which needs [`resolve`](Self::resolve).
    pub fn turn(&self) -> Option<Result<Turn, BadOrientation>> {
        let (z, x) = (self.z.aim()?, self.x.aim()?);
        Some(z.and_then(|z| Turn::between(z, x?, self)))
    }

    /// The turn, its picks found on `bodies` as they are now, within the
    /// job's `tolerance`.
    pub fn resolve(&self, bodies: &Bodies<'_>, tolerance: f64) -> Result<Turn, BadOrientation> {
        let z = self.z.find(bodies, tolerance)?;
        let x = self.x.find(bodies, tolerance)?;
        Turn::between(z, x, self)
    }
}

/// How a setup turns model coordinates into its own, worked out from its
/// [`Orientation`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Turn {
    /// Model axes onto setup axes: only swaps and negates, exactly.
    Square {
        /// The model axis that is the setup's +Z.
        z: Axis,
        /// The model axis that is the setup's +X.
        x: Axis,
    },
    /// Any other: the setup's axes as unit vectors in the model, square to
    /// each other, right-handed.
    Tilted {
        /// The setup's +X.
        x: DVec3,
        /// The setup's +Y.
        y: DVec3,
        /// The setup's +Z, up the tool.
        z: DVec3,
    },
}

impl Default for Turn {
    /// As modelled.
    fn default() -> Self {
        Turn::Square {
            z: Axis::PosZ,
            x: Axis::PosX,
        }
    }
}

/// Shorter than this, what is left of X once made square to Z lies along Z.
const PARALLEL: f64 = 1e-6;

impl Turn {
    fn between(z: Aim, x: Aim, given: &Orientation) -> Result<Self, BadOrientation> {
        let parallel = || BadOrientation::Parallel {
            z: given.z.to_string(),
            x: given.x.to_string(),
        };
        if let (Some(za), Some(xa)) = (z.axis, x.axis) {
            if za.parts().0 == xa.parts().0 {
                return Err(parallel());
            }
            return Ok(Turn::Square { z: za, x: xa });
        }
        let square = x.unit - z.unit * x.unit.dot(z.unit);
        if square.length() < PARALLEL {
            return Err(parallel());
        }
        let xs = square.normalize();
        Ok(Turn::Tilted {
            x: xs,
            y: z.unit.cross(xs),
            z: z.unit,
        })
    }

    /// The setup's X, Y and Z as unit model vectors.
    pub fn axes(&self) -> [DVec3; 3] {
        match *self {
            Turn::Square { z, x } => [x.vector(), square_y(z, x).vector(), z.vector()],
            Turn::Tilted { x, y, z } => [x, y, z],
        }
    }

    /// The model direction up the tool.
    pub fn tool_axis(&self) -> DVec3 {
        self.axes()[2]
    }

    /// The model axes it maps onto the setup's, when it maps axes onto axes.
    pub fn square(&self) -> Option<(Axis, Axis)> {
        match *self {
            Turn::Square { z, x } => Some((z, x)),
            Turn::Tilted { .. } => None,
        }
    }

    /// A model point in the setup's axes. Only swaps and negates when
    /// square, so it is exact.
    pub fn apply(&self, p: DVec3) -> DVec3 {
        match *self {
            Turn::Square { z, x } => {
                let pick = |a: Axis| {
                    let (i, s) = a.parts();
                    let v = match i {
                        0 => p.x,
                        1 => p.y,
                        _ => p.z,
                    };
                    if s > 0.0 { v } else { -v }
                };
                DVec3::new(pick(x), pick(square_y(z, x)), pick(z))
            }
            Turn::Tilted { x, y, z } => DVec3::new(x.dot(p), y.dot(p), z.dot(p)),
        }
    }

    /// Undo [`apply`](Self::apply).
    pub fn inverse(&self, q: DVec3) -> DVec3 {
        match *self {
            Turn::Square { z, x } => {
                let mut p = [0.0; 3];
                for (a, v) in [(x, q.x), (square_y(z, x), q.y), (z, q.z)] {
                    let (i, s) = a.parts();
                    if let Some(slot) = p.get_mut(i) {
                        *slot = if s > 0.0 { v } else { -v };
                    }
                }
                DVec3::from_array(p)
            }
            Turn::Tilted { x, y, z } => x * q.x + y * q.y + z * q.z,
        }
    }
}

/// The setup's Y, as a model axis, for square axes `z` and `x`.
fn square_y(z: Axis, x: Axis) -> Axis {
    let y = z.vector().cross(x.vector());
    let (i, s) = if y.x != 0.0 {
        (0, y.x)
    } else if y.y != 0.0 {
        (1, y.y)
    } else {
        (2, y.z)
    };
    Axis::from_parts(i, s)
}

/// A setup's frame: model coordinates turned by `turn`, then measured from
/// `origin` (a point in the turned coordinates).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Frame {
    /// How the part sits.
    pub turn: Turn,
    /// The work zero, in turned model coordinates.
    pub origin: DVec3,
    /// How the setup is unrolled, when it is machined on a rotary: its
    /// paths, part and stock are then in the unrolled frame ([`Unrolled`]).
    pub wrap: Option<Unrolled>,
}

/// A setup machined on a rotary that turns its round stock about its axis,
/// the tool pointing at the axis: its paths, part and stock are in the
/// cylinder unrolled. Along the axis is along it; round it is the
/// millimetres round at [`Unrolled::radius`], so the rotary's angle is
/// that over the radius; and Z is how far out from the axis.
///
/// The setup's work zero is on the axis, across it; along it, where the
/// setup puts it. Round is from the top, the side the tool comes down on,
/// towards the setup's Y (for a bar along X) or X (along Y).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Unrolled {
    /// The setup axis the bar lies along: X, turned by an A axis, or Y, by
    /// a B axis.
    pub along: camcraft_sim::stock::Axis,
    /// The radius a flat path is wrapped at: a millimetre round at it is a
    /// millimetre of the flat path.
    pub radius: f64,
}

impl Unrolled {
    /// A whole turn, in millimetres round.
    pub fn period(&self) -> f64 {
        std::f64::consts::TAU * self.radius
    }

    /// How the simulator's grid goes round.
    pub fn round(&self) -> camcraft_sim::stock::Round {
        use camcraft_sim::stock::Axis;
        camcraft_sim::stock::Round {
            round: if self.along == Axis::Y {
                Axis::X
            } else {
                Axis::Y
            },
            radius: self.radius,
        }
    }

    /// How far round an unrolled point is, in radians from the top.
    pub fn angle(&self, q: DVec3) -> f64 {
        let (_, round, _) = self.round().split(q);
        round / self.radius
    }

    /// A point of the setup, measured from its work zero on the axis, as
    /// unrolled.
    pub fn unroll(&self, p: DVec3) -> DVec3 {
        let r = if self.along == camcraft_sim::stock::Axis::Y {
            p.x.hypot(p.z)
        } else {
            p.y.hypot(p.z)
        };
        if self.along == camcraft_sim::stock::Axis::Y {
            DVec3::new(self.radius * p.x.atan2(p.z), p.y, r)
        } else {
            DVec3::new(p.x, self.radius * p.y.atan2(p.z), r)
        }
    }

    /// An unrolled point rolled back round the axis, in the setup's frame.
    pub fn roll(&self, q: DVec3) -> DVec3 {
        let (sin, cos) = self.angle(q).sin_cos();
        if self.along == camcraft_sim::stock::Axis::Y {
            DVec3::new(q.z * sin, q.y, q.z * cos)
        } else {
            DVec3::new(q.x, q.z * sin, q.z * cos)
        }
    }
}

impl Frame {
    /// A point of the setup's paths, part or stock in model coordinates:
    /// as [`Frame::to_model`], rolled back round the axis first when the
    /// setup is unrolled.
    pub fn shown(&self, q: DVec3) -> DVec3 {
        self.to_model(self.wrap.map_or(q, |w| w.roll(q)))
    }

    /// A model point in the setup's paths' frame: [`Frame::shown`] undone.
    pub fn unshown(&self, p: DVec3) -> DVec3 {
        let q = self.to_setup(p);
        self.wrap.map_or(q, |w| w.unroll(q))
    }

    /// A model point in the setup frame.
    pub fn to_setup(&self, p: DVec3) -> DVec3 {
        self.turn.apply(p) - self.origin
    }

    /// A setup point in model coordinates.
    pub fn to_model(&self, q: DVec3) -> DVec3 {
        self.turn.inverse(q + self.origin)
    }

    /// [`to_setup`](Self::to_setup) as a matrix, for drawing.
    pub fn matrix(&self) -> DMat4 {
        let [x, y, z] = self.turn.axes();
        // Rows are the setup axes; columns are what glam stores.
        DMat4::from_cols(
            DVec4::new(x.x, y.x, z.x, 0.0),
            DVec4::new(x.y, y.y, z.y, 0.0),
            DVec4::new(x.z, y.z, z.z, 0.0),
            (-self.origin).extend(1.0),
        )
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;
    use glam::dvec3;

    fn all() -> Vec<Orientation> {
        let mut out = Vec::new();
        for z in Axis::ALL {
            for x in Axis::ALL {
                let o = Orientation::square(z, x);
                if o.check().is_ok() {
                    out.push(o);
                }
            }
        }
        out
    }

    fn turn(o: &Orientation) -> Turn {
        o.turn().unwrap().unwrap()
    }

    #[test]
    fn there_are_24_proper_orientations() {
        let all = all();
        assert_eq!(all.len(), 24);
        for o in all {
            let t = turn(&o);
            assert!(t.square().is_some(), "{o:?}");
            let m = Frame {
                turn: t,
                origin: DVec3::ZERO,
                wrap: None,
            }
            .matrix();
            assert_eq!(m.determinant(), 1.0, "{o:?} is not a rotation");
        }
    }

    #[test]
    fn apply_matches_the_matrix_and_inverse_undoes_it() {
        let p = dvec3(1.25, -7.5, 3.0625);
        for o in all() {
            let f = Frame {
                turn: turn(&o),
                origin: dvec3(0.5, -2.0, 4.0),
                wrap: None,
            };
            let q = f.to_setup(p);
            assert_eq!(f.matrix().transform_point3(p), q, "{o:?}");
            assert_eq!(f.to_model(q), p, "{o:?}");
        }
    }

    #[test]
    fn the_default_changes_nothing() {
        let p = dvec3(0.1, 0.2, 0.3);
        assert_eq!(turn(&Orientation::default()).apply(p), p);
        assert_eq!(Frame::default().to_setup(p), p);
    }

    #[test]
    fn flipped_puts_the_underside_up() {
        let o = Orientation::default().flipped();
        assert!(o.check().is_ok());
        let q = turn(&o).apply(dvec3(1.0, 2.0, 3.0));
        assert_eq!(q, dvec3(1.0, -2.0, -3.0));
    }

    #[test]
    fn parallel_axes_are_refused() {
        for (z, x) in [(Axis::PosZ, Axis::NegZ), (Axis::PosX, Axis::PosX)] {
            assert!(Orientation::square(z, x).check().is_err());
        }
        let along = Orientation {
            z: Direction::Vector([1.0, 1.0, 0.0]),
            x: Direction::Vector([-2.0, -2.0, 0.0]),
        };
        assert!(matches!(
            along.check(),
            Err(BadOrientation::Parallel { .. })
        ));
        let nothing = Orientation {
            z: Direction::Vector([0.0; 3]),
            x: Direction::Axis(Axis::PosX),
        };
        assert!(matches!(
            nothing.check(),
            Err(BadOrientation::NoDirection { .. })
        ));
    }

    #[test]
    fn orientations_read_and_write_as_signed_axes() {
        let o = Orientation::square(Axis::NegZ, Axis::PosY);
        let text = toml::to_string(&o).unwrap();
        assert_eq!(text, "z = \"-z\"\nx = \"+y\"\n");
        assert_eq!(toml::from_str::<Orientation>(&text).unwrap(), o);
    }

    /// A vector, a face and an edge read and write as themselves.
    #[test]
    fn tilted_orientations_read_and_write() {
        let pick = FeatureRef {
            model: "m".into(),
            index: 3,
            at: [1.0, 2.0, 3.0],
        };
        for o in [
            Orientation {
                z: Direction::Vector([0.0, -0.5, 0.75]),
                x: Direction::Axis(Axis::PosX),
            },
            Orientation {
                z: Direction::Face {
                    face: pick.clone(),
                    reverse: false,
                },
                x: Direction::Edge {
                    edge: pick.clone(),
                    toward: Some([0.0, 1.0, 0.0]),
                    reverse: true,
                },
            },
        ] {
            let text = toml::to_string(&o).unwrap();
            assert_eq!(toml::from_str::<Orientation>(&text).unwrap(), o, "{text}");
        }
    }

    /// A vector along a model axis is tilted, as any vector is, but turns
    /// as exactly as the axis would.
    #[test]
    fn a_vector_along_an_axis_turns_exactly() {
        let o = Orientation {
            z: Direction::Vector([0.0, 0.0, -3.0]),
            x: Direction::Vector([2.0, 0.0, 0.0]),
        };
        let t = turn(&o);
        assert!(t.square().is_none());
        let p = dvec3(0.1, -0.7, 3.3);
        let square = turn(&Orientation::square(Axis::NegZ, Axis::PosX));
        assert_eq!(t.apply(p), square.apply(p));
        assert_eq!(t.inverse(p), square.inverse(p));
    }

    /// Tilted 30° about X, the tool axis comes out as Z, X stays X, and
    /// the frame is a rotation that its inverse and its matrix agree with.
    #[test]
    fn a_tilted_frame_is_a_rotation() {
        let (s, c) = 30f64.to_radians().sin_cos();
        let o = Orientation {
            z: Direction::Vector([0.0, -s, c]),
            x: Direction::Axis(Axis::PosX),
        };
        let t = turn(&o);
        assert!(t.square().is_none());
        let [x, y, z] = t.axes();
        assert!((x - DVec3::X).length() < 1e-15 && (z - dvec3(0.0, -s, c)).length() < 1e-15);
        assert!((y - dvec3(0.0, c, s)).length() < 1e-15);
        assert!((t.apply(dvec3(0.0, -s, c)) - DVec3::Z).length() < 1e-15);
        let f = Frame {
            turn: t,
            origin: dvec3(3.0, -1.0, 2.0),
            wrap: None,
        };
        assert!((f.matrix().determinant() - 1.0).abs() < 1e-12);
        let p = dvec3(1.25, -7.5, 3.0625);
        let q = f.to_setup(p);
        assert!((f.matrix().transform_point3(p) - q).length() < 1e-12);
        assert!((f.to_model(q) - p).length() < 1e-12);
    }

    /// Any tool axis and any X not along it make a right-handed frame with
    /// the tool up Z and X the one given made square to it.
    #[test]
    fn any_axis_makes_a_frame() {
        let dirs = [
            dvec3(0.3, -0.4, 0.8),
            dvec3(-1.0, 2.0, 0.5),
            dvec3(0.0, 1.0, 1.0),
            dvec3(0.2, 0.0, -1.0),
            dvec3(1.0, 1.0, 1.0),
        ];
        for z in dirs {
            for x in [DVec3::X, DVec3::Y, dvec3(1.0, -1.0, 0.25)] {
                let o = Orientation {
                    z: Direction::Vector(z.to_array()),
                    x: Direction::Vector(x.to_array()),
                };
                let t = turn(&o);
                let [ax, ay, az] = t.axes();
                assert!((az - z.normalize()).length() < 1e-12);
                assert!(ax.dot(az).abs() < 1e-12 && ay.dot(az).abs() < 1e-12);
                assert!((ax.cross(ay) - az).length() < 1e-12, "right-handed");
                // X is the one given, made square: in the plane of it and Z.
                assert!(ax.dot(x) > 0.0 && x.cross(az).dot(ax.cross(az)) > 0.0);
                assert!((t.apply(z.normalize()) - DVec3::Z).length() < 1e-12);
                let p = dvec3(4.0, -3.0, 9.0);
                assert!((t.inverse(t.apply(p)) - p).length() < 1e-12);
            }
        }
    }

    /// Unrolled and rolled back, a point is where it was; round from the
    /// top towards Y is the angle an A axis turns it up by.
    #[test]
    fn a_point_unrolls_and_rolls_back() {
        use camcraft_sim::stock::Axis as Bar;
        for along in [Bar::X, Bar::Y] {
            let w = Unrolled {
                along,
                radius: 20.0,
            };
            for p in [
                dvec3(3.0, 4.0, 15.0),
                dvec3(-7.0, -12.0, -5.0),
                dvec3(1.0, 2.0, 0.5),
            ] {
                let q = w.unroll(p);
                assert!((w.roll(q) - p).length() < 1e-12, "{along:?} {p}");
            }
        }
        let w = Unrolled {
            along: Bar::X,
            radius: 20.0,
        };
        // A quarter turn round, at the radius, is a quarter of the turn.
        let q = w.unroll(dvec3(5.0, 20.0, 0.0));
        assert!((q - dvec3(5.0, w.period() / 4.0, 20.0)).length() < 1e-12);
        assert!((w.angle(q) - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    }
}
