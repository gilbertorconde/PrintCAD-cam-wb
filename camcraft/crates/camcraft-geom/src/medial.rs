//! The middle of a region: for each point of its edge, the largest circle
//! inside it that touches the edge there.
//!
//! As a point goes round a ring, the centre of that circle runs along the
//! region's medial axis, the line of points as far from one side as from the
//! other, and its radius is how far that is: half the width there. A V-bit
//! carving the region follows the centres, as deep as each radius reaches
//! (`camcraft_core::strategy::vcarve`). Going round every ring visits every
//! branch of the axis, once from each side.
//!
//! Each circle is found exactly against the ring's straight edges, from the
//! point and the way in square to its edge: the circle of that centre line
//! that first meets another edge or corner. At a corner that turns in, the
//! circles touching the corner fan between the two edges' ways in, and the
//! centres sweep round it; at a sharp one that turns out, the circle shrinks
//! to the corner itself. Between, points are taken no further apart than the spacing
//! asked, and closer where the centres jump.

use glam::DVec2;

use crate::contour::Region;

/// Two points closer than this, in millimetres, are one.
const SAME: f64 = 1.0e-9;

/// The widest a fan of circles round an inward corner turns between one and
/// the next, in radians.
const FAN_STEP: f64 = 0.15;

/// The sharpest turn, in radians, a ring makes at a corner and still be
/// taken as a curve drawn in short straight pieces.
const SMOOTH: f64 = 0.7;

/// How far either side of a corner that turns out its circles touch, in
/// millimetres, at most.
const NUDGE: f64 = 1.0e-4;

/// How many times a piece of edge is halved where its centres jump.
const MOST_HALVINGS: u32 = 8;

/// The largest circle inside a region touching its edge at a point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Touch {
    /// Where it touches the edge.
    pub at: DVec2,
    /// Its centre.
    pub centre: DVec2,
    /// Its radius, no more than the most asked for.
    pub radius: f64,
    /// Whether the largest circle there was larger than the most asked for,
    /// and this one, that size, is moved to touch the edge at `at`.
    pub capped: bool,
}

/// The edges of every ring of a region, each from a point to the next, and
/// how far a circle may reach past them.
struct Edges {
    edges: Vec<(DVec2, DVec2)>,
    slack: f64,
}

impl Edges {
    /// The radius of the largest circle centred along `n` from `p`, touching
    /// the edge at `p`, that no edge enters by more than the slack; no more
    /// than `most`, and `None` where it would be larger.
    fn radius(&self, p: DVec2, n: DVec2, most: f64) -> Option<f64> {
        let slack = self.slack;
        let reach = 2.0 * most;
        let mut best = f64::INFINITY;
        for &(a, b) in &self.edges {
            let (lo, hi) = (a.min(b), a.max(b));
            // A circle no larger than `most` along `n` stays within twice it of `p`.
            if p.x < lo.x - reach || p.x > hi.x + reach || p.y < lo.y - reach || p.y > hi.y + reach
            {
                continue;
            }
            // Over a corner: the circle `q` is the slack inside.
            for q in [a, b] {
                let w = q - p;
                let along = w.dot(n) - slack;
                if along > SAME {
                    best = best.min((w.length_squared() - slack * slack) / (2.0 * along));
                }
            }
            // Touching the edge between its ends.
            let d = b - a;
            let Some(m) = d.perp().try_normalize() else {
                continue;
            };
            let mut off = (p - a).dot(m);
            let m = if off < 0.0 {
                off = -off;
                -m
            } else {
                m
            };
            let turn = n.dot(m);
            if off <= SAME || turn >= 1.0 - SAME {
                continue;
            }
            let r = (off + slack) / (1.0 - turn);
            let centre = p + n * r;
            let t = (centre - a).dot(d) / d.length_squared();
            if (0.0..=1.0).contains(&t) {
                best = best.min(r);
            }
        }
        (best <= most).then_some(best)
    }

    /// The circle touching at `p` with its centre along `n`, as [`Touch`].
    fn touch(&self, p: DVec2, n: DVec2, most: f64) -> Touch {
        match self.radius(p, n, most) {
            Some(r) => Touch {
                at: p,
                centre: p + n * r,
                radius: r,
                capped: false,
            },
            None => Touch {
                at: p,
                centre: p + n * most,
                radius: most,
                capped: true,
            },
        }
    }
}

