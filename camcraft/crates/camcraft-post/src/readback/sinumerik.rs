//! Reading a Sinumerik's own words back ([`crate::sinumerik`]): its work
//! offsets past `G57`, the tool's offsets taken up by `D1` and let go by
//! `D0`, and its drilling cycles, called once or after each move
//! (`MCALL`), into the moves their manual gives them.

use camcraft_core::job::SpindleDirection;

use super::{Action, MM_PER_INCH, Reader};
use crate::sinumerik::{Call, Line};

/// Whether `v` is `n`, as a whole number written is.
fn is(v: f64, n: f64) -> bool {
    (v - n).abs() < 1e-9
}

/// The most parameters each cycle understood here takes.
fn most(number: u32) -> Option<usize> {
    match number {
        81 | 82 => Some(6),
        83 => Some(17),
        84 => Some(14),
        840 => Some(13),
        _ => None,
    }
}

impl Reader {
    /// One line of a Sinumerik program, read as [`Reader::block`] reads
    /// G-code's, with what only a Sinumerik writes.
    pub(super) fn siemens_block(&mut self, l: &Line) -> Result<(), String> {
        let has_axes = l.words.iter().any(|(c, _)| "XYZABC".contains(*c));
        let machine = l.words.iter().any(|&(c, v)| c == 'G' && is(v, 53.0));
        if l.edge.is_some() && has_axes && !machine {
            return Err("D in a block that moves in the work is not understood".into());
        }
        if l.call.is_some() && (has_axes || l.mcall.is_some()) {
            return Err("a cycle called in a block that moves, or with MCALL".into());
        }
        let calling = self.siemens.as_ref().and_then(|s| s.mcall.clone());
        match &l.mcall {
            Some(None) => self.siemens_mut().mcall = None,
            Some(Some(c)) => {
                if calling.is_some() {
                    return Err(format!(
                        "MCALL {c} while another cycle is called: end it with MCALL first"
                    ));
                }
                if most(c.number).is_none() {
                    return Err(format!("{c} is not understood"));
                }
                self.siemens_mut().mcall = Some(c.clone());
            }
            None => {}
        }
        if let Some(g) = l.offset {
            let word = format!("G{g}");
            let n = self
                .siemens
                .as_ref()
                .and_then(|s| s.offsets.iter().find(|(_, w)| *w == word))
                .map(|(n, _)| *n)
                .ok_or_else(|| format!("{word} is not one of this machine's work offsets"))?;
            self.select_offset(n);
        }
        let before = self.steps.len();
        self.block(&l.words)?;
        let new = self.steps.get(before..).unwrap_or_default();
        let moved = new
            .iter()
            .any(|s| matches!(&s.action, Action::Move(m) if !m.probe));
        let in_machine = new
            .iter()
            .any(|s| matches!(s.action, Action::Machine(_) | Action::Home(_)));
        for i in before..self.steps.len() {
            match self.steps.get(i).map(|s| s.action.clone()) {
                Some(Action::Change(t)) => {
                    let s = self.siemens_mut();
                    s.tool = Some(t);
                    s.changes = true;
                    s.edge = None;
                }
                Some(Action::Pause) => {
                    let selected = self.selected;
                    let s = self.siemens_mut();
                    if !s.changes && selected.is_some() {
                        s.tool = selected;
                        s.edge = None;
                    }
                }
                _ => {}
            }
        }
        if moved {
            self.offsets_taken_up()?;
        }
        if let Some(d) = l.edge {
            match d {
                0 => {
                    self.push(Action::LengthOffset(None));
                    self.siemens_mut().edge = Some(false);
                }
                1 => {
                    let t = self
                        .siemens
                        .as_ref()
                        .and_then(|s| s.tool)
                        .ok_or("D1 with no tool in")?;
                    self.push(Action::LengthOffset(Some(t)));
                    // The same machine position is now at another Z.
                    self.at[2] = None;
                    self.siemens_mut().edge = Some(true);
                }
                n => {
                    return Err(format!(
                        "D{n}: a tool's edges past its first are not understood"
                    ));
                }
            }
        }
        if let Some(c) = &l.call {
            self.run_cycle(c)?;
        }
        if let Some(c) = calling.filter(|_| l.mcall.is_none()) {
            if in_machine {
                return Err(format!(
                    "a move in machine coordinates while {c} is called after each (MCALL)"
                ));
            }
            if moved {
                self.run_cycle(&c)?;
            } else if l.words.iter().any(|(w, _)| matches!(w, 'F' | 'S'))
                && matches!(self.motion, Some(0 | 1))
            {
                return Err(format!(
                    "a block of F or S alone calls {c} (MCALL): not understood"
                ));
            }
        }
        Ok(())
    }

