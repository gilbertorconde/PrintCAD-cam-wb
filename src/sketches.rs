//! The document's sketches as curves an operation follows: what the package
//! can read of a sketch is its profile, the closed loops it draws (text
//! among them, as the letters' outlines), placed in world space.
//!
//! Lines and arcs stay lines and arcs, so an engraving cuts its arcs as arcs;
//! a circle is two half arcs. Ellipses and splines come as the lines that
//! follow them within the setup's tolerance. A sketch's open curves close
//! nothing, so are not in its profile: the package cannot read them.

use camcraft_core::job::{Curve, Segment};
use printcad_bench_sdk::api::kernel_api::{Profile, ProfilePlane, ProfileSegment};
use printcad_bench_sdk::host;

/// The most a piece of a spline is halved to follow it within the tolerance.
const MOST_HALVINGS: u32 = 12;

/// The sketches of the document that close something, as the panel lists
/// them: id and name.
pub fn sketches() -> Vec<(String, String)> {
    host::features()
        .into_iter()
        .filter(|n| host::profile(&n.id).is_some_and(|p| !p.wires.is_empty()))
        .map(|n| (n.id, n.name))
        .collect()
}

/// Sketch `id`'s curves, splines in lines within `tolerance`; `None` when it
/// is gone or closes nothing.
pub fn of(id: &str, tolerance: f64) -> Option<Vec<Curve>> {
    let profile = host::profile(id)?;
    let curves = curves(&profile, tolerance);
    (!curves.is_empty()).then_some(curves)
}

/// A profile's loops as curves in world space, each wire one.
pub fn curves(profile: &Profile, tolerance: f64) -> Vec<Curve> {
    let plane = &profile.plane;
    profile
        .wires
        .iter()
        .filter_map(|w| wire(&w.segments, plane, tolerance))
        .collect()
}

type P2 = [f64; 2];

fn sub(a: P2, b: P2) -> P2 {
    [a[0] - b[0], a[1] - b[1]]
}

fn dist(a: P2, b: P2) -> f64 {
    let d = sub(a, b);
    d[0].hypot(d[1])
}

/// A point of the plane in world space.
fn place(plane: &ProfilePlane, p: P2) -> [f64; 3] {
    std::array::from_fn(|i| plane.origin[i] + plane.x_axis[i] * p[0] + plane.y_axis[i] * p[1])
}

/// One piece of a wire, in the plane: a line or an arc to its end, or points
/// along a curve.
enum Piece {
    Line(P2, P2),
    Arc(P2, P2, P2),
    Points(Vec<P2>),
}

impl Piece {
    fn start(&self) -> Option<P2> {
        match self {
            Piece::Line(a, _) | Piece::Arc(a, ..) => Some(*a),
            Piece::Points(p) => p.first().copied(),
        }
    }

    fn end(&self) -> Option<P2> {
        match self {
            Piece::Line(_, b) | Piece::Arc(_, _, b) => Some(*b),
            Piece::Points(p) => p.last().copied(),
        }
    }

    fn reversed(self) -> Self {
        match self {
            Piece::Line(a, b) => Piece::Line(b, a),
            Piece::Arc(a, m, b) => Piece::Arc(b, m, a),
            Piece::Points(mut p) => {
                p.reverse();
                Piece::Points(p)
            }
        }
    }
}

