//! Corner relief: dogbones and T-bones, cut into the inside corners a round
//! tool leaves round, so a square part fits them.
//!
//! # Where a corner is
//!
//! A 2D cut's tool centre runs a set distance from the wall it leaves. Where
//! the wall has an inside corner, the centre's path turns a sharp corner too,
//! away from the wall, and the tool sweeps round it, leaving the wall's corner
//! round, its radius the tool's. Where the wall goes round an outside corner,
//! the path goes round an arc instead, and nothing is left. So the corners to
//! relieve are the sharp corners of the centre's path whose outside is the
//! material kept: the right of the path, the left, or for a slot, which keeps
//! both, either.
//!
//! # How far
//!
//! A tool its centre's distance from the wall, `r`, reaches the wall's
//! corner, `r / sin(a/2)` from the centre's corner where the walls meet at
//! `a`, from:
//!
//! - a dogbone: `r / sin(a/2) − r` out halfway between the walls, the
//!   notch round and reaching the corner's point;
//! - a T-bone: `r / tan(a/2)` on along one of the walls, into the other, the
//!   notch the tool's half circle in that one.
//!
//! The tool's side reaches the corner exactly; with stock to leave, it leaves
//! that there too. A corner relieved by less than the tolerance is square
//! enough as it is, and is left; so are a curve's chords, which turn little.
//!
//! # In the path
//!
//! Each time the strategy's path cuts to a corner, at whatever level, the tool
//! goes out to the relief's end and back at that height, then on. Every level
//! of a cut in levels is relieved, each pass round a ring ramped down, and
//! every copy of a pattern.

use camcraft_geom::ring::{Ring, Span};
use glam::{DVec2, DVec3};

use crate::job::{Relief, ReliefStyle};
use crate::path::toolpath::{Move, MoveKind, MoveRole, Toolpath, ToolpathError};
use crate::strategy::engrave::{Piece, Trace};

/// The least a path turns at a corner to be relieved, in degrees: less is a
/// curve's chords.
const LEAST_TURN: f64 = 10.0;

/// The sharpest a corner may be and be relieved, in degrees between its
/// walls: sharper, the path turns back on itself, and its relief would run
/// off a long way.
const SHARPEST: f64 = 5.0;

/// How near a move's end must be to a corner, seen from above, to be at it,
/// in millimetres. The path's corners are the corners' own points.
const AT: f64 = 1.0e-4;

/// Which side of a path, as it runs, the material kept is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// On its left.
    Left,
    /// On its right.
    Right,
    /// Either side: a slot, or a cut along the line.
    Both,
}

/// One piece of a path seen from above: where it starts and ends, the way it
/// runs at each end, and how long it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edge {
    /// Where it starts.
    pub from: DVec2,
    /// Where it ends.
    pub to: DVec2,
    /// The way it sets off, a unit vector.
    pub start: DVec2,
    /// The way it arrives, a unit vector.
    pub end: DVec2,
    /// How long it is.
    pub length: f64,
}

impl Edge {
    /// A ring's span from `from`.
    pub fn of_span(from: DVec2, span: &Span) -> Self {
        Self {
            from,
            to: span.to(),
            start: span.tangent_at(from, 0.0),
            end: span.tangent_at(from, 1.0),
            length: span.length(from),
        }
    }

    /// A curve's piece from `from`, seen from above.
    pub fn of_piece(from: DVec2, piece: &Piece) -> Self {
        let to = piece.to().truncate();
        match *piece {
            Piece::Line { .. } => {
                let along = (to - from).normalize_or_zero();
                Self {
                    from,
                    to,
                    start: along,
                    end: along,
                    length: from.distance(to),
                }
            }
            Piece::Arc { centre, ccw, .. } => {
                let (r0, r1) = (from - centre, to - centre);
                // How far round it turns, its way: a whole turn where its ends
                // meet.
                let mut sweep = r0.perp_dot(r1).atan2(r0.dot(r1));
                if !ccw {
                    sweep = -sweep;
                }
                if sweep <= 1e-12 {
                    sweep += std::f64::consts::TAU;
                }
                let tangent = |r: DVec2| {
                    let t = r.perp().normalize_or_zero();
                    if ccw { t } else { -t }
                };
                Self {
                    from,
                    to,
                    start: tangent(r0),
                    end: tangent(r1),
                    length: r0.length() * sweep,
                }
            }
        }
    }
}

/// A ring's edges, in its order; the last ends where the first starts.
pub fn ring_edges(ring: &Ring) -> Vec<Edge> {
    ring.pieces().map(|(f, s)| Edge::of_span(f, s)).collect()
}

