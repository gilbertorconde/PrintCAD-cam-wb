//! What the G-code renderer writes otherwise for a Sinumerik
//! ([`crate::sinumerik`]): its program's first lines, its tool words, `D1`
//! before the tool moves in the work, and its drilling cycles.

use camcraft_core::strategy::drilling::RETRACT_GAP;
use glam::{DVec2, dvec3};

use super::Writer;
use crate::op::CycleKind;
use crate::profile::{ArcStyle, ProgramUnits, TapStyle};

/// How far `CYCLE83` backs off to break the chip (`_VRT`), in millimetres.
/// The path does not say: `G73` backs off as far as the controller is set
/// to, and this is as far as the reading back of `G73` takes it to.
const CHIP_BREAK: f64 = 0.25;

/// One hole of a run of cycles, as [`crate::op::Op::Cycle`] gives it.
pub(super) struct Hole {
    pub kind: CycleKind,
    pub at: DVec2,
    pub depth: f64,
    pub retract: f64,
    pub peck: Option<f64>,
    pub dwell: Option<f64>,
    pub feed: f64,
}

impl Writer<'_> {
    /// The program's name and where it goes, `%_N_<name>_MPF` and
    /// `;$PATH=/_N_MPF_DIR`, as a Sinumerik reads a program sent to it as
    /// text: never numbered.
    pub(super) fn program_header(&mut self, name: &str) {
        let name = crate::sinumerik::program_name(name);
        self.out
            .push_str(&format!("%_N_{name}_MPF\n;$PATH=/_N_MPF_DIR\n"));
        self.report.lines += 2;
    }

    /// The word that calls up `tool`: `T3`, or with a Sinumerik's tool
    /// management `T="3"`, by its name.
    pub(super) fn tool_word(&self, tool: u32) -> String {
        if self.siemens && self.p.tool_management {
            format!("T=\"{tool}\"")
        } else {
            format!("T{tool}")
        }
    }

    /// How arcs are written: by their centre on a Sinumerik, whose radius
    /// word is not `R`; else as the profile says.
    pub(super) fn arc_style(&self) -> ArcStyle {
        if self.siemens {
            ArcStyle::Ij
        } else {
            self.p.arc_style
        }
    }

    /// On a Sinumerik whose offsets were let go (`D0`), `D1` before the
    /// tool moves in the work again: in the work's coordinates the tool's
    /// tip is where its length puts it.
    pub(super) fn offsets(&mut self) {
        if self.siemens && !self.edge {
            self.raw("D1");
            self.edge = true;
        }
    }

    /// One hole: its cycle called after each move from here, `MCALL
    /// CYCLE8x(..)`, where it is not the one called already, at the
    /// drilling feed; then over the hole, which calls it. Both words, so
    /// the block moves even where the tool is already over it.
    pub(super) fn siemens_cycle(&mut self, h: &Hole) {
        self.offsets();
        let inches = self.inches;
        let len = |v: f64| self.p.length(v, inches);
        let top = h.retract - RETRACT_GAP;
        let rtp = self.clearance.unwrap_or(h.retract).max(h.retract);
        // RTP, RFP, SDIS, DP: where it returns to, the hole's top, the gap
        // above it fed from, and its bottom.
        let head = format!(
            "{},{},{},{}",
            len(rtp),
            len(top),
            len(RETRACT_GAP),
            len(h.depth)
        );
        let first = |peck: Option<f64>| len((top - peck.unwrap_or(top - h.depth)).max(h.depth));
        let call = match h.kind {
            CycleKind::Drill => format!("CYCLE81({head})"),
            // DTB, after DPR, in seconds.
            CycleKind::DrillDwell => format!(
                "CYCLE82({head},,{})",
                self.p.seconds(h.dwell.unwrap_or(0.0))
            ),
            // FDEP, the first peck's bottom; no degression (DAM), no
            // pauses (DTB, DTS), the first peck at the feed (FRF 1); out
            // of the hole between pecks (VARI 1).
            CycleKind::Peck => format!("CYCLE83({head},,{},,0,0,0,1,1)", first(h.peck)),
            // Backing off _VRT to break the chip (VARI 0).
            CycleKind::ChipBreak => format!(
                "CYCLE83({head},,{},,0,0,0,1,0,,,{})",
                first(h.peck),
                len(CHIP_BREAK)
            ),
            CycleKind::Tap | CycleKind::TapLeft => self.tap_call(h, &head),
        };
        let key = format!("{call} {}", h.feed);
        if self.mcall.as_deref() != Some(key.as_str()) {
            if self.mcall.take().is_some() {
                self.raw("MCALL");
            }
            // The drilling cycles feed at the F in force; a tap's own.
            if !matches!(h.kind, CycleKind::Tap | CycleKind::TapLeft) {
                let f = self.p.feed_in(h.feed, inches);
                if self.feed.as_deref() != Some(f.as_str()) || !self.p.modal.suppress_feed {
                    self.raw(&format!("F{f}"));
                    self.feed = Some(f);
                }
            }
            self.raw(&format!("MCALL {call}"));
            self.mcall = Some(key);
        }
        let s = format!("G0 X{} Y{}", len(h.at.x), len(h.at.y));
        self.raw(&s);
        self.report.motion_blocks += 1;
        // Back at RTP over the hole; G0 said again by the next block.
        self.at = Some(dvec3(h.at.x, h.at.y, rtp));
        self.motion = None;
        self.raised = false;
    }

    /// `CYCLE84`, tapping rigidly at the spindle's speed, the thread's hand
    /// its pitch's sign, the spindle turning as before after it; or with a
    /// floating holder, `CYCLE840`, without an encoder (`ENC` 11), the feed
    /// worked out from the pitch and the speed, the spindle reversed at the
    /// bottom by itself (`SDR` 0).
    fn tap_call(&self, h: &Hole, head: &str) -> String {
        let inches = self.inches;
        let rpm = self.values.rpm.max(1);
        let pitch = h.feed / f64::from(rpm);
        let (sdac, sign) = if h.kind == CycleKind::Tap {
            (3, "")
        } else {
            (4, "-")
        };
        let pit = self.p.pitch(pitch, inches);
        // The pitch's unit: millimetres, or inches a turn.
        let pita = if self.values.units == ProgramUnits::Inch {
            3
        } else {
            1
        };
        if self.p.tapping() == TapStyle::Floating {
            format!("CYCLE840({head},,0,0,{sdac},11,,{pit},,{pita})")
        } else {
            format!("CYCLE84({head},,0,{sdac},,{sign}{pit},0,{rpm},0,,{pita})")
        }
    }

    /// The end of a run of holes: the cycle no longer called (`MCALL`).
    pub(super) fn siemens_cycle_end(&mut self) {
        if self.mcall.take().is_some() {
            self.raw("MCALL");
        }
        self.clearance = None;
        self.motion = None;
    }
}
