//! Engraving: the tool's centre run along curves, open or closed, at a depth
//! below the top. Fusion's Trace and Engrave.
//!
//! A curve is lines and level arcs ([`Trace`]), seen from above; an arc is cut
//! as one, so the post writes it as `G2` or `G3`, and as a helix where the
//! tool goes down along it. Each curve ramps down along itself from the top,
//! no steeper than the ramp angle and no more than a stepdown each time along
//! it: round and round a closed curve, back and forth along an open one. Then
//! once more along it at the depth, to cut the ramp's slope away. It never
//! lifts between its levels, and never plunges into what it cuts.
//!
//! The curves are cut in the order nearest the end of the last, each the way
//! its direction says. Where the tool goes from one to the next is the
//! linker's.

use glam::{DVec2, DVec3};

use crate::job::TraceDirection;
use crate::path::toolpath::{Arc, MoveRole, Toolpath};
use crate::strategy::{StrategyError, check};

/// How far apart, in millimetres, two points may be and still be one.
const SAME_POINT: f64 = 1.0e-6;

/// The most times along one curve before the bottom: past it, a curve too
/// short to ramp down at the angle asked goes straight down instead.
const MOST_RAMPS: u32 = 1000;

/// One piece of a [`Trace`], from where the piece before it ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Piece {
    /// A straight line to `to`.
    Line {
        /// Where it ends.
        to: DVec3,
    },
    /// A level arc about `centre` to `to`, counter-clockwise seen from above
    /// when `ccw`. One whose ends meet goes all the way round.
    Arc {
        /// Its centre, seen from above.
        centre: DVec2,
        /// Which way it turns.
        ccw: bool,
        /// Where it ends.
        to: DVec3,
    },
}

impl Piece {
    /// Where it ends.
    pub fn to(&self) -> DVec3 {
        match *self {
            Piece::Line { to } | Piece::Arc { to, .. } => to,
        }
    }

    /// The same piece with its end moved to `to`.
    fn ending_at(self, to: DVec3) -> Self {
        match self {
            Piece::Line { .. } => Piece::Line { to },
            Piece::Arc { centre, ccw, .. } => Piece::Arc { centre, ccw, to },
        }
    }

    /// The arc it is, from `from`, its centre moved square to its chord so both
    /// its ends are on it exactly; `None` for a line, or an arc too small to be
    /// one.
    fn arc(&self, from: DVec3, tolerance: f64) -> Option<Arc> {
        let Piece::Arc { centre, ccw, to } = *self else {
            return None;
        };
        let (a, b) = (from.truncate(), to.truncate());
        let centre = if a.distance(b) > SAME_POINT {
            let mid = (a + b) / 2.0;
            let across = (b - a).perp().normalize();
            mid + across * (centre - mid).dot(across)
        } else {
            centre
        };
        Arc::about(centre.extend(from.z), DVec3::Z, from, to, ccw, tolerance)
    }

    /// How far it runs, seen from above, from `from`.
    fn length(&self, from: DVec3, tolerance: f64) -> f64 {
        match self.arc(from, tolerance) {
            Some(a) => a.radius(from) * a.sweep.abs(),
            None => from.truncate().distance(self.to().truncate()),
        }
    }
}

/// A curve the tool follows, seen from above, in the setup's frame: lines and
/// level arcs end to end from `start`. Its heights are where it was drawn,
/// for a height measured from it; the engraving sets its own.
#[derive(Debug, Clone, PartialEq)]
pub struct Trace {
    /// Where it starts.
    pub start: DVec3,
    /// Its pieces, in order.
    pub pieces: Vec<Piece>,
}

impl Trace {
    /// Where it ends.
    pub fn end(&self) -> DVec3 {
        self.pieces.last().map_or(self.start, Piece::to)
    }

    /// Whether it ends where it starts, seen from above, within `tolerance`.
    pub fn closed(&self, tolerance: f64) -> bool {
        !self.pieces.is_empty()
            && self.start.truncate().distance(self.end().truncate()) <= tolerance
    }

