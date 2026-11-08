//! Rotary roughing and finishing, on a setup turned on a rotary
//! ([`crate::setup::frame::Unrolled`]): the tool always on a radius, its
//! passes along the bar as the rotary indexes between them, or one helix
//! round it, the bar turning as the tool goes along.
//!
//! Planned in the bar unrolled, on the part as the simulator holds it: how
//! far out from the axis the part reaches in each cell, along and round.
//! At each point a pass goes through, the tool's tip comes down its radius
//! until it touches the part: a cell round from it is a ray out from the
//! axis at an angle `dθ` to the tool's, which meets the tool's side at
//! `ρ sin dθ` across and `ρ cos dθ` up, so the tip may come no nearer the
//! axis than `ρ cos dθ` less the tool's height there. That is exact for the
//! part as the grid has it, and the same rule the simulator cuts by, so
//! what it plans it verifies.
//!
//! Above the part it goes no lower than each level, a stepdown apart from
//! the stock's top when it roughs, so it takes the bar down a level at a
//! time, and last on the part: the first level that is on the part
//! everywhere is the last.

use camcraft_geom::heightmap::HeightMap;
use camcraft_sim::stock::{Axis, Round};
use glam::{DVec2, DVec3};
use rayon::prelude::*;

use crate::cutter::tool::ToolProfile;
use crate::path::toolpath::{MoveRole, Toolpath};
use crate::strategy::StrategyError;

/// How a rotary finish goes round the bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotaryPattern {
    /// Passes along the bar, a stepover apart round it, back and forth.
    Along,
    /// One helix round the bar, a stepover along it each turn.
    Helical,
}

/// What a rotary finish is asked for.
#[derive(Debug, Clone)]
pub struct RotaryParams {
    /// How the grid goes round.
    pub round: Round,
    /// From and to, along the bar, that the tool's tip goes.
    pub along: (f64, f64),
    /// Apart round the bar (along) or along it a turn (helical), mm.
    pub stepover: f64,
    /// How the passes go.
    pub pattern: RotaryPattern,
    /// The levels it goes down to, from the highest: it goes no lower than
    /// each. The last is how low it may go anywhere.
    pub levels: Vec<f64>,
    /// How far the line between two points it keeps may stray from those it
    /// leaves out.
    pub tolerance: f64,
    /// Cutting feed, mm/min.
    pub feed: f64,
}

/// What it made.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RotaryReport {
    /// Passes, at every level.
    pub passes: u64,
    /// Points sampled on the part.
    pub samples: u64,
}

/// The part as the simulator holds it: its distance out from the axis in
/// each cell, and how the grid goes round.
pub struct Unrolled<'a> {
    part: &'a HeightMap,
    round: Round,
    /// Cells in a turn.
    turn: u32,
}

impl<'a> Unrolled<'a> {
    /// `part`, a window of the stock's grid going round as `round` says from
    /// `first`, the grid's first cell; `None` unless whole cells make a turn.
    pub fn new(part: &'a HeightMap, round: Round, first: DVec2) -> Option<Self> {
        let pitch = part.pitch();
        let turn = (round.period() / pitch).round();
        if (turn * pitch - round.period()).abs() > 1e-6 * round.period() || turn < 3.0 {
            return None;
        }
        // The window's place in the grid must be whole cells from its start.
        let off = (part.origin() - first) / pitch;
        if (off - off.round()).abs().max_element() > 1e-6 {
            return None;
        }
        // Bounded and whole, so the narrowing is exact.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let turn = turn as u32;
        Some(Self { part, round, turn })
    }

