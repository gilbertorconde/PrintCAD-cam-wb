//! Chamfering, or deburring: a chamfer mill or V-bit run along edges, its
//! flank cutting the corner off. Fusion's 2D Chamfer.
//!
//! # Where the tool goes
//!
//! A chamfer `width` wide, seen from above, on a cone of half angle α, is
//! `width / tan α` deep. The tool's tip goes the tip offset below that, so
//! the chamfer is cut on the flank, clear of the tip's corner. There the cone
//! is the tip's radius plus the tip offset's worth of flank across, which is
//! how far the tool's centre stays from the edge, away from the material.
//! At the edge's height the cone is that and the width across: no wider
//! than the tool, or the chamfer needs more cone than it has
//! ([`Place::new`]).
//!
//! # Which side
//!
//! Away from the material: for each curve, the part is looked at a little
//! way either side of it, and the side it is lower on is away. Flipped,
//! the other. A curve with the part as high either side, as a sketch drawn
//! on a flat, cannot tell: it goes round a closed one outside, and along an
//! open one on its right, and says so.
//!
//! Each curve is then moved that far to its side, its lines and arcs moved
//! square to themselves ([`camcraft_geom::ring`]), and run so that cutting
//! climbs, and cut as an engraving is, down to the tip's depth below its
//! own height, the edge's.

use camcraft_geom::ring::{Ring, Span, offset_open};
use glam::DVec2;

use crate::strategy::StrategyError;
use crate::strategy::engrave::{Piece, Trace};

/// How far apart, in millimetres, two points may be and still be one.
const SAME_POINT: f64 = 1.0e-6;

/// Where a chamfer puts a cone.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Place {
    /// How far the tool's centre is from the edge, seen from above, away
    /// from the material: the cone's radius at the chamfer's foot.
    pub offset: f64,
    /// How far below the edge the tip goes: the chamfer's depth and the tip
    /// offset.
    pub depth: f64,
    /// The cone's radius at the edge's height: the offset and the width.
    pub reach: f64,
}

/// A chamfer asked of a cone too small for it.
#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
#[error("the chamfer needs the cone {needs:.2} mm across at the edge, and the tool is {has:.2} mm")]
pub struct TooSmall {
    /// How wide the cone has to be at the edge, in millimetres.
    pub needs: f64,
    /// How wide the tool is.
    pub has: f64,
}

impl Place {
    /// Where a chamfer `width` wide goes on a cone of `half_angle` radians
    /// from its axis, its flat tip `tip` in radius and `radius` in all, the
    /// tip `tip_offset` below the chamfer's foot.
    pub fn new(
        width: f64,
        tip_offset: f64,
        half_angle: f64,
        tip: f64,
        radius: f64,
    ) -> Result<Self, TooSmall> {
        let tan = half_angle.tan();
        let offset = tip + tip_offset * tan;
        let reach = offset + width;
        // Not a number, from a cone with no angle, is no fit either.
        if reach.is_nan() || reach > radius + 1e-9 {
            return Err(TooSmall {
                needs: 2.0 * reach,
                has: 2.0 * radius,
            });
        }
        Ok(Self {
            offset,
            depth: width / tan + tip_offset,
            reach,
        })
    }
}

/// Which side of each curve the material is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// On its left, as it runs.
    Left,
    /// On its right.
    Right,
}

/// What finding the tool's centre's curves found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Centres {
    /// Curves the part is as high either side of, whose side was guessed.
    pub guessed: u64,
    /// Curves that could not be moved to their side: too tight a turn for
    /// the offset, an arc shrunk past its centre.
    pub dropped: u64,
}

/// Which side of `trace` the material is on, by `height`, the part's
/// height at a point seen from above (`None` off the part), looked at
/// `aside` either side of its pieces' middles; `None` where it cannot tell.
pub fn material_side(
    trace: &Trace,
    height: &dyn Fn(DVec2) -> Option<f64>,
    aside: f64,
    tolerance: f64,
) -> Option<Side> {
    let mut left = 0.0;
    let mut right = 0.0;
    let (start, spans) = spans_of(trace);
    let mut at = start;
    for s in &spans {
        let mid = s.point_at(at, 0.5);
        let t = s.tangent_at(at, 0.5);
        at = s.to();
        if t.length_squared() < 0.5 {
            continue;
        }
        let side = |d: DVec2| height(mid + d * aside).unwrap_or(f64::MIN);
        let (l, r) = (side(t.perp()), side(-t.perp()));
        if (l - r).abs() > tolerance {
            if l > r {
                left += 1.0;
            } else {
                right += 1.0;
            }
        }
    }
    if left > right {
        Some(Side::Left)
    } else if right > left {
        Some(Side::Right)
    } else {
        None
    }
}