    /// Its points, its arcs in chords within `tolerance`: the start, then
    /// each piece's.
    pub fn points(&self, tolerance: f64) -> Vec<DVec3> {
        let mut out = vec![self.start];
        let mut at = self.start;
        for p in &self.pieces {
            match p.arc(at, tolerance) {
                Some(a) => out.extend(a.chords(at, p.to(), tolerance)),
                None => out.push(p.to()),
            }
            at = p.to();
        }
        out
    }

    /// How far it runs, seen from above.
    pub fn length(&self, tolerance: f64) -> f64 {
        let mut at = self.start;
        let mut total = 0.0;
        for p in &self.pieces {
            total += p.length(at, tolerance);
            at = p.to();
        }
        total
    }

    /// The same curve run the other way.
    pub fn reversed(&self) -> Self {
        let mut pieces = Vec::with_capacity(self.pieces.len());
        let mut to = self.start;
        for p in &self.pieces {
            let piece = match *p {
                Piece::Line { .. } => Piece::Line { to },
                Piece::Arc { centre, ccw, .. } => Piece::Arc {
                    centre,
                    ccw: !ccw,
                    to,
                },
            };
            pieces.push(piece);
            to = p.to();
        }
        pieces.reverse();
        Self {
            start: self.end(),
            pieces,
        }
    }

    /// The area it encloses seen from above, positive when it runs
    /// counter-clockwise, its arcs in chords within `tolerance`.
    pub fn signed_area(&self, tolerance: f64) -> f64 {
        let points = self.points(tolerance);
        points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .map(|(a, b)| a.truncate().perp_dot(b.truncate()))
            .sum::<f64>()
            / 2.0
    }
}

/// How an engraving is cut.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EngraveParams {
    /// The height it cuts down from: the surface the curves are cut into.
    pub top: f64,
    /// How far below the top it cuts.
    pub depth: f64,
    /// The most it goes down each time along a curve; all the way when not
    /// given.
    pub stepdown: Option<f64>,
    /// The steepest it ramps down along a curve, in radians from level.
    pub ramp_angle: f64,
    /// Cutting feed, in mm/min.
    pub feed: f64,
    /// Feed going down along a curve, in mm/min.
    pub ramp_feed: f64,
    /// Feed straight down, for a curve too short to ramp along, in mm/min.
    pub plunge_feed: f64,
    /// Which way it runs along each curve.
    pub direction: TraceDirection,
    /// Each curve the other way from what `direction` says.
    pub reverse: bool,
    /// Whether climbing has what it cuts on the tool's right: a clockwise
    /// spindle.
    pub climb_right: bool,
    /// Chordal tolerance, in millimetres.
    pub tolerance: f64,
}

/// What an engraving did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngraveReport {
    /// Curves cut.
    pub curves: u64,
    /// Of them, the closed ones.
    pub closed: u64,
    /// Arcs among their pieces, cut as arcs.
    pub arcs: u64,
    /// Times along a curve, ramps and the last at the depth, in all.
    pub passes: u64,
    /// Curves too short to ramp down at the angle asked, which go straight
    /// down to the depth at their start.
    pub plunged: u64,
}

/// `trace` run the way `p` asks: closed, round the way its direction says;
/// open, as drawn; then reversed if asked.
fn oriented(trace: &Trace, p: &EngraveParams) -> Trace {
    let closed = trace.closed(p.tolerance);
    let flip = closed
        && match p.direction {
            TraceDirection::AsDrawn => false,
            // Climbing round what it encloses has it on the right: clockwise
            // with a clockwise spindle.
            TraceDirection::Climb | TraceDirection::Conventional => {
                let clockwise = (p.direction == TraceDirection::Climb) == p.climb_right;
                let runs_clockwise = trace.signed_area(p.tolerance) < 0.0;
                clockwise != runs_clockwise
            }
        };
    if flip != p.reverse {
        trace.reversed()
    } else {
        trace.clone()
    }
}

/// The curves in the order they are cut: each the one whose start is nearest
/// where the last ended, from the first.
fn in_order(traces: Vec<Trace>) -> Vec<Trace> {
    let mut left = traces;
    let mut out = Vec::with_capacity(left.len());
    let mut at: Option<DVec2> = None;
    while !left.is_empty() {
        let next = match at {
            None => 0,
            Some(at) => left
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    at.distance(a.start.truncate())
                        .total_cmp(&at.distance(b.start.truncate()))
                })
                .map_or(0, |(i, _)| i),
        };
        let t = left.remove(next);
        at = Some(t.end().truncate());
        out.push(t);
    }
    out
}

