//! A setup on a rotary, laid out unrolled: its work zero moved onto the
//! bar's axis, its part unrolled about it, and its stock the bar unrolled,
//! a box once round ([`Unrolled`]).

use std::f64::consts::{PI, TAU};

use camcraft_geom::mesh::{Aabb, TriMesh};
use camcraft_sim::stock::{Axis, Block};
use glam::DVec3;

use super::PrepareError;
use super::frame::{Frame, Unrolled};
use crate::job::Wrap;

/// The deepest a triangle is split to unroll it: a turn in 4^6 pieces.
const SPLITS: u32 = 6;

/// How setup `wrap` is unrolled, its part, in model coordinates, unrolled,
/// and its stock: `frame`'s work zero moved onto the axis of the one bar of
/// `turned` (its blocks, turned, before the work zero), across it.
pub(super) fn unrolled(
    wrap: &Wrap,
    frame: &mut Frame,
    turned: &[Block],
    part: &TriMesh,
    tolerance: f64,
) -> Result<(Unrolled, TriMesh, Block), PrepareError> {
    let bar = match turned {
        [Block::Bar(bar)] if bar.axis != Axis::Z => *bar,
        _ => {
            return Err(PrepareError::Wrap(
                "its stock must be one cylinder lying along the setup's X or Y, across the tool"
                    .into(),
            ));
        }
    };
    // Across the axis, the work zero is on it.
    let o = &mut frame.origin;
    if bar.axis == Axis::X {
        (o.y, o.z) = (bar.centre.y, bar.centre.z);
    } else {
        (o.x, o.z) = (bar.centre.x, bar.centre.z);
    }
    let radius = wrap.diameter.map_or(bar.radius, |d| d * 0.5);
    let u = Unrolled {
        along: bar.axis,
        radius,
    };
    let to_setup = |p: DVec3| frame.to_setup(p);
    // A flat triangle's distance out from the axis curves between its
    // corners as it goes round: pieces so short round that the straight
    // line between theirs strays no more than half the tolerance from it.
    let step = (4.0 * tolerance.max(0.005) / bar.radius.max(1e-3))
        .sqrt()
        .min(10f64.to_radians());
    let mesh = unroll_mesh(part, &u, step, to_setup)?;
    let centre = bar.centre - frame.origin;
    let (lo, hi) = (
        centre - bar_axis(bar.axis) * (bar.length * 0.5),
        centre + bar_axis(bar.axis) * (bar.length * 0.5),
    );
    let half = PI * radius;
    let stock = Block::Box(if bar.axis == Axis::X {
        Aabb {
            min: DVec3::new(lo.x, -half, 0.0),
            max: DVec3::new(hi.x, half, bar.radius),
        }
    } else {
        Aabb {
            min: DVec3::new(-half, lo.y, 0.0),
            max: DVec3::new(half, hi.y, bar.radius),
        }
    });
    Ok((u, mesh, stock))
}

fn bar_axis(a: Axis) -> DVec3 {
    match a {
        Axis::X => DVec3::X,
        Axis::Y => DVec3::Y,
        Axis::Z => DVec3::Z,
    }
}

/// `mesh`, each point taken into the setup by `to_setup`, unrolled by `u`:
/// each triangle split until no piece goes further round than `step`
/// radians, so a straight edge between its unrolled corners is near the
/// curve its edge unrolls to. A piece across the turn's seam, at the
/// bottom, is put in on both sides of it, so the turn's first cells and its
/// last both have it.
pub(super) fn unroll_mesh(
    mesh: &TriMesh,
    u: &Unrolled,
    step: f64,
    to_setup: impl Fn(DVec3) -> DVec3,
) -> Result<TriMesh, PrepareError> {
    let mut vertices = Vec::new();
    let mut triangles = Vec::new();
    let mut out = |t: [DVec3; 3]| {
        let lo = t.iter().map(|p| u.angle(*p)).fold(f64::INFINITY, f64::min);
        let hi = t
            .iter()
            .map(|p| u.angle(*p))
            .fold(f64::NEG_INFINITY, f64::max);
        let shifts = [0.0, -TAU, TAU];
        for shift in shifts {
            if (shift < 0.0 && hi <= PI) || (shift > 0.0 && lo >= -PI) {
                continue;
            }
            let base = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
            for p in t {
                vertices.push(round_by(u, p, shift));
            }
            triangles.push([base, base + 1, base + 2]);
        }
    };
    for i in 0..mesh.len() {
        let Some(t) = mesh.triangle(i) else { continue };
        split(u, t.map(&to_setup), step, SPLITS, &mut out);
    }
    TriMesh::new(vertices, triangles).map_err(PrepareError::Mesh)
}

