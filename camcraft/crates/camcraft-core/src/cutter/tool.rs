//! Tool geometry.
//!
//! A cutter is a solid of revolution, described canonically by **`r(z)`**: the
//! radius at height `z` above the tool tip. That form is fully general — it
//! describes flat, ball, bullnose, V-bit and tapered cutters, and equally the
//! undercutting ones (dovetail, T-slot, keyhole, lollipop) where `r` simply is not
//! monotonic.
//!
//! The drop-cutter wants the other direction: `h(d)`, the height at which the tool
//! surface sits at radial distance `d` from the axis. That is the generalised
//! inverse of `r`:
//!
//! ```text
//! h(d) = inf { z >= 0 : r(z) >= d }
//! ```
//!
//! For a monotonically non-decreasing `r` this is well defined and cheap. For an
//! undercutting tool it is not, because the tool reaches radius `d` at more than one
//! height, so [`ToolProfile::height_at`] returns `None` and the fast drop-cutter
//! path does not apply. Encoding that in the type is deliberate: it is what lets
//! undercutting tools arrive later without revisiting this trait.

use crate::is_positive;

/// A cutter's geometry, as a solid of revolution about the tool axis.
///
/// The tool tip is the origin and the controlled point — never the ball centre or
/// the holder gauge line. All lengths are millimetres.
pub trait ToolProfile {
    /// Radius of the tool at height `z` above the tip.
    ///
    /// Returns `0.0` below the tip, where there is no tool.
    fn radius_at(&self, z: f64) -> f64;

    /// The largest radius anywhere on the cutting portion.
    ///
    /// The drop-cutter uses this to bound its search footprint, so understating it
    /// means missed contacts, and missed contacts mean gouges.
    fn max_radius(&self) -> f64;

    /// Height of the tool surface at radial distance `d` from the axis: the
    /// generalised inverse `inf { z >= 0 : r(z) >= d }`.
    ///
    /// `None` when `d` exceeds [`max_radius`](ToolProfile::max_radius), or when the
    /// profile is not monotonic and the inverse is therefore ambiguous.
    fn height_at(&self, d: f64) -> Option<f64>;

    /// Whether `r(z)` is monotonically non-decreasing, and so `height_at` is defined
    /// across the whole radius range. False for undercutting tools.
    fn is_monotonic(&self) -> bool {
        true
    }
}

/// A flat (square-end) end mill: a plain cylinder.
///
/// `r(z) = R`, so `h(d) = 0` for every `d` within the radius — the whole cutting
/// face lies in one plane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flat {
    radius: f64,
}

/// A ball-nose end mill: a hemisphere of radius `R` below a shank of the same
/// radius.
///
/// `r(z) = sqrt(z(2R - z))` up to `z = R`, then `R`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ball {
    radius: f64,
}

/// A bullnose (corner-radius) end mill: a flat face of radius `R - c` surrounded by
/// a toroidal corner of radius `c`.
///
/// ```text
/// r(z) = (R - c) + sqrt(z(2c - z))   for 0 <= z <= c
///        R                           for z > c
/// ```
///
/// This shape subsumes the other two: `c = 0` is a [`Flat`] mill and `c = R` is a
/// [`Ball`]. Those keep their own dedicated implementations because both have fully
/// analytic contact tests, whereas the general bullnose does not — but the fact that
/// the general form must *agree* with them at those two limits is a strong check on
/// both, and the tests exercise it.
///
/// It also matters for stock-to-leave. Leaving stock `s` means machining with the
/// tool dilated by `s`, and a cylinder dilated by a ball is precisely a bullnose of
/// radius `R + s` and corner `s`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bullnose {
    radius: f64,
    corner: f64,
}