/// A segment of a profile as pieces, in the plane.
fn pieces(segment: &ProfileSegment, tolerance: f64) -> Vec<Piece> {
    match segment {
        ProfileSegment::Line { start, end } => vec![Piece::Line(*start, *end)],
        ProfileSegment::Arc { start, mid, end } => vec![Piece::Arc(*start, *mid, *end)],
        ProfileSegment::Circle { center, radius } => {
            let at = |a: f64| [center[0] + radius * a.cos(), center[1] + radius * a.sin()];
            use std::f64::consts::{FRAC_PI_2, PI};
            vec![
                Piece::Arc(at(0.0), at(FRAC_PI_2), at(PI)),
                Piece::Arc(at(PI), at(PI + FRAC_PI_2), at(0.0)),
            ]
        }
        ProfileSegment::Ellipse {
            center,
            major,
            ratio,
        } => ellipse(
            *center,
            *major,
            *ratio,
            (0.0, std::f64::consts::TAU),
            tolerance,
        ),
        ProfileSegment::EllipseArc {
            center,
            major,
            ratio,
            start_param,
            end_param,
        } => {
            let mut to = *end_param;
            while to <= *start_param {
                to += std::f64::consts::TAU;
            }
            ellipse(*center, *major, *ratio, (*start_param, to), tolerance)
        }
        ProfileSegment::BSpline {
            control_points,
            periodic,
        } => {
            let degree = 3.min(control_points.len().saturating_sub(1));
            let spline = Spline::new(degree, &[], control_points, &[], *periodic);
            spline.map(|s| s.flatten(tolerance)).into_iter().collect()
        }
        ProfileSegment::Nurbs {
            degree,
            knots,
            control_points,
            weights,
            periodic,
        } => {
            let spline = Spline::new(*degree as usize, knots, control_points, weights, *periodic);
            spline.map(|s| s.flatten(tolerance)).into_iter().collect()
        }
        // A kind of segment this package does not know follows nothing.
        _ => Vec::new(),
    }
}

/// An ellipse between two of its parameter angles, in lines within
/// `tolerance`.
fn ellipse(c: P2, major: P2, ratio: f64, (from, to): (f64, f64), tolerance: f64) -> Vec<Piece> {
    let minor = [-major[1] * ratio, major[0] * ratio];
    let at = |t: f64| {
        let (s, k) = t.sin_cos();
        [
            c[0] + major[0] * k + minor[0] * s,
            c[1] + major[1] * k + minor[1] * s,
        ]
    };
    vec![Piece::Points(flatten(&at, (from, to), tolerance))]
}

/// Points along `at` from `from` to `to`, no chord straying more than
/// `tolerance` from it: sixteen pieces, each halved until it is close.
fn flatten(at: &dyn Fn(f64) -> P2, (from, to): (f64, f64), tolerance: f64) -> Vec<P2> {
    fn halve(
        at: &dyn Fn(f64) -> P2,
        (t0, p0): (f64, P2),
        (t1, p1): (f64, P2),
        tolerance: f64,
        depth: u32,
        out: &mut Vec<P2>,
    ) {
        let t = (t0 + t1) / 2.0;
        let mid = at(t);
        // How far the middle is from the chord.
        let chord = sub(p1, p0);
        let len = chord[0].hypot(chord[1]);
        let off = if len > 0.0 {
            (sub(mid, p0)[0] * chord[1] - sub(mid, p0)[1] * chord[0]).abs() / len
        } else {
            dist(mid, p0)
        };
        if depth > 0 && off > tolerance {
            halve(at, (t0, p0), (t, mid), tolerance, depth - 1, out);
            halve(at, (t, mid), (t1, p1), tolerance, depth - 1, out);
        } else {
            out.push(p1);
        }
    }
    let n = 16;
    let mut out = vec![at(from)];
    for k in 0..n {
        let t0 = from + (to - from) * f64::from(k) / f64::from(n);
        let t1 = from + (to - from) * f64::from(k + 1) / f64::from(n);
        let p0 = *out.last().unwrap_or(&at(t0));
        halve(
            at,
            (t0, p0),
            (t1, at(t1)),
            tolerance,
            MOST_HALVINGS,
            &mut out,
        );
    }
    out
}

/// A B-spline, rational where it has weights, over its knots.
struct Spline {
    degree: usize,
    knots: Vec<f64>,
    /// Control points in homogeneous form: x·w, y·w, w.
    points: Vec<[f64; 3]>,
}

impl Spline {
    /// The spline a profile describes: its own knots when open and given,
    /// clamped and uniform when open and not, and a uniform ring over its
    /// control points when periodic.
    fn new(
        degree: usize,
        knots: &[f64],
        control: &[P2],
        weights: &[f64],
        periodic: bool,
    ) -> Option<Self> {
        let degree = degree.max(1);
        let w = |i: usize| weights.get(i).copied().unwrap_or(1.0);
        let mut points: Vec<[f64; 3]> = control
            .iter()
            .enumerate()
            .map(|(i, p)| [p[0] * w(i), p[1] * w(i), w(i)])
            .collect();
        if points.len() < 2 {
            return None;
        }
        let knots = if periodic {
            // The first `degree` points again at the end close it smoothly.
            let wrap: Vec<[f64; 3]> = points.iter().take(degree).copied().collect();
            points.extend(wrap);
            (0..points.len() + degree + 1).map(|i| i as f64).collect()
        } else if knots.len() == points.len() + degree + 1 {
            knots.to_vec()
        } else {
            let degree = degree.min(points.len() - 1);
            let inner = points.len() - degree;
            let mut k = vec![0.0; degree + 1];
            k.extend((1..inner).map(|i| i as f64 / inner as f64));
            k.extend(std::iter::repeat_n(1.0, degree + 1));
            return Some(Self {
                degree,
                knots: k,
                points,
            });
        };
        Some(Self {
            degree,
            knots,
            points,
        })
    }