/// A curve's edges seen from above, in its order, and whether it closes.
pub fn trace_edges(trace: &Trace, tolerance: f64) -> (Vec<Edge>, bool) {
    let mut at = trace.start.truncate();
    let edges = trace
        .pieces
        .iter()
        .map(|p| {
            let e = Edge::of_piece(at, p);
            at = e.to;
            e
        })
        .filter(|e| e.length > 1e-9)
        .collect();
    (edges, trace.closed(tolerance))
}

/// One corner relieved: where the tool's centre turns it, and where the
/// relief takes the centre, out and back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corner {
    /// The centre's corner.
    pub at: DVec2,
    /// The far end of the relief.
    pub reach: DVec2,
}

/// The corners to relieve of a path of `edges`, each starting where the one
/// before ends, the last joined to the first when `closed`; the material
/// `kept` beside it, the centre `radius` from the walls, as `relief` asks.
/// A corner relieved by no more than `tolerance` is left.
pub fn corners(
    edges: &[Edge],
    closed: bool,
    kept: Kept,
    radius: f64,
    relief: &Relief,
    tolerance: f64,
) -> Vec<Corner> {
    let pairs = edges.windows(2).filter_map(|w| match w {
        [a, b] => Some((a, b)),
        _ => None,
    });
    let around = closed
        .then(|| edges.last().zip(edges.first()))
        .flatten()
        .filter(|(a, b)| a.to.distance(b.from) <= tolerance.max(AT));
    pairs
        .chain(around)
        .filter_map(|(a, b)| corner(a, b, kept, radius, relief, tolerance))
        .collect()
}

/// The relief where `a` ends and `b` starts, if that is a corner to relieve.
fn corner(
    a: &Edge,
    b: &Edge,
    kept: Kept,
    radius: f64,
    relief: &Relief,
    tolerance: f64,
) -> Option<Corner> {
    let (into, out) = (a.end, b.start);
    let turn = into.dot(out).clamp(-1.0, 1.0).acos();
    // Between the walls: a straight run is 180°.
    let between = std::f64::consts::PI - turn;
    if turn < LEAST_TURN.to_radians() || between < SHARPEST.to_radians() {
        return None;
    }
    // Turning left, the outside of the corner is on the right.
    let outside_right = into.perp_dot(out) > 0.0;
    let wanted = match kept {
        Kept::Both => true,
        Kept::Right => outside_right,
        Kept::Left => !outside_right,
    };
    if !wanted {
        return None;
    }
    if relief
        .sharper_than
        .is_some_and(|limit| between.to_degrees() >= limit)
    {
        return None;
    }
    let half = between / 2.0;
    let at = a.to;
    let (way, length) = match relief.style {
        ReliefStyle::Dogbone => (
            (into - out).normalize_or_zero(),
            radius / half.sin() - radius,
        ),
        style => {
            // On along the wall `a` runs beside, or back along `b`'s.
            let (on, back) = ((into, a.length), (-out, b.length));
            let first = match style {
                ReliefStyle::TBoneX => on.0.x.abs() >= back.0.x.abs(),
                ReliefStyle::TBoneY => on.0.y.abs() >= back.0.y.abs(),
                ReliefStyle::TBoneLong => on.1 >= back.1,
                _ => on.1 <= back.1,
            };
            let way = if first { on.0 } else { back.0 };
            (way, radius / half.tan())
        }
    };
    (length > tolerance && length.is_finite() && way != DVec2::ZERO).then(|| Corner {
        at,
        reach: at + way * length,
    })
}

/// `path` with each of `corners` relieved every time a cutting move, or a
/// ramp, ends at it: out to the relief's end and back, level, cutting at the
/// move's feed. And how many reliefs that cut.
pub fn relieve(path: &Toolpath, corners: &[Corner]) -> Result<(Toolpath, u64), ToolpathError> {
    // By X, to find the corners near a point quickly.
    let mut sorted: Vec<Corner> = corners.to_vec();
    sorted.sort_by(|a, b| a.at.x.total_cmp(&b.at.x));
    let near = |p: DVec2| {
        let first = sorted.partition_point(|c| c.at.x < p.x - AT);
        sorted
            .get(first..)
            .unwrap_or(&[])
            .iter()
            .take_while(|c| c.at.x <= p.x + AT)
            .find(|c| c.at.distance(p) <= AT)
            .copied()
    };
    let mut out = Toolpath::new();
    let mut count = 0;
    for m in path.moves() {
        out.push(*m)?;
        if !matches!(m.role, MoveRole::Cut | MoveRole::Ramp) {
            continue;
        }
        let Some(c) = near(m.to.truncate()) else {
            continue;
        };
        let level = |p: DVec2| DVec3::new(p.x, p.y, m.to.z);
        for to in [level(c.reach), m.to] {
            out.push(Move {
                to,
                kind: MoveKind::Linear,
                role: MoveRole::Cut,
                feed: m.feed,
            })?;
        }
        count += 1;
    }
    Ok((out, count))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp, clippy::indexing_slicing)]
mod tests;