    /// The part's distance out at cell `(ia, k)`, `k` counted round from the
    /// window's first row of the round axis and wrapped into a turn.
    fn at(&self, ia: i64, k: i64) -> Option<f64> {
        let (na, nr) = match self.round.round {
            Axis::X => (self.part.ny(), self.part.nx()),
            _ => (self.part.nx(), self.part.ny()),
        };
        let ia = u32::try_from(ia).ok().filter(|i| *i < na)?;
        let turn = i64::from(self.turn);
        let mut ir = k.rem_euclid(turn);
        // The window may be less than a turn round: then where it is not, the
        // part is not.
        if ir >= i64::from(nr) {
            ir -= turn;
        }
        let ir = u32::try_from(ir).ok().filter(|i| *i < nr)?;
        let h = match self.round.round {
            Axis::X => self.part.get(ir, ia),
            _ => self.part.get(ia, ir),
        }?;
        h.is_finite().then_some(h)
    }

    /// The nearest the axis the tip of `tool` may come on the radius at
    /// `along` and `at` round without going into the part, and never nearer
    /// than `floor`.
    // Cell counts far inside f64's whole numbers.
    #[allow(clippy::cast_precision_loss)]
    pub fn tip<T: ToolProfile>(&self, tool: &T, along: f64, at: f64, floor: f64) -> f64 {
        let rt = tool.max_radius();
        let pitch = self.part.pitch();
        let origin = self.part.origin();
        let (oa, or) = match self.round.round {
            Axis::X => (origin.y, origin.x),
            _ => (origin.x, origin.y),
        };
        let radius = self.round.radius;
        let a0 = ((along - rt - oa) / pitch).floor();
        let a1 = ((along + rt - oa) / pitch).ceil();
        let centre = ((at - or) / pitch).round();
        if !(a0.is_finite() && a1.is_finite() && centre.is_finite()) {
            return floor;
        }
        // Bounded by the grid's size, so the narrowing is exact.
        #[allow(clippy::cast_possible_truncation)]
        let (a0, a1, centre) = (a0 as i64, a1 as i64, centre as i64);
        let mut best = floor;
        let half = i64::from(self.turn / 4);
        // Out from the row under the tool, each way, while a cell that far
        // round could still be met higher than the best yet: its side is
        // `rt` across, so a ray `dθ` round meets it no higher than
        // `rt / tan dθ`.
        for step in 0..=half {
            let mut any = false;
            for k in [centre + step, centre - step] {
                if step == 0 && k != centre {
                    continue;
                }
                let s = or + k as f64 * pitch;
                let dth = (s - at) / radius;
                let (sin, cos) = dth.sin_cos();
                if cos <= 1e-9 {
                    continue;
                }
                if sin.abs() > 1e-12 && rt / (sin.abs() / cos) < best {
                    continue;
                }
                any = true;
                for ia in a0..=a1 {
                    let dx = oa + ia as f64 * pitch - along;
                    if dx.abs() > rt {
                        continue;
                    }
                    let Some(rho) = self.at(ia, k) else {
                        continue;
                    };
                    // The highest the column comes inside the tool's side.
                    let side = if sin.abs() > 1e-12 {
                        (rt * rt - dx * dx).max(0.0).sqrt() / sin.abs()
                    } else {
                        f64::INFINITY
                    };
                    let rho = rho.min(side);
                    let d = dx.hypot(rho * sin);
                    let Some(h) = tool.height_at(d.min(rt)) else {
                        continue;
                    };
                    best = best.max(rho * cos - h);
                }
            }
            if !any && step > 0 {
                break;
            }
        }
        best
    }
}