/// Run along `trace` once, from `from` down to `to` in height evenly along
/// its length, `length` seen from above, as `role` at `feed`.
fn along(
    path: &mut Toolpath,
    trace: &Trace,
    (from, to): (f64, f64),
    length: f64,
    (role, feed): (MoveRole, f64),
    tolerance: f64,
) -> Result<u64, StrategyError> {
    let mut arcs = 0;
    let mut at = trace.start.truncate().extend(from);
    let mut run = 0.0;
    for piece in &trace.pieces {
        run += piece.length(at, tolerance);
        let z = if length > 0.0 {
            from + (to - from) * (run / length).min(1.0)
        } else {
            to
        };
        let end = piece.to().truncate().extend(z);
        match piece.ending_at(end).arc(at, tolerance) {
            Some(arc) => {
                path.arc_to(end, arc, role, feed)?;
                arcs += 1;
            }
            None => path.feed_to(end, role, feed)?,
        }
        at = end;
    }
    Ok(arcs)
}

/// The engraving of `traces`: each oriented as asked, in order, ramped down
/// along itself and then cut once at the depth. The path holds a
/// [`MoveRole::Link`] to each curve's start at the top, then its cuts.
pub fn engrave(
    traces: &[Trace],
    p: &EngraveParams,
) -> Result<(Toolpath, EngraveReport), StrategyError> {
    check("depth", p.depth)?;
    check("feed", p.feed)?;
    check("ramp feed", p.ramp_feed)?;
    check("plunge feed", p.plunge_feed)?;
    check("tolerance", p.tolerance)?;
    if let Some(s) = p.stepdown {
        check("stepdown", s)?;
    }
    if !(p.ramp_angle > 0.0 && p.ramp_angle < std::f64::consts::FRAC_PI_2) {
        return Err(StrategyError::BadParameter {
            name: "ramp angle",
            value: p.ramp_angle,
        });
    }
    let bottom = p.top - p.depth;
    let mut path = Toolpath::new();
    let mut report = EngraveReport::default();
    let oriented: Vec<Trace> = traces
        .iter()
        .filter(|t| !t.pieces.is_empty())
        .map(|t| oriented(t, p))
        .collect();
    for trace in in_order(oriented) {
        let closed = trace.closed(p.tolerance);
        let length = trace.length(p.tolerance);
        let back = trace.reversed();
        let step = p
            .stepdown
            .unwrap_or(p.depth)
            .min(length * p.ramp_angle.tan());
        // How many times along it ramping down takes.
        let ramps = (p.depth / step).ceil();
        let start = trace.start.truncate();
        path.feed_to(start.extend(p.top), MoveRole::Link, p.feed)?;
        report.curves += 1;
        report.closed += u64::from(closed);
        let mut forward = true;
        let next = |forward: &mut bool| {
            // Round a closed curve the same way each time; along an open one
            // back and forth.
            let t = if *forward || closed { &trace } else { &back };
            if !closed {
                *forward = !*forward;
            }
            t
        };
        let mut z = p.top;
        if ramps.is_finite() && ramps <= f64::from(MOST_RAMPS) {
            while z > bottom + crate::strategy::SAME_LEVEL {
                let lower = (z - step).max(bottom);
                let t = next(&mut forward);
                report.arcs += along(
                    &mut path,
                    t,
                    (z, lower),
                    length,
                    (MoveRole::Ramp, p.ramp_feed),
                    p.tolerance,
                )?;
                report.passes += 1;
                z = lower;
            }
        } else {
            path.feed_to(start.extend(bottom), MoveRole::Plunge, p.plunge_feed)?;
            report.plunged += 1;
        }
        let t = next(&mut forward);
        report.arcs += along(
            &mut path,
            t,
            (bottom, bottom),
            length,
            (MoveRole::Cut, p.feed),
            p.tolerance,
        )?;
        report.passes += 1;
    }
    Ok((path, report))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp, clippy::indexing_slicing)]
mod tests;