/// The curves the tool's centre follows to chamfer `traces`: each moved
/// `offset` away from the material (as `side_of` says, or `flipped`, the
/// other side), run so that its cutting climbs (`climb_right`: climbing has
/// what it cuts on the tool's right), at its own height.
pub fn centres(
    traces: &[Trace],
    side_of: &dyn Fn(&Trace) -> Option<Side>,
    offset: f64,
    flipped: bool,
    climb_right: bool,
    tolerance: f64,
) -> (Vec<Trace>, Centres) {
    let mut out = Vec::with_capacity(traces.len());
    let mut report = Centres::default();
    for t in traces.iter().filter(|t| !t.pieces.is_empty()) {
        let closed = t.closed(tolerance);
        let side = side_of(t).unwrap_or_else(|| {
            report.guessed += 1;
            // Outside a closed curve; an open one, on its right.
            if closed && t.signed_area(tolerance) < 0.0 {
                Side::Right
            } else {
                Side::Left
            }
        });
        let material = match (side, flipped) {
            (Side::Left, false) | (Side::Right, true) => Side::Left,
            _ => Side::Right,
        };
        // Run with the material on the side climbing cuts on.
        let wanted = if climb_right { Side::Right } else { Side::Left };
        let t = if material == wanted {
            t.clone()
        } else {
            t.reversed()
        };
        // Away from it: to the left of its run with it on the right.
        let d = if wanted == Side::Right {
            -offset
        } else {
            offset
        };
        match moved(&t, d, closed) {
            Some(m) => out.push(m),
            None => report.dropped += 1,
        }
    }
    (out, report)
}

/// `trace` moved `d` to its right, square to every piece, as a ring is
/// ([`Ring::offset`]): at its start's height.
fn moved(trace: &Trace, d: f64, closed: bool) -> Option<Trace> {
    let z = trace.start.z;
    let (start, spans) = spans_of(trace);
    let (start, spans) = if closed {
        let mut ring = spans.clone();
        // A ring closes on its start exactly.
        if let Some(last) = ring.last_mut() {
            *last = ending_at(*last, start);
        }
        let ring = Ring::new(start, ring)?.offset(d)?;
        (ring.start(), ring.spans().to_vec())
    } else {
        offset_open(start, &spans, d)?
    };
    let pieces = spans
        .iter()
        .map(|s| match *s {
            Span::Line { to } => Piece::Line { to: to.extend(z) },
            Span::Arc { to, centre, sweep } => Piece::Arc {
                centre,
                ccw: sweep > 0.0,
                to: to.extend(z),
            },
        })
        .collect();
    Some(Trace {
        start: start.extend(z),
        pieces,
    })
}

/// `s` with its end moved to `to`.
fn ending_at(s: Span, to: DVec2) -> Span {
    match s {
        Span::Line { .. } => Span::Line { to },
        Span::Arc { centre, sweep, .. } => Span::Arc { to, centre, sweep },
    }
}

/// `trace` seen from above, as spans: where it starts, and its lines and
/// arcs, each arc's sweep the way it turns, a whole turn where its ends
/// meet.
fn spans_of(trace: &Trace) -> (DVec2, Vec<Span>) {
    let start = trace.start.truncate();
    let mut at = start;
    let mut out = Vec::with_capacity(trace.pieces.len());
    for p in &trace.pieces {
        let to = p.to().truncate();
        match *p {
            Piece::Line { .. } => {
                if to.distance(at) > SAME_POINT {
                    out.push(Span::Line { to });
                }
            }
            Piece::Arc { centre, ccw, .. } => {
                let (a, b) = (at - centre, to - centre);
                let turn = a.perp_dot(b).atan2(a.dot(b));
                let tau = std::f64::consts::TAU;
                let sweep = if ccw {
                    let s = turn.rem_euclid(tau);
                    if s < 1e-9 { tau } else { s }
                } else {
                    let s = (-turn).rem_euclid(tau);
                    -(if s < 1e-9 { tau } else { s })
                };
                out.push(Span::Arc { to, centre, sweep });
            }
        }
        at = to;
    }
    (start, out)
}

/// A [`StrategyError`] for a chamfer too big for its cone.
impl From<TooSmall> for StrategyError {
    fn from(e: TooSmall) -> Self {
        StrategyError::BadParameter {
            name: "chamfer width",
            value: e.needs,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp, clippy::indexing_slicing)]
mod tests;
