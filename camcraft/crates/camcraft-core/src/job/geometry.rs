//! What an operation machines: Fusion's Geometry tab, which is three settings
//! here: the picks, a 3D machining boundary and a 2D outline.

use serde::{Deserialize, Serialize};

use super::{FeatureRef, Operation, is_default};

/// What a 3D operation is kept to, seen from above: Fusion's boundary mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Boundary {
    /// The stock's footprint: the whole part, and everything around it.
    #[default]
    Stock,
    /// The rectangle around the setup's models.
    BoundingBox,
    /// The setup's models' outline: their shadow from straight above.
    Silhouette,
    /// The picked faces seen from above, and the loops the picked edges close.
    Selection,
}

/// What a 2D operation cuts round or clears, seen from above: Fusion's contour
/// or pocket selection. Unlike a [`MachiningBoundary`] it does not fence the tool
/// in; the tool runs beside it, on the side the strategy says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outline {
    /// The setup's models' outline: their shadow from straight above.
    #[default]
    Silhouette,
    /// The picked faces seen from above, and the loops the picked edges close.
    Selection,
}

impl From<Outline> for Boundary {
    fn from(o: Outline) -> Self {
        match o {
            Outline::Silhouette => Boundary::Silhouette,
            Outline::Selection => Boundary::Selection,
        }
    }
}

/// Where the tool may go, against the machining boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Containment {
    /// The tool's centre stays inside the boundary, so its side reaches past it by
    /// its radius. Fusion's default.
    #[default]
    Center,
    /// The whole tool stays inside.
    Inside,
    /// The tool may go its whole width outside, so its side still reaches the
    /// boundary from beyond it.
    Outside,
}

/// How a hole operation (drilling, boring, circular milling, threading)
/// finds its holes beyond those picked, and the order it takes them in:
/// Fusion's Select Same Diameter and its ordering.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HolePicking {
    /// Every hole of the setup's models as wide as one picked, its axis
    /// along the tool's, taken as well; a boss's, every boss as wide.
    #[serde(default, skip_serializing_if = "is_default")]
    pub same_diameter: bool,
    /// Every hole of the setup's models this wide, in millimetres, its axis
    /// along the tool's, whether or not one is picked: a rule that needs no
    /// pick, so it holds for whatever part the operation is put on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diameter: Option<f64>,
    /// The order they are cut in.
    #[serde(default, skip_serializing_if = "is_default")]
    pub order: HoleOrder,
}

/// The order a hole operation takes its holes in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HoleOrder {
    /// The first picked, then always the nearest not yet cut.
    #[default]
    Nearest,
    /// In columns along X, each column along Y, up one and down the next.
    XThenY,
    /// As picked, those found by their diameter after.
    AsPicked,
}

/// Faces and edges picked on the models, and curves drawn beside them.
///
/// What they mean is for the rest of the operation to say, which is why they live
/// here and not with the strategy: a boundary or outline of the selection is
/// their outline from above, a height can be measured from them, a drill's are
/// the rims of its holes, an engraving's are what it follows.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    /// Faces picked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub faces: Vec<FeatureRef>,
    /// Edges picked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<FeatureRef>,
    /// Curves drawn, a sketch's, in the coordinates the models are placed in.
    /// They are on no model, so are not found again: they are what they say.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub curves: Vec<Curve>,
}

impl Selection {
    /// Whether nothing is picked.
    pub fn is_empty(&self) -> bool {
        self.faces.is_empty() && self.edges.is_empty() && self.curves.is_empty()
    }

    /// Every pick, faces first.
    pub fn iter(&self) -> impl Iterator<Item = &FeatureRef> {
        self.faces.iter().chain(&self.edges)
    }
}

/// A curve of lines and arcs, end to end from `start`, in the coordinates the
/// models are placed in. It is closed when it ends where it starts. A spline
/// comes as the lines that follow it within the job's tolerance.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Curve {
    /// Where it starts.
    pub start: [f64; 3],
    /// Its pieces, in order, each from where the one before ends.
    pub segments: Vec<Segment>,
}

/// One piece of a [`Curve`].
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Segment {
    /// The circular arc from where the curve is through `via` to `to`.
    Arc {
        /// A point on it between its ends.
        via: [f64; 3],
        /// Where it ends.
        to: [f64; 3],
    },
    /// A straight line to `to`.
    Line {
        /// Where it ends.
        to: [f64; 3],
    },
}

impl Segment {
    /// Where it ends.
    pub fn to(&self) -> [f64; 3] {
        match *self {
            Segment::Arc { to, .. } | Segment::Line { to } => to,
        }
    }
}

impl Curve {
    /// Where it ends.
    pub fn end(&self) -> [f64; 3] {
        self.segments.last().map_or(self.start, Segment::to)
    }

    /// Whether it ends where it starts, within `tolerance`.
    pub fn closed(&self, tolerance: f64) -> bool {
        let (a, b) = (self.start, self.end());
        !self.segments.is_empty()
            && (a[0] - b[0]).hypot(a[1] - b[1]).hypot(a[2] - b[2]) <= tolerance
    }

    /// Every point it gives: its start, and each segment's middle and end.
    pub fn points(&self) -> impl Iterator<Item = [f64; 3]> + '_ {
        std::iter::once(self.start).chain(self.segments.iter().flat_map(|s| match *s {
            Segment::Arc { via, to } => vec![via, to],
            Segment::Line { to } => vec![to],
        }))
    }
}

/// Where a 3D operation's tool may go: Fusion's machining boundary, where the
/// tool may go against it, and how far past it.
///
/// The stock's footprint, the default, machines the whole part and everything
/// around it; the models' bounding box or silhouette keeps the tool near the
/// part; the selection keeps it to the picks' outline.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MachiningBoundary {
    /// What the tool is kept to.
    #[serde(default, skip_serializing_if = "is_default")]
    pub kind: Boundary,
    /// Where the tool may go against it.
    #[serde(default, skip_serializing_if = "is_default")]
    pub containment: Containment,
    /// Fusion's additional offset, in millimetres: how far the boundary moves
    /// out, or in when the whole tool is kept inside it. Past the stock's
    /// footprint, with no boundary.
    #[serde(default, skip_serializing_if = "is_default")]
    pub margin: f64,
}

impl Default for MachiningBoundary {
    fn default() -> Self {
        Self {
            kind: Boundary::Stock,
            containment: Containment::Center,
            margin: 0.0,
        }
    }
}

impl Operation {
    /// Whether what it cuts is its selection: a 2D outline or a 3D boundary of
    /// it, a drill's holes, or what an engraving follows or a V-carve cuts.
    pub fn cuts_selection(&self) -> bool {
        let uses = self.strategy.uses();
        uses.holes
            || uses.traces
            || uses.carves
            || (uses.outline && self.outline == Outline::Selection)
            || (uses.boundary && self.boundary.kind == Boundary::Selection)
    }
}