impl Bullnose {
    /// Build a bullnose mill of overall radius `radius` and corner radius `corner`.
    ///
    /// `corner` must lie in `[0, radius]`; outside that the shape is not a cutter.
    pub fn new(radius: f64, corner: f64) -> Result<Self, ToolError> {
        let radius = check_radius(radius)?;
        if !corner.is_finite() || corner < 0.0 || corner > radius {
            return Err(ToolError::BadCorner { corner, radius });
        }
        Ok(Self { radius, corner })
    }

    /// Radius of the flat portion of the face, `R - c`.
    pub fn flat_radius(&self) -> f64 {
        self.radius - self.corner
    }

    /// The corner radius.
    pub fn corner(&self) -> f64 {
        self.corner
    }
}

impl ToolProfile for Bullnose {
    fn radius_at(&self, z: f64) -> f64 {
        let (r, c) = (self.radius, self.corner);
        if z < 0.0 {
            0.0
        } else if z >= c {
            r
        } else {
            (r - c) + (z * (2.0 * c - z)).max(0.0).sqrt()
        }
    }

    fn max_radius(&self) -> f64 {
        self.radius
    }

    fn height_at(&self, d: f64) -> Option<f64> {
        let (r, c) = (self.radius, self.corner);
        if d > r {
            return None;
        }
        let flat = r - c;
        if d <= flat {
            return Some(0.0);
        }
        let k = d - flat;
        Some(c - (c * c - k * k).max(0.0).sqrt())
    }
}

/// Cutters expressible in the bullnose family.
///
/// Flat, ball and corner-radius mills are each a flat face of radius `R - c`
/// surrounded by a corner of radius `c`. Stating that as a trait is what lets
/// stock-to-leave be computed once, for all of them, instead of per shape — the
/// family is closed under dilation by the stock solid. A [`Cone`] is not in it.
pub trait BullnoseFamily {
    /// This cutter as a bullnose. Infallible: the dimensions were validated when the
    /// cutter was built.
    fn as_bullnose(&self) -> Bullnose;
}

impl BullnoseFamily for Bullnose {
    fn as_bullnose(&self) -> Bullnose {
        *self
    }
}

impl BullnoseFamily for Flat {
    fn as_bullnose(&self) -> Bullnose {
        Bullnose {
            radius: self.radius,
            corner: 0.0,
        }
    }
}

impl BullnoseFamily for Ball {
    fn as_bullnose(&self) -> Bullnose {
        Bullnose {
            radius: self.radius,
            corner: self.radius,
        }
    }
}

/// A cone: a V-bit, chamfer mill or drill point. A flat tip of radius `tip`
/// widening at half-angle `α` to the full radius `R`, then a shank of radius `R`.
///
/// ```text
/// h(d) = 0                     for d <= tip
///        (d - tip) / tan(α)    for tip < d <= R
/// ```
///
/// `h` is convex, which is what makes its drop-cutter contacts exact or bounded
/// solves, as the bullnose's are. A cone is not closed under stock to leave: grown
/// by a ball, its tip rounds, so a cone cuts to size only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cone {
    radius: f64,
    tip: f64,
    /// `1 / tan(α)`: how far the flank rises for each unit out.
    rise: f64,
}

impl Cone {
    /// A cone of overall radius `radius`, tip radius `tip` and half-angle
    /// `half_angle` radians from the axis: 45° for a 90° V-bit.
    pub fn new(radius: f64, tip: f64, half_angle: f64) -> Result<Self, ToolError> {
        let radius = check_radius(radius)?;
        if !(half_angle.is_finite() && half_angle > 0.0 && half_angle < std::f64::consts::FRAC_PI_2)
        {
            return Err(ToolError::BadAngle(half_angle.to_degrees() * 2.0));
        }
        if !(tip.is_finite() && tip >= 0.0 && tip < radius) {
            return Err(ToolError::BadTip { tip, radius });
        }
        Ok(Self {
            radius,
            tip,
            rise: 1.0 / half_angle.tan(),
        })
    }

    /// The flat tip's radius.
    pub fn tip(&self) -> f64 {
        self.tip
    }

