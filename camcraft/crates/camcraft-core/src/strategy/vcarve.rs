//! V-carving: a V-bit run down the middle of closed areas, at each point as
//! deep as the V has to go to reach the walls either side, so the width it
//! cuts is the area's: a letter's stroke, wide where it is wide and to a
//! point in its corners.
//!
//! The middle comes from [`camcraft_geom::medial`]: going round each ring of
//! the area, the largest circle inside touching it there. Its centre is where
//! the tool goes, and its radius, less the tip's, over the tangent of half
//! the V's angle is how deep. The tool goes round each ring once, from where
//! it is shallowest (a sharp corner's point, at the top), so it goes down
//! the middle once from each side: each wall is cut by the side of the V it
//! faces.
//!
//! Where an area is wider than the V reaches at the deepest it may go, the
//! tool goes round its walls at that depth instead, their V cut to the full
//! depth; the flat floor between is left, and said, for a flat end mill to
//! clear: an operation cuts with one tool.

use camcraft_geom::contour::Region;
use camcraft_geom::medial::{Touch, inscribed};
use glam::DVec3;

use crate::path::toolpath::{MoveRole, Toolpath};
use crate::strategy::{StrategyError, check};

/// How a V-carving is cut.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VCarveParams {
    /// The surface it carves into.
    pub top: f64,
    /// Half the V's angle, in radians.
    pub half_angle: f64,
    /// The radius of the V's flat tip; zero for a point.
    pub tip_radius: f64,
    /// The radius where the V is as wide as the tool, past which it is not a V.
    pub radius: f64,
    /// The deepest it may go below the top: the V's own full depth when not
    /// given.
    pub max_depth: Option<f64>,
    /// Levels a stepdown apart, each going no deeper than it: all at once
    /// when not given.
    pub stepdown: Option<f64>,
    /// Cutting feed, in mm/min.
    pub feed: f64,
    /// Chordal tolerance, in millimetres.
    pub tolerance: f64,
}

impl VCarveParams {
    /// How deep the V goes to be `r` wide either side of its centre.
    fn depth(&self, r: f64) -> f64 {
        (r - self.tip_radius).max(0.0) / self.half_angle.tan()
    }

    /// The widest either side it cuts: its full V, or at the deepest it may go.
    fn widest(&self) -> f64 {
        let full = self.radius;
        match self.max_depth {
            Some(d) => (self.tip_radius + d * self.half_angle.tan()).min(full),
            None => full,
        }
    }
}

/// What a V-carving did.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VCarveReport {
    /// Rings gone round, at every level.
    pub passes: u64,
    /// The deepest it cuts below the top.
    pub deepest: f64,
    /// How much floor, in mm², it leaves flat at the deepest, wider than the
    /// V reaches.
    pub floor_mm2: f64,
    /// How far either side of the tool's centre it cuts at the deepest: what
    /// a flat end mill clearing that floor leaves on the walls.
    pub widest: f64,
}

/// How far `p` is from the segment `a`→`b`.
fn off_line(p: DVec3, a: DVec3, b: DVec3) -> f64 {
    let along = b - a;
    let t = if along.length_squared() > 0.0 {
        ((p - a).dot(along) / along.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a + along * t).distance(p)
}

/// `points`, ends kept, with as many left out as leaves the line through the
/// rest within `tolerance` of every one: Douglas and Peucker's.
pub(crate) fn simplified(points: &[DVec3], tolerance: f64) -> Vec<DVec3> {
    let n = points.len();
    let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
        return Vec::new();
    };
    let mut keep = vec![false; n];
    let mut spans = vec![(0, first, n - 1, last)];
    while let Some((a, pa, b, pb)) = spans.pop() {
        let far = points
            .iter()
            .enumerate()
            .take(b)
            .skip(a + 1)
            .map(|(i, q)| (i, *q, off_line(*q, pa, pb)))
            .max_by(|x, y| x.2.total_cmp(&y.2));
        if let Some((i, q, d)) = far
            && d > tolerance
        {
            if let Some(k) = keep.get_mut(i) {
                *k = true;
            }
            spans.push((a, pa, i, q));
            spans.push((i, q, b, pb));
        }
    }
    if let Some(k) = keep.first_mut() {
        *k = true;
    }
    if let Some(k) = keep.last_mut() {
        *k = true;
    }
    points
        .iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

/// The tool's path round one ring of touching circles: each centre, as deep
/// as its radius reaches, starting at the shallowest.
fn round(touches: &[Touch], p: &VCarveParams) -> Vec<DVec3> {
    let first = touches
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.radius.total_cmp(&b.1.radius))
        .map_or(0, |(i, _)| i);
    let mut points: Vec<DVec3> = touches
        .iter()
        .cycle()
        .skip(first)
        .take(touches.len())
        .map(|t| t.centre.extend(p.top - p.depth(t.radius)))
        .collect();
    if let Some(&start) = points.first() {
        points.push(start);
    }
    simplified(&points, p.tolerance / 4.0)
}

/// The V-carving of `area`: each ring gone round, level by level when a
/// stepdown is given. The path holds a [`MoveRole::Link`] to the start of
/// each ring, then its cuts.
pub fn vcarve(area: &Region, p: &VCarveParams) -> Result<(Toolpath, VCarveReport), StrategyError> {
    check("V angle", p.half_angle)?;
    check("tool radius", p.radius)?;
    check("feed", p.feed)?;
    check("tolerance", p.tolerance)?;
    if let Some(d) = p.max_depth {
        check("max depth", d)?;
    }
    if let Some(s) = p.stepdown {
        check("stepdown", s)?;
    }
    let widest = p.widest();
    if widest <= p.tip_radius {
        return Err(StrategyError::BadParameter {
            name: "max depth",
            value: p.max_depth.unwrap_or(0.0),
        });
    }
    // Points close enough that the depth between them is within tolerance on
    // the V's slope, and the walls a tolerance's half off either way.
    let spacing = (p.tolerance * p.half_angle.tan()).max(p.tolerance / 4.0);
    let rings: Vec<Vec<DVec3>> = inscribed(area, spacing, widest, p.tolerance / 2.0)
        .iter()
        .filter(|r| !r.is_empty())
        .map(|r| round(r, p))
        .collect();
    let deepest = rings
        .iter()
        .flatten()
        .map(|q| p.top - q.z)
        .fold(0.0, f64::max);
    let mut levels = Vec::new();
    if let Some(step) = p.stepdown {
        let mut z = p.top - step;
        while z > p.top - deepest + crate::strategy::SAME_LEVEL {
            levels.push(z);
            z -= step;
        }
    }
    levels.push(f64::NEG_INFINITY);
    let mut path = Toolpath::new();
    let mut report = VCarveReport {
        deepest,
        widest,
        ..VCarveReport::default()
    };
    for level in levels {
        for ring in &rings {
            let mut moves = ring.iter().map(|q| q.truncate().extend(q.z.max(level)));
            let Some(start) = moves.next() else {
                continue;
            };
            path.feed_to(start, MoveRole::Link, p.feed)?;
            for q in moves {
                path.feed_to(q, MoveRole::Cut, p.feed)?;
            }
            report.passes += 1;
        }
    }
    // The floor is what is further from every wall than the V reaches.
    report.floor_mm2 = area.offset(-widest, p.tolerance / 2.0).area();
    Ok((path, report))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp, clippy::indexing_slicing)]
mod tests;