    fn siemens_mut(&mut self) -> &mut super::Siemens {
        self.siemens.get_or_insert_with(Default::default)
    }

    /// A move in the work's coordinates needs the tool's offsets taken up.
    fn offsets_taken_up(&self) -> Result<(), String> {
        match self.siemens.as_ref().and_then(|s| s.edge) {
            Some(true) => Ok(()),
            Some(false) => Err("moves in the work with the tool's offsets let go (D0)".into()),
            None => Err("moves in the work before the tool's offsets are taken up (D1)".into()),
        }
    }

    /// The cycle `c`, run where the tool is: the moves its manual gives it.
    /// Each starts at its reference plane and safety clearance (RFP +
    /// SDIS), at rapid, and ends at its retract plane (RTP), at rapid.
    fn run_cycle(&mut self, c: &Call) -> Result<(), String> {
        self.offsets_taken_up()?;
        let max = most(c.number).ok_or_else(|| format!("{c} is not understood"))?;
        if c.args.len() > max {
            return Err(format!("{c} with more parameters than are understood"));
        }
        if self.absolute != Some(true) {
            return Err("cycles in incremental mode are not understood".into());
        }
        if self.plane != 17 {
            return Err("cycles out of the XY plane are not understood".into());
        }
        let scale = self.scale()?;
        let (x, y) = (self.at[0], self.at[1]);
        if x.is_none() || y.is_none() {
            return Err(format!("{c} at a place not known"));
        }
        self.at[2].ok_or_else(|| format!("{c} from a height not known"))?;
        let need = |i: usize, what: &str| c.arg(i).ok_or_else(|| format!("{c} with no {what}"));
        let none_or = |i: usize, ok: &[f64], what: &str| match c.arg(i) {
            Some(v) if !ok.iter().any(|o| is(v, *o)) => {
                Err(format!("{c} with {what} {v} is not understood"))
            }
            _ => Ok(()),
        };
        let rtp = need(0, "RTP")? * scale;
        let rfp = need(1, "RFP")? * scale;
        let sdis = c.arg(2).unwrap_or(0.0).abs() * scale;
        let dp = match c.arg(4) {
            Some(dpr) if !is(dpr, 0.0) => rfp - dpr.abs() * scale,
            _ => need(3, "depth (DP)")? * scale,
        };
        if dp >= rfp {
            return Err(format!("{c}'s depth is not below its reference plane"));
        }
        let r = rfp + sdis;
        if rtp < r - 1e-9 {
            return Err(format!(
                "{c} returns to RTP below its reference plane and safety clearance"
            ));
        }
        let at = |z: f64| [x, y, Some(z)];
        self.go(at(r), None, None);
        match c.number {
            81 | 82 => {
                let feed = self.feed.ok_or_else(|| format!("{c} with no feed"))?;
                self.go(at(dp), Some(feed), None);
                self.pause_of(c, c.arg(5))?;
            }
            83 => {
                let feed = self.feed.ok_or_else(|| format!("{c} with no feed"))?;
                let first = match (c.arg(6), c.arg(5)) {
                    (Some(fdpr), _) if !is(fdpr, 0.0) => rfp - fdpr.abs() * scale,
                    (_, Some(fdep)) => fdep * scale,
                    _ => return Err(format!("{c} with no first depth (FDEP or FDPR)")),
                };
                none_or(7, &[0.0], "a degression (DAM)")?;
                none_or(8, &[0.0], "a pause at each depth (DTB)")?;
                none_or(9, &[0.0], "a pause at the start (DTS)")?;
                none_or(10, &[0.0, 1.0], "a first depth's feed factor (FRF)")?;
                none_or(11, &[0.0, 1.0], "VARI")?;
                none_or(12, &[0.0, 3.0], "a drilling axis (_AXN)")?;
                none_or(15, &[0.0], "a pause at the bottom (_DTD)")?;
                let removes = c.arg(11).is_some_and(|v| is(v, 1.0));
                // Backed off to break the chip: 1 mm unless said.
                let back = match c.arg(14) {
                    Some(v) if v > 0.0 => v * scale,
                    _ => 1.0,
                };
                let stroke = rfp - first;
                if stroke <= 0.0 || first < dp - 1e-9 {
                    return Err(format!(
                        "{c}'s first depth is not between its reference plane and its depth"
                    ));
                }
                let mut next = first;
                loop {
                    self.go(at(next), Some(feed), None);
                    let drilled = next;
                    if drilled <= dp {
                        break;
                    }
                    if removes {
                        // Out to clear the chips, and back down as far as
                        // drilled, the deepest it may go.
                        self.go(at(r), None, None);
                        self.go(at(drilled), None, None);
                    } else {
                        self.go(at((drilled + back).min(r)), Some(feed), None);
                    }
                    next = (drilled - stroke).max(dp);
                }
            }
            84 | 840 => self.tap(c, r, dp)?,
            _ => {}
        }
        self.go(at(rtp), None, None);
        // Its own motion is no move's: the next says its own.
        self.motion = None;
        Ok(())
    }