    /// Height of the flank per unit of radius: `1 / tan(α)`.
    pub fn rise(&self) -> f64 {
        self.rise
    }

    /// Height above the tip where the flank reaches full radius.
    #[cfg(test)]
    pub(crate) fn flank_height(&self) -> f64 {
        (self.radius - self.tip) * self.rise
    }
}

impl ToolProfile for Cone {
    fn radius_at(&self, z: f64) -> f64 {
        if z < 0.0 {
            0.0
        } else {
            (self.tip + z / self.rise).min(self.radius)
        }
    }

    fn max_radius(&self) -> f64 {
        self.radius
    }

    fn height_at(&self, d: f64) -> Option<f64> {
        if d > self.radius {
            return None;
        }
        Some((d - self.tip).max(0.0) * self.rise)
    }
}

/// A tapered end: a flat tip of radius `flat`, a round corner of radius
/// `corner` about it, and a straight flank tangent to the corner, `α` out
/// from the axis, widening to the full radius `R`; then a shank of radius
/// `R`. A tapered or tapered ball end mill (no flat), a V-bit with a round
/// tip, and a face mill whose inserts have a lead angle are each one.
///
/// ```text
/// h(d) = 0                                for d <= a
///        c - sqrt(c^2 - (d - a)^2)        for a < d <= a + c cos α
///        c (1 - sin α) + (d - a - c cos α) / tan α     up to R
/// ```
///
/// The corner ends where its slope is the flank's, so `h` is convex, as
/// the cone's and the bullnose's are, and its drop-cutter contacts are the
/// same bounded solves. Grown by stock to leave it stays one: its corner
/// grows by the stock, its flat by the radial stock past that, and its
/// flank moves out with them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tapered {
    radius: f64,
    flat: f64,
    corner: f64,
    /// `sin α`, `cos α` and `tan α`, `α` the flank's angle from the axis.
    sin: f64,
    cos: f64,
    tan: f64,
}

impl Tapered {
    /// A tapered end of overall radius `radius`, flat tip radius `flat`,
    /// corner `corner`, its flank `half_angle` radians from the axis.
    pub fn new(radius: f64, flat: f64, corner: f64, half_angle: f64) -> Result<Self, ToolError> {
        let radius = check_radius(radius)?;
        if !(half_angle.is_finite() && half_angle > 0.0 && half_angle < std::f64::consts::FRAC_PI_2)
        {
            return Err(ToolError::BadTaper(half_angle.to_degrees()));
        }
        if !(corner.is_finite() && corner >= 0.0) {
            return Err(ToolError::BadCorner { corner, radius });
        }
        let (sin, cos) = half_angle.sin_cos();
        if !(flat.is_finite() && flat >= 0.0 && flat + corner * cos <= radius + 1e-9) {
            return Err(ToolError::BadTip {
                tip: flat + corner * cos,
                radius,
            });
        }
        Ok(Self {
            radius,
            flat,
            corner,
            sin,
            cos,
            tan: half_angle.tan(),
        })
    }

    /// The flat tip's radius.
    pub fn flat(&self) -> f64 {
        self.flat
    }

    /// The corner's radius.
    pub fn corner(&self) -> f64 {
        self.corner
    }

    /// The flank's angle from the axis, in radians.
    pub fn half_angle(&self) -> f64 {
        self.tan.atan()
    }

    /// Where the corner meets the flank: its radius and height.
    fn shoulder(&self) -> (f64, f64) {
        (
            self.flat + self.corner * self.cos,
            self.corner * (1.0 - self.sin),
        )
    }

    /// Height above the tip where the flank reaches full radius.
    pub fn flank_top(&self) -> f64 {
        let (rt, zt) = self.shoulder();
        zt + (self.radius - rt) / self.tan
    }

