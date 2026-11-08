//! A helper kept running to play a setup back at full resolution: it runs
//! the setup once, keeps it, and answers each seek with the material that
//! changed, swept again from the nearest checkpoint as camcraft's app
//! sweeps it, the tool's tip, and how far the toolpath has gone.
//!
//! The package opens it with [`SessionIn::Open`] and a setup, as it asks for
//! a run; the session answers [`SessionOut::Ready`] once it has run, then a
//! [`SessionOut::Stock`] for each [`SessionIn::Seek`]. One line each way.
//!
//! A setup of several parts plays back as its program cuts them: each part
//! has material of its own, cut as far as the program has cut that part by
//! then, so roughing the second part by tool leaves the first's roughed and
//! waiting for its finish. Between parts the tool goes from one to the next.

use std::io::{BufRead, Write};

use cam_protocol::{
    GridPatch, GridRound, SessionIn, SessionOut, SessionReady, StockPatch, base64, class, pack_bare,
};
use camcraft_core::replay::{Changed, Live, Position, Replay};
use camcraft_sim::stock::{Axis, Block};
use glam::{DVec2, DVec3};

use crate::{Executed, execute, to_f32};

/// The most points a side of the material grid has: a sheet's stock is
/// thousands of cells across, more than a view shows or a line should
/// carry.
const MOST_POINTS: u32 = 600;

/// What the checkpoints going back may hold, packed.
const CHECKPOINT_BYTES: usize = 256 << 20;

/// How many moves one step of a seek sweeps before it says how far it got.
const SEEK_BUDGET: usize = 1_000_000;

/// A run kept to play back.
struct Kept {
    done: Executed,
    replay: Replay,
    /// Each part's material, as far as it is cut.
    lives: Vec<Live>,
    /// Stock cells a grid point stands for, a side.
    every: u32,
    cols: u32,
    rows: u32,
    low: f64,
    quantum: f64,
    /// How far from the part the material counts as finished: the run's
    /// gouge budget.
    band: f64,
    /// Whether the package has had each part's whole grid: the first seek
    /// sends it all, whatever changed.
    sent: bool,
}

/// Serve a session that began with `first`, reading the rest of its lines
/// from `lines` and answering on `out`, until the package closes it.
pub fn serve(first: SessionIn, lines: impl BufRead, mut out: impl Write) -> std::io::Result<()> {
    let mut kept = None;
    let answer = |kept: &mut Option<Kept>, message: SessionIn| -> Option<SessionOut> {
        match message {
            SessionIn::Open(request) => Some(match open(&request) {
                Ok((k, ready)) => {
                    *kept = Some(k);
                    SessionOut::Ready(Box::new(ready))
                }
                Err(why) => SessionOut::Refused { why },
            }),
            SessionIn::Seek { seconds } => kept
                .as_mut()
                .map(|k| SessionOut::Stock(Box::new(k.seek(seconds)))),
        }
    };
    let send = |out: &mut dyn Write, said: Option<SessionOut>| -> std::io::Result<()> {
        if let Some(said) = said {
            let line = serde_json::to_string(&said).map_err(std::io::Error::other)?;
            writeln!(out, "{line}")?;
            out.flush()?;
        }
        Ok(())
    };
    let said = answer(&mut kept, first);
    send(&mut out, said)?;
    for line in lines.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let said = match serde_json::from_str::<SessionIn>(&line) {
            Ok(message) => answer(&mut kept, message),
            Err(e) => Some(SessionOut::Refused {
                why: format!("the package's line does not read: {e}"),
            }),
        };
        send(&mut out, said)?;
    }
    Ok(())
}

