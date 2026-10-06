//! A pocket's toolpath, worked out in the plane: rows across the outline,
//! each cut where the tool's whole disc stays inside it, joined row to row
//! where the step between them stays inside too, and repeated at every
//! depth. Everything here is plain arithmetic on points, so the job and
//! the bench's own commands run the same code.

use printcad_bench_sdk::api::kernel_api::{Profile, ProfileSegment};

/// A point of the plane, in millimetres.
pub type P = [f64; 2];

/// The most rows one toolpath may have: past it the stepover is too fine
/// for the pocket.
pub const MAX_ROWS: usize = 200_000;

/// How far a flattened arc strays from the true one, at most.
const CHORD_TOLERANCE: f64 = 0.01;

/// What a toolpath is worked out from.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Input {
    /// Closed loops in world X and Y; holes (islands) inside outlines.
    pub loops: Vec<Vec<P>>,
    pub radius: f64,
    pub stepover: f64,
    pub top: f64,
    pub depth: f64,
    pub step_down: f64,
}

/// The cutting runs, each a line through points cut at one depth, and
/// the depths they are cut at, top down.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Toolpath {
    pub runs: Vec<Vec<P>>,
    pub levels: Vec<f64>,
}

impl Toolpath {
    /// The length the tool cuts, over every pass.
    pub fn cut_length(&self) -> f64 {
        let one: f64 = self
            .runs
            .iter()
            .flat_map(|run| run.windows(2))
            .map(|w| dist(w[0], w[1]))
            .sum();
        one * self.levels.len() as f64
    }

    pub fn points(&self) -> usize {
        self.runs.iter().map(Vec::len).sum()
    }
}

/// The depths from just under `top` down to `top - depth`, evenly
/// spaced, none further apart than `step_down`.
pub fn levels(top: f64, depth: f64, step_down: f64) -> Vec<f64> {
    let passes = ((depth / step_down) - 1e-9).ceil().max(1.0) as usize;
    (1..=passes)
        .map(|i| top - depth * i as f64 / passes as f64)
        .collect()
}

/// The rows the toolpath runs along, bottom to top: the outline's height
/// less the tool on both sides, divided no finer than `stepover`.
fn rows(loops: &[Vec<P>], radius: f64, stepover: f64) -> Result<Vec<f64>, String> {
    let ys = loops.iter().flatten().map(|p| p[1]);
    let low = ys.clone().fold(f64::INFINITY, f64::min);
    let high = ys.fold(f64::NEG_INFINITY, f64::max);
    // Just inside the tool's reach of the lowest and highest edges, so a
    // flat edge does not take the whole row.
    let (low, high) = (low + radius + 1e-6, high - radius - 1e-6);
    if high < low {
        return Err("The tool is wider than the pocket.".into());
    }
    let count = ((high - low) / stepover).ceil() as usize + 1;
    if count > MAX_ROWS {
        return Err(format!(
            "The stepover is too fine for this pocket: it makes {count} rows, \
             at most {MAX_ROWS}."
        ));
    }
    Ok(if count == 1 {
        vec![(low + high) / 2.0]
    } else {
        (0..count)
            .map(|i| low + (high - low) * i as f64 / (count - 1) as f64)
            .collect()
    })
}

/// The runs that clear `input`'s loops at one depth. `going(done, of)` is
/// told of every row; when it answers `false` the work stops.
pub fn plan(
    input: &Input,
    mut going: impl FnMut(usize, usize) -> bool,
) -> Result<Toolpath, String> {
    if input.radius <= 0.0 || input.stepover <= 0.0 {
        return Err("The tool and its stepover must be more than 0.".into());
    }
    let edges = edges(&input.loops);
    if edges.len() < 3 {
        return Err("The outline closes no area.".into());
    }
    let rows = rows(&input.loops, input.radius, input.stepover)?;
    let mut runs: Vec<Vec<P>> = Vec::new();
    let mut run: Vec<P> = Vec::new();
    for (i, y) in rows.iter().enumerate() {
        if !going(i, rows.len()) {
            return Err("stopped".into());
        }
        let mut spans = clear_spans(&edges, *y, input.radius);
        // Back and forth, row after row.
        if i % 2 == 1 {
            spans.reverse();
            for span in &mut spans {
                *span = (span.1, span.0);
            }
        }
        for (from, to) in spans {
            let (a, b) = ([from, *y], [to, *y]);
            let joins = run
                .last()
                .is_some_and(|last| step_clears(&edges, *last, a, input.radius));
            if !joins && !run.is_empty() {
                runs.push(std::mem::take(&mut run));
            }
            run.push(a);
            run.push(b);
        }
    }
    if !run.is_empty() {
        runs.push(run);
    }
    if runs.is_empty() {
        return Err("The tool fits nowhere inside the outline.".into());
    }
    Ok(Toolpath {
        runs,
        levels: levels(input.top, input.depth, input.step_down),
    })
}