/// For each ring of `region`, going round it, the largest circle inside the
/// region touching it at each point no more than `spacing` apart, and at each
/// corner: no larger than `most` across its radius.
///
/// A circle may reach `slack` past the edges, which are only within a
/// tolerance of what they were drawn from: so the slight turns a curve is
/// drawn in do not stop a circle as a corner would, and a sharp corner's
/// circle is the slack, not nothing. The region's outers wind
/// counter-clockwise and its holes clockwise, so inside is on each ring's
/// left.
pub fn inscribed(region: &Region, spacing: f64, most: f64, slack: f64) -> Vec<Vec<Touch>> {
    let edges = Edges {
        slack: slack.max(0.0),
        edges: region
            .rings()
            .iter()
            .flat_map(|r| r.edges())
            .filter(|(a, b)| a.distance(*b) > SAME)
            .collect(),
    };
    let spacing = spacing.max(1.0e-4);
    region
        .rings()
        .iter()
        .map(|ring| {
            // Its edges, less any of no length.
            let sides: Vec<(DVec2, DVec2)> = ring
                .edges()
                .filter(|(a, b)| a.distance(*b) > SAME)
                .collect();
            let count = sides.len();
            let side_at = |i: usize| sides.get(i % count).copied();
            // The way in at each corner, where it is a slight turn of a curve:
            // between its two edges', as the curve's would be.
            let smooth: Vec<Option<DVec2>> = (0..count)
                .map(|i| {
                    let (before, after) = (side_at(i + count - 1)?, side_at(i)?);
                    let (n0, n1) = (inward(before.0, before.1), inward(after.0, after.1));
                    (n0.angle_to(n1).abs() < SMOOTH).then(|| (n0 + n1).normalize_or_zero())
                })
                .collect();
            let mut out = Vec::new();
            for (i, &(a, b)) in sides.iter().enumerate() {
                let before = side_at(i + count - 1).unwrap_or((a, b));
                let n = inward(a, b);
                let (start, end) = (
                    smooth.get(i).copied().flatten(),
                    smooth.get((i + 1) % count).copied().flatten(),
                );
                match start {
                    Some(m) => out.push(edges.touch(a, m, most)),
                    None => corner(&edges, before, (a, b), most, &mut out),
                }
                side(
                    &edges,
                    (a, b),
                    (start.unwrap_or(n), end.unwrap_or(n)),
                    (spacing, most),
                    &mut out,
                );
            }
            out
        })
        .collect()
}

/// The way in from edge `a`→`b`: its left.
fn inward(a: DVec2, b: DVec2) -> DVec2 {
    (b - a).perp().normalize_or_zero()
}

/// The circles touching the corner where `before` meets `after`, at its
/// start: a sharp one. At one that turns out, the circles just either side
/// of it, as good as of no size. At one that turns in, a fan of them
/// touching the corner itself.
fn corner(
    edges: &Edges,
    before: (DVec2, DVec2),
    after: (DVec2, DVec2),
    most: f64,
    out: &mut Vec<Touch>,
) {
    let p = after.0;
    let (n0, n1) = (inward(before.0, before.1), inward(after.0, after.1));
    let turn = (before.1 - before.0).perp_dot(after.1 - after.0);
    let angle = n0.angle_to(n1);
    if turn > 0.0 {
        // Turning left, towards the inside: the corner points out.
        let (u0, u1) = (
            (before.1 - before.0).normalize_or_zero(),
            (after.1 - after.0).normalize_or_zero(),
        );
        let nudge = |a: DVec2, b: DVec2| (a.distance(b) / 100.0).min(NUDGE);
        out.push(edges.touch(p - u0 * nudge(before.0, before.1), n0, most));
        out.push(edges.touch(p + u1 * nudge(after.0, after.1), n1, most));
        return;
    }
    // Turning in: every way between the two edges' is in.
    let steps = (angle.abs() / FAN_STEP).ceil().max(1.0);
    // At most a half turn in steps no wider than the fan's.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let steps = steps as u32;
    for k in 0..steps {
        let t = angle * f64::from(k) / f64::from(steps);
        out.push(edges.touch(p, DVec2::from_angle(t).rotate(n0), most));
    }
}

/// The circles touching edge `a`→`b` between its ends, no more than `spacing`
/// apart, and closer where their centres are further apart than that; their
/// centres along the way in, which turns from `n0` at `a` to `n1` at `b`.
fn side(
    edges: &Edges,
    (a, b): (DVec2, DVec2),
    (n0, n1): (DVec2, DVec2),
    (spacing, most): (f64, f64),
    out: &mut Vec<Touch>,
) {
    let count = (a.distance(b) / spacing).ceil().max(1.0);
    // At least one, and no more than the edge's length over a tenth of a micron.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = count.min(1.0e7) as u32;
    let at = |t: f64| {
        let n = n0.lerp(n1, t).try_normalize().unwrap_or(n0);
        edges.touch(a.lerp(b, t), n, most)
    };
    let mut last = out.last().copied().unwrap_or_else(|| at(0.0));
    let mut t0 = 0.0;
    for k in 1..count {
        let t1 = f64::from(k) / f64::from(count);
        let next = at(t1);
        between(&at, (t0, last), (t1, next), spacing, MOST_HALVINGS, out);
        out.push(next);
        last = next;
        t0 = t1;
    }
    // Up to the corner at its end, which is the next edge's to touch.
    between(&at, (t0, last), (1.0, at(1.0)), spacing, MOST_HALVINGS, out);
}

/// The circles between two along an edge, at `t0` and `t1`, where their
/// centres are further apart than `spacing`: halving, at most `depth` times.
fn between(
    at: &dyn Fn(f64) -> Touch,
    (t0, first): (f64, Touch),
    (t1, last): (f64, Touch),
    spacing: f64,
    depth: u32,
    out: &mut Vec<Touch>,
) {
    if depth == 0 || first.centre.distance(last.centre) <= 2.0 * spacing {
        return;
    }
    let t = (t0 + t1) / 2.0;
    let mid = at(t);
    between(at, (t0, first), (t, mid), spacing, depth - 1, out);
    out.push(mid);
    between(at, (t, mid), (t1, last), spacing, depth - 1, out);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp, clippy::indexing_slicing)]
mod tests;