/// Run `request`'s setup and keep it; the grid and timing the package
/// draws it with.
fn open(request: &cam_protocol::Request) -> Result<(Kept, SessionReady), String> {
    let mut errors = std::io::stderr();
    let mut progress = |done: u64, total: u64| {
        let _ = writeln!(errors, "progress {done} {total}");
        let _ = errors.flush();
    };
    let done = execute(request, &mut progress, crate::cache::folder().as_deref())?;
    let setup = done.job.setups.first().ok_or("the job has no setup")?;
    let r = &done.ran.result;
    let replay = Replay::new(&done.job, setup, r).map_err(|e| e.to_string())?;
    // Each part's checkpoints share what one would hold.
    let parts = done.cut.parts.len().max(1);
    let lives = (0..parts)
        .map(|_| Live::new(&replay, CHECKPOINT_BYTES / parts))
        .collect::<Option<Vec<_>>>()
        .ok_or("the run kept no material")?;
    let field = lives[0].stock().field();
    let (nx, ny, pitch) = (field.nx(), field.ny(), field.pitch());
    // Several parts share the points one would have: the package takes
    // every part's grid in, and draws it, within one frame's budget.
    let most = (f64::from(MOST_POINTS) / (parts as f64).sqrt()).max(2.0) as u32;
    let every = nx.max(ny).div_ceil(most).max(1);
    let (cols, rows) = ((nx - 1) / every + 1, (ny - 1) / every + 1);
    let corner = field.origin().extend(0.0);
    let at = |p: DVec3| r.frame.shown(p);
    let origin = at(corner);
    let step = f64::from(every) * pitch;
    // On a rotary the grid stands on the axis, the way round it turned.
    let round = r.frame.wrap.map(|w| {
        let zero = r.frame.to_model(DVec3::ZERO);
        let turn = (step / w.radius) as f32;
        let (side, up) = if w.along == Axis::Y {
            (DVec3::X, DVec3::Z)
        } else {
            (DVec3::Y, DVec3::Z)
        };
        let first = w.angle(corner) as f32;
        let (per_col, per_row) = if w.along == Axis::Y {
            (turn, 0.0)
        } else {
            (0.0, turn)
        };
        (
            GridRound {
                side: to_f32(r.frame.to_model(side) - zero),
                first,
                per_col,
                per_row,
            },
            to_f32(r.frame.to_model(up) - zero),
        )
    });
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
    if (high - low).partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return Err("the stock has no height".into());
    }
    let quantum = (high - low) / f64::from(u16::MAX);
    let times = done
        .cut
        .sections
        .iter()
        .map(|s| {
            done.draws.get(s.op).map_or_else(Vec::new, |d| {
                d.times.iter().map(|t| (s.start + t) as f32).collect()
            })
        })
        .collect();
    // Each part's grid is the first's, moved as far as the part is.
    let zero = r.frame.to_model(DVec3::ZERO);
    let offsets = if parts > 1 {
        done.cut
            .parts
            .iter()
            .map(|a| to_f32(r.frame.to_model(*a) - zero))
            .collect()
    } else {
        Vec::new()
    };
    // Squares with no stock at the start, so round stock's box's corners
    // are not drawn.
    let squares = (0..rows.saturating_sub(1))
        .flat_map(|j| (0..cols.saturating_sub(1)).map(move |i| (i, j)))
        .map(|(i, j)| {
            let corner = field.origin() + DVec2::new(f64::from(i), f64::from(j)) * step;
            // The grid runs on past the box to whole cells, and the stock
            // with it, a cell past a round edge at most.
            !crate::stocked(&r.stocks, corner, DVec2::splat(step), pitch)
        });
    let bare: Vec<bool> = squares.collect();
    let bare = if bare.contains(&true) {
        base64(&pack_bare(bare))
    } else {
        String::new()
    };
    let ready = SessionReady {
        cols,
        rows,
        origin: to_f32(origin),
        across: to_f32(at(corner + DVec3::X * step) - origin),
        along: to_f32(at(corner + DVec3::Y * step) - origin),
        up: round.map_or_else(|| to_f32(at(corner + DVec3::Z) - origin), |(_, up)| up),
        round: round.map(|(g, _)| g),
        low: low as f32,
        step: quantum as f32,
        times,
        parts: offsets,
        timing: done.run.timing,
        bare,
    };
    let band = r.verify_budget.max(1e-3);
    Ok((
        Kept {
            done,
            replay,
            lives,
            every,
            cols,
            rows,
            low,
            quantum,
            band,
            sent: false,
        },
        ready,
    ))
}