fn edges(loops: &[Vec<P>]) -> Vec<(P, P)> {
    loops
        .iter()
        .flat_map(|l| l.iter().zip(l.iter().cycle().skip(1)))
        .filter(|(a, b)| dist(**a, **b) > 1e-12)
        .map(|(a, b)| (*a, *b))
        .collect()
}

fn dist(a: P, b: P) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Where along the row at `y` the tool's centre may stand: inside the
/// loops (even-odd, so islands are holes) and at least `radius` from every
/// edge.
fn clear_spans(edges: &[(P, P)], y: f64, radius: f64) -> Vec<(f64, f64)> {
    let mut crossings: Vec<f64> = edges
        .iter()
        .filter(|(a, b)| (a[1] > y) != (b[1] > y))
        .map(|(a, b)| a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]))
        .collect();
    crossings.sort_by(f64::total_cmp);
    let mut spans: Vec<(f64, f64)> = crossings
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[from, to]| (*from, *to))
        .collect();
    for (a, b) in edges {
        if let Some(near) = too_near(*a, *b, y, radius) {
            spans = without(spans, near);
        }
    }
    spans.retain(|(from, to)| to - from > 1e-9);
    spans
}

/// The stretch of the row at `y` closer than `radius` to the edge from `a` to `b`:
/// the row through the capsule around the edge, which is convex, so one
/// stretch, the hull of the row through its two end discs and its band.
fn too_near(a: P, b: P, y: f64, radius: f64) -> Option<(f64, f64)> {
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for c in [a, b] {
        let dy = y - c[1];
        if dy.abs() < radius {
            let half = (radius * radius - dy * dy).sqrt();
            low = low.min(c[0] - half);
            high = high.max(c[0] + half);
        }
    }
    let length = dist(a, b);
    let along = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
    let across = [-along[1], along[0]];
    // Each bound is linear in x along the row: k x + c within [min, max].
    let (mut from, mut to) = (f64::NEG_INFINITY, f64::INFINITY);
    let mut clip = |k: f64, c: f64, min: f64, max: f64| {
        if k.abs() < 1e-15 {
            if c <= min || c >= max {
                from = f64::INFINITY;
                to = f64::NEG_INFINITY;
            }
            return;
        }
        let (x1, x2) = ((min - c) / k, (max - c) / k);
        from = from.max(x1.min(x2));
        to = to.min(x1.max(x2));
    };
    clip(
        along[0],
        (y - a[1]) * along[1] - a[0] * along[0],
        0.0,
        length,
    );
    clip(
        across[0],
        (y - a[1]) * across[1] - a[0] * across[0],
        -radius,
        radius,
    );
    if from < to {
        low = low.min(from);
        high = high.max(to);
    }
    (low < high).then_some((low, high))
}

/// `spans` less the open stretch `(low, high)`; its ends stay.
fn without(spans: Vec<(f64, f64)>, (low, high): (f64, f64)) -> Vec<(f64, f64)> {
    let mut out = Vec::with_capacity(spans.len() + 1);
    for (from, to) in spans {
        if to <= low || from >= high {
            out.push((from, to));
            continue;
        }
        if from <= low {
            out.push((from, low));
        }
        if to >= high {
            out.push((high, to));
        }
    }
    out
}

/// Whether the tool can go straight from `a` to `b`, both clear, without
/// coming nearer than `radius` to any edge.
fn step_clears(edges: &[(P, P)], a: P, b: P, radius: f64) -> bool {
    edges
        .iter()
        .all(|(p, q)| segment_distance(a, b, *p, *q) >= radius - 1e-9)
}

fn segment_distance(a: P, b: P, p: P, q: P) -> f64 {
    let side = |o: P, s: P, t: P| (s[0] - o[0]) * (t[1] - o[1]) - (s[1] - o[1]) * (t[0] - o[0]);
    let crosses = side(a, b, p).signum() * side(a, b, q).signum() < 0.0
        && side(p, q, a).signum() * side(p, q, b).signum() < 0.0;
    if crosses {
        return 0.0;
    }
    point_distance(a, p, q)
        .min(point_distance(b, p, q))
        .min(point_distance(p, a, b))
        .min(point_distance(q, a, b))
}