    /// A pause at the bottom, `DTB` in seconds, where it is given.
    fn pause_of(&mut self, c: &Call, dtb: Option<f64>) -> Result<(), String> {
        match dtb {
            Some(s) if s < 0.0 => Err(format!("{c}: a pause in turns is not understood")),
            Some(s) if s > 0.0 => {
                self.push(Action::Dwell(s));
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// `CYCLE84`, tapping rigidly at `SST`, the thread's hand the sign of
    /// its pitch `PIT`, and back out at `SST1`; or `CYCLE840`, with a
    /// floating holder, at the pitch times the spindle's speed both ways.
    /// The spindle must be turning the thread's way at the speed tapped
    /// at, and be left so (`SDAC`).
    fn tap(&mut self, c: &Call, r: f64, dp: f64) -> Result<(), String> {
        let floating = c.number == 840;
        let (x, y) = (self.at[0], self.at[1]);
        let at = |z: f64| [x, y, Some(z)];
        let need = |i: usize, what: &str| c.arg(i).ok_or_else(|| format!("{c} with no {what}"));
        let (sdac_at, mpit_at, pit_at, pita_at, axn_at) = if floating {
            (7, 9, 10, 12, 11)
        } else {
            (6, 7, 8, 13, 12)
        };
        if c.arg(mpit_at).is_some_and(|v| !is(v, 0.0)) {
            return Err(format!(
                "{c} with a thread's size (MPIT) is not understood: its pitch, PIT"
            ));
        }
        if c.arg(axn_at).is_some_and(|v| !is(v, 0.0) && !is(v, 3.0)) {
            return Err(format!(
                "{c} along other than the third axis is not understood"
            ));
        }
        let pit = need(pit_at, "pitch (PIT)")?;
        let per = match c.arg(pita_at) {
            Some(v) if is(v, 1.0) => 1.0,
            Some(v) if is(v, 3.0) => MM_PER_INCH,
            _ => {
                return Err(format!(
                    "{c} with its pitch's unit (_PITA) other than 1, mm, or 3, inches a turn, \
                     is not understood"
                ));
            }
        };
        let pitch = pit.abs() * per;
        if pitch == 0.0 {
            return Err(format!("{c} with no pitch"));
        }
        let spin = self
            .spin
            .ok_or_else(|| format!("{c} without the spindle turning"))?;
        let turning = self.spun_at.ok_or_else(|| format!("{c} with no speed"))?;
        let (word, m) = match spin {
            SpindleDirection::Clockwise => (3.0, "M3"),
            SpindleDirection::CounterClockwise => (4.0, "M4"),
        };
        if !c.arg(sdac_at).is_some_and(|v| is(v, word)) {
            return Err(format!(
                "{c} leaves the spindle other than turning as it did ({m}): SDAC"
            ));
        }
        let (down, up) = if floating {
            if pit < 0.0 {
                return Err(format!("{c} with a negative pitch is not understood"));
            }
            if !c.arg(8).is_some_and(|v| is(v, 11.0)) {
                return Err(format!(
                    "{c} other than without an encoder, its feed from its pitch (ENC 11), \
                     is not understood"
                ));
            }
            let back = 7.0 - word;
            if c.arg(6).is_some_and(|v| !is(v, 0.0) && !is(v, back)) {
                return Err(format!("{c} backing out the way it went in (SDR)"));
            }
            (pitch * turning, pitch * turning)
        } else {
            let want = if pit > 0.0 {
                SpindleDirection::Clockwise
            } else {
                SpindleDirection::CounterClockwise
            };
            if spin != want {
                return Err(format!(
                    "{c} tapping {} without the spindle turning {}",
                    if pit > 0.0 { "right-hand" } else { "left-hand" },
                    if pit > 0.0 { "M3" } else { "M4" }
                ));
            }
            let sst = need(10, "speed (SST)")?;
            if (sst - turning).abs() > 0.5 {
                return Err(format!(
                    "{c} taps at {sst} rev/min where the spindle turns at {turning}"
                ));
            }
            let sst1 = c.arg(11).filter(|v| *v > 0.0).unwrap_or(sst);
            (pitch * sst, pitch * sst1)
        };
        self.go(at(dp), Some(down), None);
        self.pause_of(c, c.arg(5))?;
        self.go(at(r), Some(up), None);
        Ok(())
    }
}