impl Kept {
    /// The section under way at `seconds`, the moves made and fraction of
    /// the next, and where the tool is if it is going there from another
    /// part (then none made).
    fn place(&self, seconds: f64) -> (usize, usize, f64, Option<DVec3>) {
        let cut = &self.done.cut;
        let k = cut.section_at(seconds);
        if let Some(tip) = cut.between(k + 1, seconds) {
            return (k + 1, 0, 0.0, Some(tip));
        }
        let Some(section) = cut.sections.get(k) else {
            return (0, 0, 0.0, None);
        };
        let Some(drawn) = self.done.draws.get(section.op) else {
            return (k, 0, 0.0, None);
        };
        let local = seconds - section.start;
        let times = &drawn.times;
        let made = times.partition_point(|t| *t <= local);
        if made >= times.len() {
            return (k, times.len(), 0.0, None);
        }
        let before = made.checked_sub(1).map_or(0.0, |i| times[i]);
        let span = times[made] - before;
        let fraction = if span > 0.0 {
            ((local - before) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (k, made, fraction, None)
    }

    /// Stand at `seconds`: the material that changed at each part since
    /// the last seek, the tool, and how far the toolpath has gone.
    fn seek(&mut self, seconds: f64) -> StockPatch {
        let (k, made, fraction, between) = self.place(seconds);
        let cut = &self.done.cut;
        let section = cut.sections.get(k).copied().unwrap_or_default();
        let op = section.op;
        // Each part as far as the program has cut it: the section under
        // way at its own part, each other where its last section left it.
        // A section done is its operation done, however the simulator
        // counts its moves.
        let done = usize::MAX;
        let of = self.done.draws.get(op).map_or(0, |d| d.ends.len());
        let targets: Vec<(Position, f64)> = cut
            .standing(k)
            .into_iter()
            .map(|last| match last {
                Some(j) if j == k && made < of => (Position { op, moves: made }, fraction),
                Some(j) => {
                    let op = cut.sections[j].op;
                    (Position { op, moves: done }, 0.0)
                }
                None => (Position { op: 0, moves: 0 }, 0.0),
            })
            .collect();
        let first = !std::mem::replace(&mut self.sent, true);
        let mut changes = Vec::new();
        for (part, (target, fraction)) in targets.into_iter().enumerate() {
            let Some(live) = self.lives.get_mut(part) else {
                continue;
            };
            let mut changed = Changed::Nothing;
            loop {
                let (step, reached) = live.seek(&self.replay, target, fraction, SEEK_BUDGET);
                changed = changed.and(step);
                if reached {
                    break;
                }
            }
            if first {
                changed = Changed::All;
            }
            changes.push((part, changed));
        }
        let drawn = &self.done.draws[op];
        let end = |i: usize| drawn.ends.get(i).copied();
        let tip = between
            .or_else(|| {
                match made {
                    0 => end(0),
                    m => match (end(m - 1), end(m)) {
                        (Some(a), Some(b)) => Some(a.lerp(b, fraction)),
                        (a, _) => a,
                    },
                }
                .map(|p| p + cut.offset(k))
            })
            .unwrap_or(DVec3::ZERO);
        let mut patch = StockPatch {
            seconds,
            section: k,
            op,
            part: section.part,
            moves: made,
            of: drawn.ends.len(),
            tool: to_f32(self.done.ran.result.frame.shown(tip)),
            drawn: drawn.drawn_after(made),
            ..StockPatch::default()
        };
        for (part, changed) in changes {
            let rect = match changed {
                Changed::Nothing => continue,
                Changed::All => (0, 0, self.cols - 1, self.rows - 1),
                Changed::Cells(c) => {
                    let (d, lc, lr) = (self.every, self.cols - 1, self.rows - 1);
                    (
                        (c.x0 / d).min(lc),
                        (c.y0 / d).min(lr),
                        c.x1.div_ceil(d).min(lc),
                        c.y1.div_ceil(d).min(lr),
                    )
                }
            };
            let (c0, r0, c1, r1) = rect;
            let (heights, classes) = self.points(part, c0, r0, c1, r1);
            patch.changed.push(GridPatch {
                part,
                col: c0,
                row: r0,
                width: c1 - c0 + 1,
                height: r1 - r0 + 1,
                heights: base64(&heights),
                classes: base64(&classes),
            });
        }
        patch
    }

    /// Part `part`'s grid's points from `(c0, r0)` to `(c1, r1)`, row by
    /// row: their heights packed, and how each stands against the part.
    fn points(&self, part: usize, c0: u32, r0: u32, c1: u32, r1: u32) -> (Vec<u8>, Vec<u8>) {
        let stock = self.lives[part].stock();
        let field = stock.field();
        let floor = stock.floor();
        let part = &self.done.ran.result.part;
        let n = ((c1 - c0 + 1) * (r1 - r0 + 1)) as usize;
        let (mut heights, mut classes) = (Vec::with_capacity(n * 2), Vec::with_capacity(n));
        for r in r0..=r1 {
            for c in c0..=c1 {
                let (ix, iy) = (
                    (c * self.every).min(field.nx() - 1),
                    (r * self.every).min(field.ny() - 1),
                );
                let z = field.get(ix, iy).unwrap_or(floor).max(floor);
                let packed = ((z - self.low) / self.quantum)
                    .round()
                    .clamp(0.0, f64::from(u16::MAX)) as u16;
                heights.extend_from_slice(&packed.to_le_bytes());
                let at = field.position(ix, iy);
                classes.push(match at.and_then(|p| part.height_at(p.x, p.y)) {
                    None => class::CLEAR,
                    Some(p) if z < p - self.band => class::GOUGE,
                    Some(p) if z <= p + self.band => class::FINISHED,
                    Some(_) => class::STOCK,
                });
            }
        }
        (heights, classes)
    }
}
