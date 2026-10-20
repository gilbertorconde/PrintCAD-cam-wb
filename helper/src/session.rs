//! A helper kept running to play a setup back at full resolution: it runs
//! the setup once, keeps it, and answers each seek with the material that
//! changed, swept again from the nearest checkpoint as camcraft's app
//! sweeps it, the tool's tip, and how far the toolpath has gone.
//!
//! The package opens it with [`SessionIn::Open`] and a setup, as it asks for
//! a run; the session answers [`SessionOut::Ready`] once it has run, then a
//! [`SessionOut::Stock`] for each [`SessionIn::Seek`]. One line each way.

use std::io::{BufRead, Write};

use cam_protocol::{SessionIn, SessionOut, SessionReady, StockPatch, base64, class};
use camcraft_core::replay::{Changed, Live, Position, Replay};
use glam::DVec3;

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
    live: Live,
    /// Stock cells a grid point stands for, a side.
    every: u32,
    cols: u32,
    rows: u32,
    low: f64,
    quantum: f64,
    /// When each operation starts, in machining seconds.
    starts: Vec<f64>,
    /// How far from the part the material counts as finished: the run's
    /// gouge budget.
    band: f64,
    /// Whether the package has had the whole grid: the first seek sends it
    /// all, whatever changed.
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
    let done = execute(request, &mut progress)?;
    let setup = done.job.setups.first().ok_or("the job has no setup")?;
    let r = &done.ran.result;
    let replay = Replay::new(&done.job, setup, r).map_err(|e| e.to_string())?;
    let live = Live::new(&replay, CHECKPOINT_BYTES).ok_or("the run kept no material")?;
    let field = live.stock().field();
    let (nx, ny, pitch) = (field.nx(), field.ny(), field.pitch());
    let every = nx.max(ny).div_ceil(MOST_POINTS).max(1);
    let (cols, rows) = ((nx - 1) / every + 1, (ny - 1) / every + 1);
    let corner = field.origin().extend(0.0);
    let at = |p: DVec3| r.frame.to_model(p);
    let origin = at(corner);
    let step = f64::from(every) * pitch;
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
    if (high - low).partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return Err("the stock has no height".into());
    }
    let quantum = (high - low) / f64::from(u16::MAX);
    let mut starts = Vec::new();
    let mut elapsed = 0.0;
    let times = done
        .draws
        .iter()
        .map(|d| {
            starts.push(elapsed);
            let t: Vec<f32> = d.times.iter().map(|s| (elapsed + s) as f32).collect();
            elapsed += d.times.last().copied().unwrap_or(0.0);
            t
        })
        .collect();
    let ready = SessionReady {
        cols,
        rows,
        origin: to_f32(origin),
        across: to_f32(at(corner + DVec3::X * step) - origin),
        along: to_f32(at(corner + DVec3::Y * step) - origin),
        up: to_f32(at(corner + DVec3::Z) - origin),
        low: low as f32,
        step: quantum as f32,
        times,
    };
    let band = r.verify_budget.max(1e-3);
    Ok((
        Kept {
            done,
            replay,
            live,
            every,
            cols,
            rows,
            low,
            quantum,
            starts,
            band,
            sent: false,
        },
        ready,
    ))
}

impl Kept {
    /// The operation, moves made and fraction of the next, at `seconds`.
    fn place(&self, seconds: f64) -> (usize, usize, f64) {
        let k = self.starts.iter().rposition(|s| *s <= seconds).unwrap_or(0);
        let Some(drawn) = self.done.draws.get(k) else {
            return (0, 0, 0.0);
        };
        let local = seconds - self.starts[k];
        let times = &drawn.times;
        let made = times.partition_point(|t| *t <= local);
        if made >= times.len() {
            return (k, times.len(), 0.0);
        }
        let before = made.checked_sub(1).map_or(0.0, |i| times[i]);
        let span = times[made] - before;
        let fraction = if span > 0.0 {
            ((local - before) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (k, made, fraction)
    }

    /// Stand at `seconds`: the material that changed since the last seek,
    /// the tool, and how far the toolpath has gone.
    fn seek(&mut self, seconds: f64) -> StockPatch {
        let (op, made, fraction) = self.place(seconds);
        let target = Position { op, moves: made };
        let mut changed = Changed::Nothing;
        loop {
            let (step, reached) = self.live.seek(&self.replay, target, fraction, SEEK_BUDGET);
            changed = changed.and(step);
            if reached {
                break;
            }
        }
        let drawn = &self.done.draws[op];
        let end = |i: usize| drawn.ends.get(i).copied();
        let tip = match made {
            0 => end(0),
            m => match (end(m - 1), end(m)) {
                (Some(a), Some(b)) => Some(a.lerp(b, fraction)),
                (a, _) => a,
            },
        }
        .unwrap_or(DVec3::ZERO);
        let mut patch = StockPatch {
            seconds,
            op,
            moves: made,
            of: drawn.ends.len(),
            tool: to_f32(self.done.ran.result.frame.to_model(tip)),
            drawn: drawn.drawn_after(made),
            ..StockPatch::default()
        };
        if !std::mem::replace(&mut self.sent, true) {
            changed = Changed::All;
        }
        let rect = match changed {
            Changed::Nothing => None,
            Changed::All => Some((0, 0, self.cols - 1, self.rows - 1)),
            Changed::Cells(c) => {
                let (d, lc, lr) = (self.every, self.cols - 1, self.rows - 1);
                Some((
                    (c.x0 / d).min(lc),
                    (c.y0 / d).min(lr),
                    c.x1.div_ceil(d).min(lc),
                    c.y1.div_ceil(d).min(lr),
                ))
            }
        };
        if let Some((c0, r0, c1, r1)) = rect {
            let (heights, classes) = self.points(c0, r0, c1, r1);
            patch.col = c0;
            patch.row = r0;
            patch.width = c1 - c0 + 1;
            patch.height = r1 - r0 + 1;
            patch.heights = base64(&heights);
            patch.classes = base64(&classes);
        }
        patch
    }

    /// The grid's points from `(c0, r0)` to `(c1, r1)`, row by row: their
    /// heights packed, and how each stands against the part.
    fn points(&self, c0: u32, r0: u32, c1: u32, r1: u32) -> (Vec<u8>, Vec<u8>) {
        let stock = self.live.stock();
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