    /// Where its parameter runs, from and to.
    fn domain(&self) -> (f64, f64) {
        let p = self.degree;
        let n = self.points.len();
        (
            self.knots.get(p).copied().unwrap_or(0.0),
            self.knots.get(n).copied().unwrap_or(1.0),
        )
    }

    /// The point at `t`, by de Boor's.
    fn at(&self, t: f64) -> P2 {
        let p = self.degree;
        let n = self.points.len();
        let k = &self.knots;
        // The span `t` is in: knots[s] <= t < knots[s + 1], within the domain.
        let mut s = p;
        while s + 1 < n && k.get(s + 1).is_some_and(|&v| v <= t) {
            s += 1;
        }
        let mut d: Vec<[f64; 3]> = (0..=p)
            .map(|j| {
                self.points
                    .get(s + j - p)
                    .copied()
                    .unwrap_or([0.0, 0.0, 1.0])
            })
            .collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = s + j - p;
                let (lo, hi) = (
                    k.get(i).copied().unwrap_or(0.0),
                    k.get(i + p + 1 - r).copied().unwrap_or(1.0),
                );
                let a = if hi > lo { (t - lo) / (hi - lo) } else { 0.0 };
                let (prev, cur) = (d.get(j - 1).copied(), d.get(j).copied());
                if let (Some(prev), Some(cur), Some(slot)) = (prev, cur, d.get_mut(j)) {
                    *slot = std::array::from_fn(|c| (1.0 - a) * prev[c] + a * cur[c]);
                }
            }
        }
        let [x, y, w] = d.get(p).copied().unwrap_or([0.0, 0.0, 1.0]);
        if w.abs() > 1e-12 {
            [x / w, y / w]
        } else {
            [x, y]
        }
    }

    /// It in lines within `tolerance`.
    fn flatten(&self, tolerance: f64) -> Piece {
        Piece::Points(flatten(&|t| self.at(t), self.domain(), tolerance))
    }
}

