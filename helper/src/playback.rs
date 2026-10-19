//! A setup's run as moments to play back: the material, coarsely, and where the
//! tool is, at moments spread along its moves.
//!
//! The material is swept again from the stock, move by move, with the tools
//! the run cut with, on a grid as coarse as the one drawn. Sweeping the run's
//! own, as camcraft's app does to show it closely, would take longer than the
//! run itself; drawn coarsely, the coarse sweep looks the same.

use cam_protocol::{Moment, Playback};
use camcraft_core::job::Job;
use camcraft_core::path::simadapt::sim_moves;
use camcraft_core::run::cutting_tool;
use camcraft_session::Ran;
use camcraft_sim::stock::Stock;
use glam::{DVec2, DVec3};

use crate::{Drawn, to_f32};

/// How many moments a run is played back in, at most, besides its start.
const MOMENTS: usize = 60;

/// How many columns a side the played-back material has, at most.
const COLUMNS: usize = 60;

/// How many cells a side the material is swept on, at most: finer than it is
/// drawn, so a pass narrower than a drawn column still shows.
const SWEPT: usize = 240;

/// `ran`'s setup as moments, `draws` being its operations as drawn; `None`
/// when it cut nothing or cannot be replayed.
pub(crate) fn playback(job: &Job, ran: &Ran, draws: &[Drawn]) -> Option<Playback> {
    let r = &ran.result;
    let setup = job.setups.first()?;
    let total: usize = draws.iter().map(|d| d.ends.len()).sum();
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
        .map(|p| to_f32(r.frame.to_model(p.extend(0.0))))
        .collect();
    let zero = r.frame.to_model(DVec3::ZERO);
    let axis = to_f32(r.frame.to_model(DVec3::Z) - zero);
    let mut indices = Vec::with_capacity((nx - 1) * (ny - 1) * 6);
    for j in 0..ny - 1 {
        for i in 0..nx - 1 {
            let a = (j * nx + i) as u32;
            let c = a + nx as u32;
            indices.extend([a, a + 1, c + 1, a, c + 1, c]);
        }
    }
    let low = r
        .stocks
        .iter()
        .map(|b| b.min.z)
        .fold(f64::INFINITY, f64::min);
    let high = r
        .stocks
        .iter()
        .map(|b| b.max.z)
        .fold(f64::NEG_INFINITY, f64::max);
    // A stock with no height, or none at all (the folds start infinite).
    if (high - low).partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return None;
    }
    let quantum = (high - low) / f64::from(u16::MAX);

    // The stock, swept on a coarse grid of its own.
    let boxes: Vec<(DVec2, DVec2, f64)> = r
        .stocks
        .iter()
        .map(|b| (b.min.truncate(), b.max.truncate(), b.max.z))
        .collect();
    let mut stock = Stock::from_boxes(&boxes, low, span.x.max(span.y) / SWEPT as f64).ok()?;
    let floor = stock.floor();

    // The moments, spread evenly over all the moves, each operation's last
    // among them so every operation ends on one.
    let mut moments = Vec::new();
    let mut done = 0;
    for (k, drawn) in draws.iter().enumerate() {
        let n = drawn.ends.len();
        let ran_op = r.operations.get(k)?;
        let sweep = if ran_op.made.has_paths() {
            let tool = cutting_tool(job, setup.operations.get(k)?).ok()?;
            Some((tool, sim_moves(&ran_op.path)))
        } else {
            None
        };
        let sim = sweep.as_ref().map_or(0, |(_, m)| m.len());
        let mut swept = 0;
        let mut at: Vec<usize> = (1..=MOMENTS)
            .map(|f| f * total / MOMENTS)
            .filter(|g| *g > done && *g < done + n)
            .map(|g| g - done)
            .collect();
        if n > 0 {
            at.push(n);
        } else if k == 0 {
            at.push(0);
        }
        for i in at {
            // The simulator's moves need not be the toolpath's one for one;
            // they run in step.
            let target = if n == 0 { 0 } else { (i * sim).div_ceil(n) };
            if let Some((tool, moves)) = &sweep
                && target > swept
            {
                // From the move before, which the sweep starts from.
                if let Some(slice) = moves.get(swept.saturating_sub(1)..target) {
                    stock.sweep(slice, tool);
                }
                swept = target;
            }
            let heights = grid
                .iter()
                .map(|p| {
                    let z = stock.height_at(p.x, p.y).unwrap_or(floor);
                    ((z - low) / quantum)
                        .round()
                        .clamp(0.0, f64::from(u16::MAX)) as u16
                })
                .collect();
            let tool = match i {
                0 => drawn.ends.first(),
                _ => drawn.ends.get(i - 1),
            }
            .map_or([0.0; 3], |p| to_f32(r.frame.to_model(*p)));
            moments.push(Moment {
                op: k,
                moves: i,
                of: n,
                tool,
                drawn: drawn.drawn_after(i),
                heights,
            });
        }
        done += n;
    }
    Some(Playback {
        base,
        axis,
        indices,
        low: low as f32,
        step: quantum as f32,
        moments,
    })
}