fn point_distance(x: P, a: P, b: P) -> f64 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let length2 = d[0] * d[0] + d[1] * d[1];
    let t = if length2 > 0.0 {
        (((x[0] - a[0]) * d[0] + (x[1] - a[1]) * d[1]) / length2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    dist(x, [a[0] + d[0] * t, a[1] + d[1] * t])
}

/// A profile's loops in world X and Y, and the height of its plane: it
/// must lie square to Z, as a pocket milled from above does.
pub fn loops_of(profile: &Profile) -> Result<(Vec<Vec<P>>, f64), String> {
    let plane = &profile.plane;
    if plane.normal[2].abs() < 1.0 - 1e-6 {
        return Err("The outline must lie flat: its sketch square to Z.".into());
    }
    let world = |p: P| {
        [
            plane.origin[0] + p[0] * plane.x_axis[0] + p[1] * plane.y_axis[0],
            plane.origin[1] + p[0] * plane.x_axis[1] + p[1] * plane.y_axis[1],
        ]
    };
    let mut loops = Vec::new();
    for wire in &profile.wires {
        let mut points: Vec<P> = Vec::new();
        for (i, segment) in wire.segments.iter().enumerate() {
            let mut piece = flatten(segment)?;
            let (Some(&last), Some(&first), Some(&end)) =
                (points.last(), piece.first(), piece.last())
            else {
                points = piece;
                continue;
            };
            // The first piece turns round when the second meets its start.
            if i == 1
                && dist(points[0], first).min(dist(points[0], end))
                    < dist(last, first).min(dist(last, end))
            {
                points.reverse();
            }
            let last = points[points.len() - 1];
            if dist(last, end) < dist(last, first) {
                piece.reverse();
            }
            points.extend(piece.into_iter().skip(1));
        }
        if points.len() > 1 && dist(points[0], points[points.len() - 1]) < 1e-6 {
            points.pop();
        }
        if points.len() >= 3 {
            loops.push(points.into_iter().map(world).collect());
        }
    }
    if loops.is_empty() {
        return Err("The outline closes no area.".into());
    }
    Ok((loops, plane.origin[2]))
}

/// Points along a segment, start to end, no further from it than the
/// chord tolerance.
fn flatten(segment: &ProfileSegment) -> Result<Vec<P>, String> {
    let steps = |radius: f64, sweep: f64| {
        let per = 2.0 * (1.0 - CHORD_TOLERANCE / radius.max(CHORD_TOLERANCE)).acos();
        ((sweep.abs() / per.max(1e-3)).ceil() as usize).clamp(8, 4096)
    };
    let around = |center: P, radius: f64, start: f64, sweep: f64, n: usize| -> Vec<P> {
        (0..=n)
            .map(|i| {
                let a = start + sweep * i as f64 / n as f64;
                [center[0] + radius * a.cos(), center[1] + radius * a.sin()]
            })
            .collect()
    };
    let tau = std::f64::consts::TAU;
    Ok(match segment {
        ProfileSegment::Line { start, end } => vec![*start, *end],
        ProfileSegment::Arc { start, mid, end } => {
            let center = circumcentre(*start, *mid, *end)
                .ok_or("An arc of the outline is a straight line.")?;
            let radius = dist(center, *start);
            let angle = |p: P| (p[1] - center[1]).atan2(p[0] - center[0]);
            let a0 = angle(*start);
            let to_end = (angle(*end) - a0).rem_euclid(tau);
            let to_mid = (angle(*mid) - a0).rem_euclid(tau);
            let sweep = if to_mid <= to_end {
                to_end
            } else {
                to_end - tau
            };
            around(center, radius, a0, sweep, steps(radius, sweep))
        }
        ProfileSegment::Circle { center, radius } => {
            around(*center, *radius, 0.0, tau, steps(*radius, tau))
        }
        ProfileSegment::Ellipse {
            center,
            major,
            ratio,
        } => ellipse(*center, *major, *ratio, 0.0, tau),
        ProfileSegment::EllipseArc {
            center,
            major,
            ratio,
            start_param,
            end_param,
        } => {
            let sweep = (end_param - start_param).rem_euclid(tau);
            ellipse(*center, *major, *ratio, *start_param, sweep)
        }
        ProfileSegment::BSpline { .. } | ProfileSegment::Nurbs { .. } => {
            return Err(
                "The pocket follows lines, arcs, circles and ellipses, not splines.".into(),
            );
        }
        _ => {
            return Err("The outline holds a kind of curve the pocket does not follow.".into());
        }
    })
}

fn ellipse(center: P, major: P, ratio: f64, start: f64, sweep: f64) -> Vec<P> {
    let minor = [-major[1] * ratio, major[0] * ratio];
    let n = 256;
    (0..=n)
        .map(|i| {
            let t = start + sweep * i as f64 / n as f64;
            [
                center[0] + major[0] * t.cos() + minor[0] * t.sin(),
                center[1] + major[1] * t.cos() + minor[1] * t.sin(),
            ]
        })
        .collect()
}

fn circumcentre(a: P, b: P, c: P) -> Option<P> {
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    if d.abs() < 1e-12 {
        return None;
    }
    let sq = |p: P| p[0] * p[0] + p[1] * p[1];
    Some([
        (sq(a) * (b[1] - c[1]) + sq(b) * (c[1] - a[1]) + sq(c) * (a[1] - b[1])) / d,
        (sq(a) * (c[0] - b[0]) + sq(b) * (a[0] - c[0]) + sq(c) * (b[0] - a[0])) / d,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<P> {
        vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
    }

    fn input(loops: Vec<Vec<P>>, radius: f64, stepover: f64) -> Input {
        Input {
            loops,
            radius,
            stepover,
            top: 0.0,
            depth: 3.0,
            step_down: 1.0,
        }
    }

    /// Every cut, along each run, keeps the tool's centre at least
    /// `radius` from every edge of the loops.
    fn assert_clear(path: &Toolpath, loops: &[Vec<P>], radius: f64) {
        let edges = edges(loops);
        for run in &path.runs {
            for w in run.windows(2) {
                for (p, q) in &edges {
                    let d = segment_distance(w[0], w[1], *p, *q);
                    assert!(
                        d >= radius - 1e-6,
                        "{:?}..{:?} comes {d} from an edge",
                        w[0],
                        w[1]
                    );
                }
            }
        }
    }

    #[test]
    fn a_square_pocket_stays_inside_the_outline_shrunk_by_the_tool() {
        let loops = vec![rect(0.0, 0.0, 40.0, 30.0)];
        let path = plan(&input(loops.clone(), 3.0, 2.4), |_, _| true).unwrap();
        assert_clear(&path, &loops, 3.0);
        let points: Vec<P> = path.runs.iter().flatten().copied().collect();
        for p in &points {
            assert!((3.0 - 1e-6..=37.0 + 1e-6).contains(&p[0]), "{p:?}");
            assert!((3.0 - 1e-6..=27.0 + 1e-6).contains(&p[1]), "{p:?}");
        }
        // The rows reach both sides of the shrunk outline, no further apart
        // than the stepover.
        let mut ys: Vec<f64> = points.iter().map(|p| p[1]).collect();
        ys.sort_by(f64::total_cmp);
        ys.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        assert!(ys[0] < 3.0 + 1e-3 && ys[ys.len() - 1] > 27.0 - 1e-3);
        assert!(ys.windows(2).all(|w| w[1] - w[0] <= 2.4 + 1e-9));
        // A convex pocket is one zig-zag, the tool never lifting.
        assert_eq!(path.runs.len(), 1);
    }

    #[test]
    fn the_tool_goes_around_an_island() {
        let loops = vec![rect(0.0, 0.0, 60.0, 60.0), rect(20.0, 20.0, 40.0, 40.0)];
        let path = plan(&input(loops.clone(), 2.0, 1.5), |_, _| true).unwrap();
        assert_clear(&path, &loops, 2.0);
        for p in path.runs.iter().flatten() {
            let inside_island = (20.0..=40.0).contains(&p[0]) && (20.0..=40.0).contains(&p[1]);
            assert!(!inside_island, "{p:?} is on the island");
        }
        // Rows beside the island are cut on both sides of it.
        let beside = |x: f64| {
            path.runs.iter().flat_map(|r| r.windows(2)).any(|w| {
                (w[0][1] - 30.0).abs() < 1.0
                    && w[0][0].min(w[1][0]) <= x
                    && x <= w[0][0].max(w[1][0])
            })
        };
        assert!(beside(10.0) && beside(50.0));
    }

    #[test]
    fn passes_reach_the_floor_no_deeper_than_the_depth_per_pass() {
        for (top, depth, step) in [
            (0.0, 3.0, 1.0),
            (5.0, 2.5, 1.0),
            (-1.0, 0.4, 1.0),
            (2.0, 7.0, 0.3),
        ] {
            let levels = levels(top, depth, step);
            assert!((levels[levels.len() - 1] - (top - depth)).abs() < 1e-9);
            let mut above = top;
            for z in &levels {
                assert!(
                    above - z > 0.0 && above - z <= step + 1e-9,
                    "{above} to {z}"
                );
                above = *z;
            }
        }
        assert_eq!(levels(0.0, 3.0, 1.0).len(), 3);
    }

    #[test]
    fn a_tool_wider_than_the_pocket_is_refused() {
        let loops = vec![rect(0.0, 0.0, 5.0, 5.0)];
        assert!(plan(&input(loops, 3.0, 1.0), |_, _| true).is_err());
    }

    #[test]
    fn a_stop_ends_the_work() {
        let loops = vec![rect(0.0, 0.0, 40.0, 40.0)];
        let result = plan(&input(loops, 1.0, 0.5), |done, _| done < 5);
        assert_eq!(result, Err("stopped".into()));
    }
}