/// A wire of a profile as one curve in world space: its pieces end to end,
/// each turned round where it is drawn the other way.
fn wire(segments: &[ProfileSegment], plane: &ProfilePlane, tolerance: f64) -> Option<Curve> {
    let mut all: Vec<Piece> = segments.iter().flat_map(|s| pieces(s, tolerance)).collect();
    // The first turned to end where the second is.
    if let (Some(first), Some(second)) = (all.first(), all.get(1))
        && let (Some(a), Some(b), Some(c), Some(d)) =
            (first.start(), first.end(), second.start(), second.end())
        && dist(a, c).min(dist(a, d)) < dist(b, c).min(dist(b, d))
    {
        let turned = all.remove(0).reversed();
        all.insert(0, turned);
    }
    // Each after it turned to start where the one before ends.
    for i in 1..all.len() {
        let (Some(before), Some(piece)) = (all.get(i - 1), all.get(i)) else {
            continue;
        };
        let (Some(at), Some(start), Some(end)) = (before.end(), piece.start(), piece.end()) else {
            continue;
        };
        if dist(end, at) < dist(start, at) {
            let turned = all.remove(i).reversed();
            all.insert(i, turned);
        }
    }
    let start = all.first()?.start()?;
    let mut out = Vec::new();
    for piece in all {
        match piece {
            Piece::Line(_, b) => out.push(Segment::Line {
                to: place(plane, b),
            }),
            Piece::Arc(_, m, b) => out.push(Segment::Arc {
                via: place(plane, m),
                to: place(plane, b),
            }),
            Piece::Points(p) => out.extend(p.into_iter().skip(1).map(|q| Segment::Line {
                to: place(plane, q),
            })),
        }
    }
    Some(Curve {
        start: place(plane, start),
        segments: out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use printcad_bench_sdk::api::kernel_api::ProfileWire;

    fn on_xy(z: f64, segments: Vec<ProfileSegment>) -> Profile {
        Profile {
            plane: ProfilePlane {
                origin: [0.0, 0.0, z],
                ..ProfilePlane::default()
            },
            wires: vec![ProfileWire::new(segments)],
        }
    }

    /// Lines and arcs come through as they are, at the plane's height, a
    /// circle as two half arcs.
    #[test]
    fn lines_and_arcs_stay_lines_and_arcs() {
        let slot = on_xy(
            5.0,
            vec![
                ProfileSegment::Line {
                    start: [0.0, 0.0],
                    end: [10.0, 0.0],
                },
                ProfileSegment::Arc {
                    start: [10.0, 0.0],
                    mid: [12.0, 2.0],
                    end: [10.0, 4.0],
                },
                // Drawn the other way: turned round.
                ProfileSegment::Line {
                    start: [0.0, 4.0],
                    end: [10.0, 4.0],
                },
                ProfileSegment::Line {
                    start: [0.0, 4.0],
                    end: [0.0, 0.0],
                },
            ],
        );
        let slots = curves(&slot, 0.01);
        assert_eq!(slots.len(), 1);
        let c = &slots[0];
        assert_eq!(c.start, [0.0, 0.0, 5.0]);
        assert_eq!(c.segments.len(), 4);
        assert!(matches!(
            c.segments[1],
            Segment::Arc {
                via: [12.0, 2.0, 5.0],
                ..
            }
        ));
        assert_eq!(c.segments[2].to(), [0.0, 4.0, 5.0]);
        assert!(c.closed(1e-9));

        let round = on_xy(
            0.0,
            vec![ProfileSegment::Circle {
                center: [1.0, 1.0],
                radius: 2.0,
            }],
        );
        let c = &curves(&round, 0.01)[0];
        assert_eq!(c.segments.len(), 2);
        assert!(c.segments.iter().all(|s| matches!(s, Segment::Arc { .. })));
        assert!(c.closed(1e-9));
    }

    /// A spline comes as lines that stay within the tolerance of it: a
    /// quadratic through three points is a parabola, checked between them.
    #[test]
    fn a_spline_is_followed_within_the_tolerance() {
        let tolerance = 0.01;
        let profile = on_xy(
            0.0,
            vec![
                ProfileSegment::Nurbs {
                    degree: 2,
                    knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    control_points: vec![[0.0, 0.0], [5.0, 10.0], [10.0, 0.0]],
                    weights: vec![],
                    periodic: false,
                },
                ProfileSegment::Line {
                    start: [10.0, 0.0],
                    end: [0.0, 0.0],
                },
            ],
        );
        let c = &curves(&profile, tolerance)[0];
        assert!(c.segments.len() > 8, "{}", c.segments.len());
        assert!(c.closed(1e-9));
        // y = x (10 - x) / 5 between the ends: the parabola's. Square to
        // each chord, its middle is within the tolerance of it.
        let mut at = c.start;
        for s in &c.segments[..c.segments.len() - 1] {
            let to = s.to();
            let mid = [(at[0] + to[0]) / 2.0, (at[1] + to[1]) / 2.0];
            let want = mid[0] * (10.0 - mid[0]) / 5.0;
            let slope = (to[1] - at[1]) / (to[0] - at[0]);
            let off = (mid[1] - want).abs() / slope.hypot(1.0);
            assert!(off <= tolerance * 1.01, "{mid:?} vs {want}");
            at = to;
        }
    }

    /// A sketch on a plane that is not level is placed on it.
    #[test]
    fn a_sketch_is_placed_on_its_plane() {
        let mut profile = on_xy(
            0.0,
            vec![ProfileSegment::Circle {
                center: [0.0, 0.0],
                radius: 1.0,
            }],
        );
        profile.plane = ProfilePlane {
            origin: [0.0, 0.0, 3.0],
            x_axis: [0.0, 1.0, 0.0],
            y_axis: [0.0, 0.0, 1.0],
            normal: [1.0, 0.0, 0.0],
        };
        let c = &curves(&profile, 0.01)[0];
        assert_eq!(c.start, [0.0, 1.0, 3.0]);
    }
}