/// `q`, unrolled, moved round by `shift` radians.
fn round_by(u: &Unrolled, q: DVec3, shift: f64) -> DVec3 {
    let d = shift * u.radius;
    if u.along == Axis::Y {
        q + DVec3::X * d
    } else {
        q + DVec3::Y * d
    }
}

/// Triangle `t`, in the setup, unrolled into `out`, each corner round the
/// same way as the one furthest out so it does not go the long way round,
/// split in four while it goes further round than `step`.
fn split(u: &Unrolled, t: [DVec3; 3], step: f64, depth: u32, out: &mut impl FnMut([DVec3; 3])) {
    let q = t.map(|p| u.unroll(p));
    let far = q
        .iter()
        .copied()
        .max_by(|a, b| a.z.total_cmp(&b.z))
        .unwrap_or(DVec3::ZERO);
    // On the axis nothing is round: a corner there is round as the rest.
    let near = far.z * 1e-6;
    if far.z <= 1e-9 {
        return;
    }
    let reference = u.angle(far);
    let mut angles = [0.0; 3];
    for (a, p) in angles.iter_mut().zip(&q) {
        let mut x = u.angle(*p);
        x -= TAU * ((x - reference) / TAU).round();
        *a = x;
    }
    let on_axis: Vec<bool> = q.iter().map(|p| p.z <= near).collect();
    let (sum, count) = angles
        .iter()
        .zip(&on_axis)
        .filter(|(_, o)| !**o)
        .fold((0.0, 0.0), |(s, n), (a, _)| (s + a, n + 1.0));
    let mean = sum / f64::max(count, 1.0);
    for (a, o) in angles.iter_mut().zip(&on_axis) {
        if *o {
            *a = mean;
        }
    }
    let lo = angles.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = angles.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if hi - lo > step && depth > 0 {
        let [a, b, c] = t;
        let (ab, bc, ca) = ((a + b) * 0.5, (b + c) * 0.5, (c + a) * 0.5);
        for piece in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
            split(u, piece, step, depth - 1, out);
        }
        return;
    }
    let mut pieces = [DVec3::ZERO; 3];
    for ((slot, p), a) in pieces.iter_mut().zip(&q).zip(angles) {
        *slot = round_by(u, *p, a - u.angle(*p));
    }
    out(pieces);
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;

    /// A cylinder of radius 10 along X, 16 sides round, 20 long.
    fn tube() -> TriMesh {
        let n = 16;
        let mut v = Vec::new();
        for x in [0.0, 20.0] {
            for i in 0..n {
                let a = TAU * f64::from(i) / f64::from(n);
                v.push(DVec3::new(x, 10.0 * a.sin(), 10.0 * a.cos()));
            }
        }
        let mut t = Vec::new();
        for i in 0..n {
            let j = (i + 1) % n;
            t.push([i, j, n + i]);
            t.push([j, n + j, n + i]);
        }
        TriMesh::new(v, t).unwrap()
    }

    /// Unrolled, a cylinder about the axis is level at its radius, to its
    /// facets' sag, all the way round and across the seam, and no piece goes
    /// the long way round.
    #[test]
    fn a_cylinder_unrolls_level() {
        let u = Unrolled {
            along: Axis::X,
            radius: 10.0,
        };
        let m = unroll_mesh(&tube(), &u, 0.2, |p| p).unwrap();
        let b = m.bounds().unwrap();
        // Its corners at the radius; the points split in between on its
        // flat sides, no further in than their sag.
        let sag = 10.0 * (1.0 - (PI / 16.0).cos());
        assert!((b.max.z - 10.0).abs() < 1e-9, "{b:?}");
        assert!(b.min.z > 10.0 - sag - 1e-9, "{b:?}");
        // Each piece at most 0.2 radians round.
        for i in 0..m.len() {
            let tb = m.triangle_bounds(i).unwrap();
            assert!(tb.max.y - tb.min.y <= 0.2 * 10.0 + 1e-9, "{tb:?}");
        }
        // Past the seam on both sides.
        assert!(b.min.y < -PI * 10.0 + 1e-6 && b.max.y > PI * 10.0 - 1e-6);
    }
}