    /// The radius at which a plane rising at slope `s` along its steepest
    /// line touches it first: on the corner where its slope is the plane's,
    /// or past the flank, at the full radius, for a plane steeper than it.
    pub(crate) fn touches_slope(&self, s: f64) -> f64 {
        if s * self.tan >= 1.0 {
            self.radius
        } else {
            self.flat + self.corner * s / (1.0 + s * s).sqrt()
        }
    }

    /// The same end grown by stock to leave: `radial` beside, `least` the
    /// lesser of the two, all round the corner. `None` where shrunk it is
    /// no longer a cutter.
    fn grown(&self, radial: f64, least: f64) -> Option<Self> {
        let corner = (self.corner + least).max(0.0);
        let flat = (self.flat + radial - least).max(0.0);
        Self::new(self.radius + radial, flat, corner, self.half_angle()).ok()
    }
}

impl ToolProfile for Tapered {
    fn radius_at(&self, z: f64) -> f64 {
        let (rt, zt) = self.shoulder();
        if z < 0.0 {
            0.0
        } else if z <= zt {
            self.flat + (z * (2.0 * self.corner - z)).max(0.0).sqrt()
        } else {
            (rt + (z - zt) * self.tan).min(self.radius)
        }
    }

    fn max_radius(&self) -> f64 {
        self.radius
    }

    fn height_at(&self, d: f64) -> Option<f64> {
        if d > self.radius {
            return None;
        }
        let (rt, zt) = self.shoulder();
        let c = self.corner;
        Some(if d <= self.flat {
            0.0
        } else if d <= rt {
            let k = d - self.flat;
            c - (c * c - k * k).max(0.0).sqrt()
        } else {
            zt + (d - rt) / self.tan
        })
    }
}

/// An undercutting head on a narrower neck: a slot (key) cutter, a disc
/// `height` thick, or a lollipop, a ball. From below it is its head, a
/// flat or ball end of the head's radius, which is what a drop and an
/// offset see. Above the head it narrows to its neck, so the head reaches
/// in under what overhangs it: its radius is not monotonic, and
/// [`ToolProfile::height_at`] gives only its underside.
///
/// A height field cannot hold an undercut, so what the material loses to it
/// is taken as only what its neck's width takes from above
/// ([`Cutter::swept_height_at`]); the neck and shank above the head are
/// checked against the material as a holder is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Undercut {
    head: Bullnose,
    neck: f64,
    height: f64,
}

impl Undercut {
    /// A head `head`, `height` tall, on a neck of radius `neck`.
    pub fn new(head: Bullnose, neck: f64, height: f64) -> Result<Self, ToolError> {
        let radius = head.max_radius();
        if !(neck.is_finite() && neck > 0.0 && neck < radius) {
            return Err(ToolError::BadNeck { neck, radius });
        }
        if !is_positive(height) {
            return Err(ToolError::BadRadius(height));
        }
        Ok(Self { head, neck, height })
    }

    /// The head, as its underside cuts.
    pub fn head(&self) -> &Bullnose {
        &self.head
    }

    /// The neck's radius.
    pub fn neck(&self) -> f64 {
        self.neck
    }

    /// How tall the head is, from the tip to where the neck starts.
    pub fn height(&self) -> f64 {
        self.height
    }
}

impl ToolProfile for Undercut {
    fn radius_at(&self, z: f64) -> f64 {
        if z > self.height {
            self.neck
        } else if self.head.corner() > 0.0 && z > self.head.corner() {
            // A ball's upper half, narrowing back in to the neck.
            let r = self.head.max_radius();
            let up = z - self.head.corner();
            (r * r - up * up).max(0.0).sqrt().max(self.neck)
        } else {
            self.head.radius_at(z)
        }
    }

    fn max_radius(&self) -> f64 {
        self.head.max_radius()
    }

    fn height_at(&self, d: f64) -> Option<f64> {
        self.head.height_at(d)
    }

    fn is_monotonic(&self) -> bool {
        false
    }
}