/// A rotary finish (or, with levels above the part, a roughing) of `part`
/// with `tool`, as `p` asks: the passes each a link to their start for the
/// linker, then cuts.
// Pass and sample counts far inside f64's whole numbers.
#[allow(clippy::cast_precision_loss)]
pub fn rotary_finish<T: ToolProfile + Sync>(
    part: &Unrolled<'_>,
    tool: &T,
    p: &RotaryParams,
) -> Result<(Toolpath, RotaryReport), StrategyError> {
    if !(p.stepover > 0.0 && p.stepover.is_finite()) {
        return Err(StrategyError::BadParameter {
            name: "stepover",
            value: p.stepover,
        });
    }
    let period = p.round.period();
    let (lo, hi) = (p.along.0.min(p.along.1), p.along.0.max(p.along.1));
    // How far apart the points it samples first are, along the pass; it
    // samples between two again where the tool's tip strays from the line
    // between them by more than a quarter of the tolerance.
    let step = (tool.max_radius() * 0.25).clamp(part.part.pitch() * 0.5, 1.0);
    let mut path = Toolpath::new();
    let mut report = RotaryReport::default();
    let floor = p.levels.last().copied().unwrap_or(0.0);
    for &level in &p.levels {
        // Whether any point of the level is held up by it, above the part:
        // once none is, it went over the part everywhere, and the levels
        // below would go the same way again.
        let held = std::sync::atomic::AtomicBool::new(false);
        let point = |(along, round): (f64, f64)| -> DVec3 {
            let on = part.tip(tool, along, round, floor);
            if level > on + 1e-9 {
                held.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            let r = on.max(level);
            match p.round.round {
                Axis::X => DVec3::new(round, along, r),
                _ => DVec3::new(along, round, r),
            }
        };
        // Each pass from one end to the other, along and round.
        let runs: Vec<((f64, f64), (f64, f64))> = match p.pattern {
            RotaryPattern::Along => {
                let n = (period / p.stepover).ceil().max(1.0);
                let spacing = period / n;
                // Whole and at least one, so the narrowing is exact.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let n = n as usize;
                (0..n)
                    .map(|k| {
                        let round = k as f64 * spacing;
                        if k % 2 == 0 {
                            ((lo, round), (hi, round))
                        } else {
                            ((hi, round), (lo, round))
                        }
                    })
                    .collect()
            }
            // A turn a stepover along, from end to end, round on past a
            // turn as it goes.
            RotaryPattern::Helical => vec![((lo, 0.0), (hi, (hi - lo) / p.stepover * period))],
        };
        for (a, b) in runs {
            let at = |t: f64| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            let length = (b.0 - a.0).hypot(b.1 - a.1);
            let n = (length / step).ceil().clamp(1.0, 1e7);
            // Bounded, so the narrowing is exact.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let n = n as usize;
            let ts: Vec<f64> = (0..=n).map(|i| i as f64 / n as f64).collect();
            let first: Vec<(f64, DVec3)> = ts.par_iter().map(|&t| (t, point(at(t)))).collect();
            let mut points = Vec::with_capacity(first.len());
            for pair in first.windows(2) {
                let [a, b] = pair else { continue };
                points.push(a.1);
                refine(&point, &at, *a, *b, p.tolerance * 0.25, 6, &mut points);
            }
            if let Some(&(_, last)) = first.last() {
                points.push(last);
            }
            report.samples += points.len() as u64;
            let kept = crate::strategy::vcarve::simplified(&points, p.tolerance * 0.25);
            let Some((&start, rest)) = kept.split_first() else {
                continue;
            };
            path.feed_to(start, MoveRole::Link, p.feed)?;
            for &q in rest {
                path.feed_to(q, MoveRole::Cut, p.feed)?;
            }
            report.passes += 1;
        }
        if !held.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
    }
    Ok((path, report))
}

/// The points between `a` and `b`, at `t` along the pass `at` maps, where
/// the tip found there strays from the line between them by more than
/// `tolerance`, pushed in order onto `out`, `depth` times over at most.
fn refine(
    point: &impl Fn((f64, f64)) -> DVec3,
    at: &impl Fn(f64) -> (f64, f64),
    (t0, a): (f64, DVec3),
    (t1, b): (f64, DVec3),
    tolerance: f64,
    depth: u32,
    out: &mut Vec<DVec3>,
) {
    if depth == 0 {
        return;
    }
    let t = 0.5 * (t0 + t1);
    let m = point(at(t));
    if (m.z - 0.5 * (a.z + b.z)).abs() <= tolerance {
        return;
    }
    refine(point, at, (t0, a), (t, m), tolerance, depth - 1, out);
    out.push(m);
    refine(point, at, (t, m), (t1, b), tolerance, depth - 1, out);
}
