//! A setup's run as moments to play back: the material, coarsely, and where the
//! tool is, at moments spread along its moves.
//!
//! A setup of several parts is played back as its program cuts them, each
//! moment the material of the part being cut. Each part is cut as the first
//! is, so the material at a part, so far into an operation, is the first's
//! there: one sweep through the operations in turn gives every part's.
//!
//! The material is swept again from the stock, move by move, with the tools
//! the run cut with, on a grid as coarse as the one drawn. Sweeping the run's
//! own, as camcraft's app does to show it closely, would take longer than the
//! run itself; drawn coarsely, the coarse sweep looks the same.

use cam_protocol::{Moment, Playback};
use camcraft_core::job::Job;
use camcraft_core::path::simadapt::sim_moves;
use camcraft_core::run::cutting_tool;
use camcraft_core::setup::bounds_all;
use camcraft_session::Ran;
use camcraft_sim::stock::{Axis, Block, Carry, Stock, round_pitch};
use glam::{DAffine3, DVec2, DVec3};

use crate::cut::Cut;
use crate::{Drawn, to_f32};

/// How many moments a run is played back in, at most, besides its start.
const MOMENTS: usize = 60;

/// How many columns a side the played-back material has, at most.
const COLUMNS: usize = 60;

/// How many cells a side the material is swept on, at most: finer than it is
/// drawn, so a pass narrower than a drawn column still shows.
const SWEPT: usize = 240;