/// Any cutter the engine cuts with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cutter {
    /// Flat, ball and corner-radius end mills.
    Bullnose(Bullnose),
    /// V-bits, chamfer mills and drill points.
    Cone(Cone),
    /// Tapered end mills, round-tipped V-bits and face mills with a lead.
    Tapered(Tapered),
    /// Slot cutters and lollipops.
    Undercut(Undercut),
}

impl Cutter {
    /// The cutter as a bullnose, when it is one.
    #[cfg(test)]
    pub(crate) fn bullnose(&self) -> Option<&Bullnose> {
        match self {
            Cutter::Bullnose(b) => Some(b),
            Cutter::Cone(_) | Cutter::Tapered(_) | Cutter::Undercut(_) => None,
        }
    }

    /// Widest the material is cut beside the axis: an undercutting head's
    /// neck, which is all a height field sees it take; any other's radius.
    pub fn swept_max_radius(&self) -> f64 {
        match self {
            Cutter::Undercut(u) => u.neck(),
            _ => self.max_radius(),
        }
    }

    /// Height the material is cut to at `d` from the axis, above the tip:
    /// under an undercutting head's neck, its underside, and nothing
    /// further out, where the head reaches only under what stays.
    pub fn swept_height_at(&self, d: f64) -> Option<f64> {
        match self {
            Cutter::Undercut(u) if d > u.neck() => None,
            _ => self.height_at(d),
        }
    }

    /// The same cutter grown by stock to leave, `radial` beside and
    /// `axial` down, by the rounded cylinder [`super::allowance`] says;
    /// `None` for a cone, which grown is no longer one.
    pub(crate) fn grown(&self, radial: f64, axial: f64) -> Option<Result<Self, ToolError>> {
        let m = radial.min(axial);
        let bullnose =
            |b: &Bullnose| Bullnose::new(b.max_radius() + radial, (b.corner() + m).max(0.0));
        Some(match self {
            Cutter::Bullnose(b) => bullnose(b).map(Cutter::Bullnose),
            Cutter::Cone(_) => return None,
            Cutter::Tapered(t) => t
                .grown(radial, m)
                .map(Cutter::Tapered)
                .ok_or(ToolError::BadRadius(t.max_radius() + radial)),
            // Only its head is ever dropped, so its neck stays as it is,
            // even past a head shrunk by negative stock.
            Cutter::Undercut(u) => bullnose(u.head()).map(|head| {
                Cutter::Undercut(Undercut {
                    head,
                    neck: u.neck(),
                    height: u.height(),
                })
            }),
        })
    }
}

impl From<Tapered> for Cutter {
    fn from(t: Tapered) -> Self {
        Cutter::Tapered(t)
    }
}

impl From<Undercut> for Cutter {
    fn from(u: Undercut) -> Self {
        Cutter::Undercut(u)
    }
}

impl From<Bullnose> for Cutter {
    fn from(b: Bullnose) -> Self {
        Cutter::Bullnose(b)
    }
}

impl From<Flat> for Cutter {
    fn from(f: Flat) -> Self {
        Cutter::Bullnose(f.as_bullnose())
    }
}

impl From<Ball> for Cutter {
    fn from(b: Ball) -> Self {
        Cutter::Bullnose(b.as_bullnose())
    }
}

impl From<Cone> for Cutter {
    fn from(c: Cone) -> Self {
        Cutter::Cone(c)
    }
}

impl ToolProfile for Cutter {
    fn radius_at(&self, z: f64) -> f64 {
        match self {
            Cutter::Bullnose(t) => t.radius_at(z),
            Cutter::Cone(t) => t.radius_at(z),
            Cutter::Tapered(t) => t.radius_at(z),
            Cutter::Undercut(t) => t.radius_at(z),
        }
    }

    fn max_radius(&self) -> f64 {
        match self {
            Cutter::Bullnose(t) => t.max_radius(),
            Cutter::Cone(t) => t.max_radius(),
            Cutter::Tapered(t) => t.max_radius(),
            Cutter::Undercut(t) => t.max_radius(),
        }
    }

    fn height_at(&self, d: f64) -> Option<f64> {
        match self {
            Cutter::Bullnose(t) => t.height_at(d),
            Cutter::Cone(t) => t.height_at(d),
            Cutter::Tapered(t) => t.height_at(d),
            Cutter::Undercut(t) => t.height_at(d),
        }
    }

    fn is_monotonic(&self) -> bool {
        !matches!(self, Cutter::Undercut(_))
    }
}

/// A tool dimension that was not physically meaningful.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ToolError {
    /// A radius was zero, negative, or not finite.
    #[error("tool radius must be finite and greater than zero, got {0}")]
    BadRadius(f64),
    /// A corner radius did not fit within the tool.
    #[error("corner radius must be finite and within [0, {radius}], got {corner}")]
    BadCorner {
        /// The offending corner radius.
        corner: f64,
        /// The tool radius it had to fit inside.
        radius: f64,
    },
    /// A cone's included angle was not between 0° and 180°.
    #[error("the angle must be between 0 and 180 degrees, got {0}")]
    BadAngle(f64),
    /// A cone's tip did not fit within the tool.
    #[error(
        "the tip must be at least zero and narrower than the tool ({radius} radius), got {tip}"
    )]
    BadTip {
        /// The offending tip radius.
        tip: f64,
        /// The tool radius it had to fit inside.
        radius: f64,
    },
    /// A taper or lead was not between 0° and 90° from the axis.
    #[error("the taper must be more than 0 and less than 90 degrees from the axis, got {0}")]
    BadTaper(f64),
    /// An undercutting head's neck was not narrower than the head.
    #[error("the neck must be narrower than the head ({radius} radius), got {neck}")]
    BadNeck {
        /// The neck's radius.
        neck: f64,
        /// The head's radius.
        radius: f64,
    },
    /// Stock to leave was asked of a cone, which cannot be grown and stay a cone.
    #[error("a V-bit, chamfer mill or drill cuts to size: it cannot leave stock")]
    ConeStock,
}

fn check_radius(radius: f64) -> Result<f64, ToolError> {
    if is_positive(radius) {
        Ok(radius)
    } else {
        Err(ToolError::BadRadius(radius))
    }
}

impl Flat {
    /// Build a flat end mill of the given radius in millimetres.
    pub fn new(radius: f64) -> Result<Self, ToolError> {
        check_radius(radius).map(|radius| Self { radius })
    }
}

impl Ball {
    /// Build a ball-nose end mill of the given radius in millimetres.
    pub fn new(radius: f64) -> Result<Self, ToolError> {
        check_radius(radius).map(|radius| Self { radius })
    }
}

impl ToolProfile for Flat {
    fn radius_at(&self, z: f64) -> f64 {
        if z < 0.0 { 0.0 } else { self.radius }
    }

    fn max_radius(&self) -> f64 {
        self.radius
    }

    fn height_at(&self, d: f64) -> Option<f64> {
        (d <= self.radius).then_some(0.0)
    }
}

impl ToolProfile for Ball {
    fn radius_at(&self, z: f64) -> f64 {
        let r = self.radius;
        if z < 0.0 {
            0.0
        } else if z >= r {
            r
        } else {
            // sqrt(R^2 - (R - z)^2), arranged to stay accurate for small z.
            (z * (2.0 * r - z)).max(0.0).sqrt()
        }
    }

    fn max_radius(&self) -> f64 {
        self.radius
    }

    fn height_at(&self, d: f64) -> Option<f64> {
        let r = self.radius;
        if d > r {
            return None;
        }
        // The ball centre sits at z = R, so the surface at radius d is at
        // R - sqrt(R^2 - d^2).
        Some(r - (r * r - d * d).max(0.0).sqrt())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp, clippy::panic)]
mod tests;