/// `ran`'s setup as moments, `draws` being its operations as drawn and `cut`
/// the order its program cuts them at its parts; `None` when it cut
/// nothing or cannot be replayed.
pub(crate) fn playback(job: &Job, ran: &Ran, draws: &[Drawn], cut: &Cut) -> Option<Playback> {
    let r = &ran.result;
    let setup = job.setups.first()?;
    let moves_of = |op: usize| draws.get(op).map_or(0, |d| d.ends.len());
    let total: usize = cut.sections.iter().map(|s| moves_of(s.op)).sum();
    if total == 0 {
        return None;
    }

    // The grid, at height zero, and the tool's axis.
    let (lo, hi) = (r.stock_min, r.stock_max);
    let span = hi - lo;
    if !(span.x > 0.0 && span.y > 0.0) {
        return None;
    }
    let pitch = span.x.max(span.y) / COLUMNS as f64;
    let nx = ((span.x / pitch).ceil() as usize).max(1) + 1;
    let ny = ((span.y / pitch).ceil() as usize).max(1) + 1;
    let step = DVec2::new(span.x / (nx - 1) as f64, span.y / (ny - 1) as f64);
    let grid: Vec<DVec2> = (0..ny)
        .flat_map(|j| (0..nx).map(move |i| (i, j)))
        .map(|(i, j)| lo + step * DVec2::new(i as f64, j as f64))
        .collect();
    let base = grid
        .iter()
        .map(|p| to_f32(r.frame.shown(p.extend(0.0))))
        .collect();
    let zero = r.frame.to_model(DVec3::ZERO);
    let axis = to_f32(r.frame.to_model(DVec3::Z) - zero);
    // On a rotary each point stands out from the axis its own way.
    let (axes, about) = match r.frame.wrap {
        Some(w) => {
            let axes = grid
                .iter()
                .map(|p| to_f32(r.frame.shown(p.extend(1.0)) - r.frame.shown(p.extend(0.0))))
                .collect();
            let along = if w.along == Axis::Y {
                DVec3::Y
            } else {
                DVec3::X
            };
            let dir = r.frame.to_model(along) - zero;
            (axes, Some([to_f32(zero), to_f32(dir)]))
        }
        None => (Vec::new(), None),
    };
    let indices = crate::grid_indices(nx, ny, lo, step, &r.stocks);
    let low = r
        .stocks
        .iter()
        .map(|b| b.bounds().min.z)
        .fold(f64::INFINITY, f64::min);
    let high = r
        .stocks
        .iter()
        .map(Block::top)
        .fold(f64::NEG_INFINITY, f64::max);
    // A stock with no height, or none at all (the folds start infinite).
    if (high - low).partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return None;
    }
    let quantum = (high - low) / f64::from(u16::MAX);

    // The stock, swept on a coarse grid of its own: the setup's blocks, or
    // what an earlier setup left, as the run started from it.
    let coarse = span.x.max(span.y) / SWEPT as f64;
    let first = r.keyframes.first()?.unpack().ok()?;
    let round = r.frame.wrap.map(|w| w.round());
    let coarse = round.map_or(coarse, |round| round_pitch(&round, coarse));
    let mut stock = if first.under().is_some() {
        let all = bounds_all(&r.stocks)?;
        let same = Carry {
            to: DAffine3::IDENTITY,
            within: all,
        };
        first.carried(&same, all, low - 1.0, coarse).ok()?
    } else {
        let fresh = Stock::from_blocks(&r.stocks, low, coarse).ok()?;
        match round {
            Some(round) => fresh.rounded(round).ok()?,
            None => fresh,
        }
    };
    let floor = stock.floor();
    let heights = |stock: &Stock| -> Vec<u16> {
        grid.iter()
            .map(|p| {
                let z = stock.height_at(p.x, p.y).unwrap_or(floor);
                ((z - low) / quantum)
                    .round()
                    .clamp(0.0, f64::from(u16::MAX)) as u16
            })
            .collect()
    };

    // The places shown: the start, then moments spread evenly over all the
    // sections' moves, each section's last among them so every one ends on
    // one.
    let mut places = vec![(0, 0)];
    let mut done = 0;
    for (s, section) in cut.sections.iter().enumerate() {
        let n = moves_of(section.op);
        places.extend(
            (1..=MOMENTS)
                .map(|f| f * total / MOMENTS)
                .filter(|g| *g > done && *g < done + n)
                .map(|g| (s, g - done)),
        );
        if n > 0 {
            places.push((s, n));
        }
        done += n;
    }

    // The material at each place's operation and moves, swept once through
    // the operations in turn. The simulator's moves need not be the
    // toolpath's one for one; they run in step.
    let mut wanted: Vec<(usize, usize)> = places
        .iter()
        .filter_map(|&(s, i)| Some((cut.sections.get(s)?.op, i)))
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    let mut sweeps = Vec::with_capacity(draws.len());
    for (k, ran_op) in r.operations.iter().enumerate().take(draws.len()) {
        sweeps.push(if ran_op.made.has_paths() {
            let tool = cutting_tool(job, setup.operations.get(k)?).ok()?;
            Some((tool, sim_moves(&ran_op.path)))
        } else {
            None
        });
    }
    let mut swept_to = (0, 0);
    let mut material = Vec::with_capacity(wanted.len());
    for &(op, i) in &wanted {
        while swept_to.0 <= op {
            let k = swept_to.0;
            let sim = sweeps
                .get(k)
                .and_then(Option::as_ref)
                .map_or(0, |(_, m)| m.len());
            let n = moves_of(k);
            let target = if k < op {
                sim
            } else if n == 0 {
                0
            } else {
                (i * sim).div_ceil(n)
            };
            if let Some(Some((tool, moves))) = sweeps.get(k)
                && target > swept_to.1
            {
                // From the move before, which the sweep starts from.
                if let Some(slice) = moves.get(swept_to.1.saturating_sub(1)..target) {
                    stock.sweep(slice, tool);
                }
            }
            if k < op {
                swept_to = (k + 1, 0);
            } else {
                swept_to.1 = swept_to.1.max(target);
                break;
            }
        }
        material.push(heights(&stock));
    }

    let moments = places
        .iter()
        .filter_map(|&(s, i)| {
            let section = cut.sections.get(s)?;
            let drawn = draws.get(section.op)?;
            let at = wanted.binary_search(&(section.op, i)).ok()?;
            let tool = match i {
                0 => drawn.ends.first(),
                _ => drawn.ends.get(i - 1),
            }
            .map_or([0.0; 3], |p| to_f32(r.frame.shown(*p + cut.offset(s))));
            Some(Moment {
                section: s,
                op: section.op,
                part: section.part,
                moves: i,
                of: drawn.ends.len(),
                tool,
                drawn: drawn.drawn_after(i),
                seconds: section.start
                    + i.checked_sub(1)
                        .and_then(|j| drawn.times.get(j))
                        .copied()
                        .unwrap_or(0.0),
                heights: material.get(at)?.clone(),
            })
        })
        .collect();
    Some(Playback {
        base,
        axis,
        axes,
        about,
        indices,
        low: low as f32,
        step: quantum as f32,
        moments,
    })
}
