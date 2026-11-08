//! Heidenhain's conversational language, Klartext, as the TNC controls read
//! it: written from an instruction stream, and read back into the steps a
//! G-code program's are, so the same checks run on it.
//!
//! A profile writes it with `dialect = "heidenhain"`. Its program:
//!
//! ```text
//! 0 BEGIN PGM TOP-01-ROUGH MM
//! 1 BLK FORM 0.1 Z X-40 Y-30 Z-20
//! 2 BLK FORM 0.2 X+40 Y+30 Z+0
//! 3 ;rough - adaptive - T3 G54
//! 4 M5
//! 5 L Z+0 R0 FMAX M91
//! 6 TOOL CALL 3 Z S18000
//! 7 CYCL DEF 247 DATUM SETTING ~
//!   Q339=+1 ;DATUM NUMBER
//! 8 M3
//! 9 L X+10 Y+5 R0 FMAX
//! 10 L Z+2 R0 FMAX
//! 11 L Z-1 R0 F300
//! 12 CC X+15 Y+5
//! 13 C X+20 Y+5 DR- F1200
//! 14 CP IPA+360 IZ-1 DR+ F400
//! ...
//! 30 M30
//! 31 END PGM TOP-01-ROUGH MM
//! ```
//!
//! Every block is numbered from 0, comments too (`;` to the line's end).
//! `BEGIN PGM` gives the units, so there is no `G20`: lengths are in
//! millimetres or inches, every coordinate signed, and feeds in mm/min or,
//! in an inch program, tenths of an inch a minute, as Heidenhain's manuals
//! say. `TOOL CALL` changes the tool and takes up its length and radius
//! from the tool table, with the spindle's speed; `M3` starts it. `FMAX` is
//! a rapid, for its block only. An arc is its centre (`CC`, which holds)
//! and its end (`C`, `DR+` counter-clockwise about the plane's normal, as
//! `G3`), in any of the three planes the centre's axes name; a helix about
//! Z is `CP` round the centre by an angle (`IPA`) and down (`IZ`), and one
//! about X or Y, which `CP` is not documented for, is written as straight
//! moves along it. The work offset is a preset, `CYCL DEF 247 DATUM
//! SETTING`, `G54` its first row (`Q339=1`) up to `G59`, its sixth.
//!
//! Holes are `CYCL DEF 200 DRILLING` (with a peck, `Q202`, and a pause at
//! the bottom, `Q211`), `203 UNIVERSAL DRILLING` to break the chip and `207
//! RIGID TAPPING NEW` (its pitch's sign the thread's hand), each called at
//! a hole by `L X.. Y.. R0 FMAX M99`. The surface (`Q203`) is the hole's
//! top, `Q200` above it the R plane, and the tool returns to the clearance
//! height (`Q204` above the top) between holes, as `G98` does.

use std::f64::consts::{PI, TAU};
use std::fmt::Write as _;

use camcraft_core::job::{Coolant, SpindleDirection};
use camcraft_core::path::toolpath::Arc;
use camcraft_core::strategy::drilling::RETRACT_GAP;
use glam::{DVec2, DVec3, dvec3};

use crate::custom::Concern;
use crate::op::{CycleKind, Op};
use crate::placeholder::{Values, fill};
use crate::profile::{Profile, ProgramUnits, ToolChangeStyle};
use crate::readback::{Action, Motion, Position, Problem, Step, Turn, flat, known, turned};
use crate::render::{
    RenderReport, SHORTEST_CHORD, STRAIGHTENED, Title, custom_begin, custom_end, values,
};

/// Millimetres in a tenth of an inch: a feed's unit in an inch program.
const MM_PER_TENTH: f64 = 2.54;

/// Millimetres in an inch.
const MM_PER_INCH: f64 = 25.4;

/// How many chip breaks cycle 203 makes before it takes the drill out of the
/// hole (`Q213`): as many as it takes, the most it takes, so it breaks the
/// chip all the way down, as `G73` does.
const BREAKS: u32 = 99;

/// How far cycle 203 backs off to break the chip (`Q256`), in millimetres.
/// The path does not say: `G73` backs off as far as the controller is set to,
/// and this is as far as the reading back of `G73` takes it to.
const CHIP_BREAK: f64 = 0.25;

/// A program's name as `BEGIN PGM` can hold it: letters, digits, `_` and
/// `-`, anything else an underscore.
fn program_name(name: &str) -> String {
    let n: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if n.is_empty() { "PGM".into() } else { n }
}

fn units_word(units: ProgramUnits) -> &'static str {
    match units {
        ProgramUnits::Mm => "MM",
        ProgramUnits::Inch => "INCH",
    }
}

/// Render a stream into Klartext, called as `title` says: one `BEGIN PGM`
/// and `END PGM`, and the stock once, however many operations it holds.
pub(crate) fn render(ops: &[Op], profile: &Profile, title: &Title) -> (String, RenderReport) {
    let mut sections = values(ops, profile, title).into_iter().peekable();
    let first = sections.peek().map(|(_, v)| v.clone()).unwrap_or_default();
    let mut w = Writer::new(profile, first);
    let name = program_name(title.name);
    let units = units_word(w.values.units);
    w.block(&format!("BEGIN PGM {name} {units}"));
    // The stock, drawn as it is cut, first: the control looks for it there.
    if let Some((low, high)) = ops.iter().find_map(|o| match o {
        Op::Stock { low, high } => Some((*low, *high)),
        _ => None,
    }) {
        let (l, h) = (w.xyz(low), w.xyz(high));
        w.block(&format!("BLK FORM 0.1 Z {l}"));
        w.block(&format!("BLK FORM 0.2 {h}"));
    }
    for line in &profile.header {
        w.filled(line);
    }
    w.report.op_lines.reserve(ops.len());
    for (i, op) in ops.iter().enumerate() {
        if let Some((_, v)) = sections.next_if(|(start, _)| *start <= i) {
            w.values = v;
        }
        w.op(op);
        w.report.op_lines.push(w.report.lines);
    }
    for line in &profile.footer {
        w.filled(line);
    }
    w.block(&format!("END PGM {name} {units}"));
    (w.out, w.report)
}

/// One cycle's definition, as written: written again only when a hole needs
/// another.
#[derive(PartialEq)]
struct Definition(String);

struct Writer<'a> {
    p: &'a Profile,
    out: String,
    report: RenderReport,
    number: u32,
    values: Values,
    inches: bool,
    /// Where the machine has been told the tool is; `None` where not known.
    at: Option<DVec3>,
    /// The feed last written, as written.
    feed: Option<String>,
    /// Whether the tool is as high as it goes, its place otherwise unknown.
    raised: bool,
    /// The circle centre last written, as written.
    centre: Option<String>,
    /// The cycle last defined.
    defined: Option<Definition>,
    /// The height a run of holes starts from and returns to between them.
    clearance: Option<f64>,
    /// The spindle's speed, for a tap's pitch.
    rpm: u32,
}

impl<'a> Writer<'a> {
    fn new(p: &'a Profile, values: Values) -> Self {
        Self {
            p,
            out: String::new(),
            report: RenderReport::default(),
            number: p.numbering.start,
            inches: values.units == ProgramUnits::Inch,
            rpm: values.rpm,
            values,
            at: None,
            feed: None,
            raised: false,
            centre: None,
            defined: None,
            clearance: None,
        }
    }

    /// One block, numbered; its continuation lines, after a `~`, are not.
    fn block(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let _ = writeln!(self.out, "{} {text}", self.number);
        self.number = self.number.saturating_add(self.p.numbering.increment);
        self.report.lines += 1 + text.matches('\n').count() as u64;
    }

    fn comment(&mut self, text: &str) {
        let c = self.p.comment(text);
        self.block(&c);
    }

    fn filled(&mut self, line: &str) {
        let p = self.p;
        let text = fill(line, &self.values, &|t| p.escaped(t));
        self.block(&text);
    }

    /// The profile's lines for a step, after which where the machine is is
    /// not known.
    fn lines(&mut self, lines: &[String]) {
        for line in lines {
            self.filled(line);
        }
        if !lines.is_empty() {
            self.lost();
        }
    }

    fn lost(&mut self) {
        self.at = None;
        self.feed = None;
        self.raised = false;
    }

    /// A length, signed, as every Klartext coordinate is.
    fn num(&self, mm: f64) -> String {
        signed(self.p.length(mm, self.inches))
    }

    fn xyz(&self, v: DVec3) -> String {
        format!("X{} Y{} Z{}", self.num(v.x), self.num(v.y), self.num(v.z))
    }

    /// A feed in mm/min as an `F` word's number: tenths of an inch a minute
    /// in an inch program.
    fn feed_number(&self, mm_per_min: f64) -> String {
        if self.inches {
            self.p.feed(mm_per_min / MM_PER_TENTH)
        } else {
            self.p.feed(mm_per_min)
        }
    }

    fn op(&mut self, op: &Op) {
        match op {
            Op::SectionStart {
                op_name,
                tool,
                strategy,
                wcs,
            } => self.comment(&format!("{op_name} - {strategy} - T{tool} G{wcs}")),
            // Said once, by BEGIN PGM and BLK FORM.
            Op::Units(_) | Op::Stock { .. } | Op::SectionEnd | Op::ProgramEnd => {}
            Op::WorkOffset(n) => {
                let row = i32::from(*n) - 53;
                self.block(&format!(
                    "CYCL DEF 247 DATUM SETTING ~\n  Q339=+{row} ;DATUM NUMBER"
                ));
                self.at = None;
            }
            Op::SafeRetract => {
                let p = self.p;
                self.lines(&p.safe_retract);
                self.raised = !p.safe_retract.is_empty();
            }
            Op::Home => {
                let p = self.p;
                self.lines(&p.end_home);
            }
            Op::OptionalStop => {
                self.block("M1");
                self.lost();
            }
            Op::ToolChange { tool, comment } => self.tool_change(*tool, comment),
            Op::PreSelect(tool) => {
                if self.p.preselect_tool {
                    self.block(&format!("TOOL DEF {tool}"));
                }
            }
            Op::SpindleOn { rpm, dir } => {
                if *rpm != self.rpm {
                    // Another speed than the tool was called with.
                    self.block(&format!("TOOL CALL Z S{rpm}"));
                    self.rpm = *rpm;
                }
                self.block(match dir {
                    SpindleDirection::Clockwise => "M3",
                    SpindleDirection::CounterClockwise => "M4",
                });
            }
            Op::SpindleOff => self.block("M5"),
            Op::CoolantOn(c) => self.block(match c {
                Coolant::Flood => "M8",
                Coolant::Mist => "M7",
                Coolant::Off => "M9",
            }),
            Op::CoolantOff => self.block("M9"),
            Op::Rapid { to } => self.line(*to, None),
            Op::Feed { to, feed, .. } => self.line(*to, Some(*feed)),
            Op::Arc { to, arc, feed, .. } => self.arc(*to, arc, *feed),
            Op::CycleStart => {
                self.clearance = self.at.map(|a| a.z);
                self.defined = None;
            }
            Op::Cycle {
                kind,
                at,
                depth,
                retract,
                peck,
                dwell,
                feed,
            } => self.cycle(*kind, *at, *depth, *retract, *peck, *dwell, *feed),
            // Klartext taps only with cycle 207, which keeps the feed to the
            // spindle itself: a profile in it may not ask for these
            // (`Profile::validate`).
            Op::RigidMode { .. } | Op::RigidTap { .. } => {}
            // Never asked of Klartext: a Heidenhain profile cannot probe
            // (`Profile::validate`), so a probe operation is refused first.
            Op::Probe { .. } | Op::ProbeOpen(_) | Op::ProbeClose => {}
            // Never asked of Klartext: a setup wrapped round a rotary is
            // refused for it first (`LowerError::WrapNotWritten`).
            Op::Turning { .. } | Op::InverseTime(_) => {}
            Op::CycleEnd => {
                // Each cycle returns to the clearance height; where across is
                // where the last hole was.
                self.clearance = None;
            }
            Op::Raw(text) => {
                self.block(text);
                self.lost();
            }
            Op::Comment(text) => self.comment(text),
            Op::Custom(custom) => {
                let begin = custom_begin(self.p, &custom.name);
                self.block(&begin);
                for line in &custom.lines {
                    self.filled(line);
                }
                let end = custom_end(self.p);
                self.block(&end);
                self.lost();
            }
            Op::Rotary(at) => {
                if !self.raised {
                    let p = self.p;
                    self.lines(&p.safe_retract);
                    self.raised = !p.safe_retract.is_empty();
                }
                let words: Vec<String> = at
                    .iter()
                    .map(|a| format!("{}{}", a.axis, signed(self.p.axis(a.degrees))))
                    .collect();
                self.block(&format!("L {} R0 FMAX", words.join(" ")));
                self.at = None;
            }
        }
    }

    /// `TOOL CALL`: the tool in, its length and radius taken up, at the
    /// spindle's speed; by hand, a stop after it to fit it.
    fn tool_change(&mut self, tool: u32, comment: &str) {
        if !self.p.tool_change_lines.is_empty() {
            let p = self.p;
            for line in &p.tool_change_lines {
                self.filled(line);
            }
            self.lost();
            return;
        }
        let by_hand = self.p.tool_change == ToolChangeStyle::Pause;
        let note = match (by_hand, comment.is_empty()) {
            (true, true) => format!("fit tool {tool}"),
            (true, false) => format!("fit tool {tool}: {comment}"),
            (false, _) => comment.to_string(),
        };
        if !note.is_empty() {
            self.comment(&note);
        }
        // At the speed of the operation it is called for.
        let rpm = self.values.rpm;
        self.rpm = rpm;
        self.block(&format!("TOOL CALL {tool} Z S{rpm}"));
        if by_hand {
            self.block("M0");
            self.lost();
        } else {
            self.at = None;
        }
    }

    /// A straight move: at `FMAX` for a rapid, which holds for its block
    /// only, or at a feed. A rapid from a place not known goes across and
    /// down one at a time, as G-code's does.
    fn line(&mut self, to: DVec3, feed: Option<f64>) {
        if feed.is_none() && self.at.is_none() {
            let (x, y, z) = (self.num(to.x), self.num(to.y), self.num(to.z));
            let (first, second) = if self.raised {
                (format!("L X{x} Y{y} R0 FMAX"), format!("L Z{z} R0 FMAX"))
            } else {
                (format!("L Z{z} R0 FMAX"), format!("L X{x} Y{y} R0 FMAX"))
            };
            self.block(&first);
            self.block(&second);
            self.report.motion_blocks += 2;
            self.at = Some(to);
            self.raised = false;
            return;
        }
        self.raised = false;
        let words = ['X', 'Y', 'Z']
            .into_iter()
            .enumerate()
            .filter_map(|(i, axis)| {
                let v = self.num(to[i]);
                let same = self.at.is_some_and(|a| self.num(a[i]) == v);
                (!(same && self.p.modal.suppress_axes)).then(|| format!("{axis}{v}"))
            })
            .collect::<Vec<_>>();
        if words.is_empty() {
            self.at = Some(to);
            return;
        }
        let mut block = format!("L {} R0", words.join(" "));
        match feed {
            None => {
                block.push_str(" FMAX");
                // The feed after it is said again, as Fusion's posts say it.
                self.feed = None;
            }
            Some(f) => self.feed_word(&mut block, f),
        }
        self.block(&block);
        self.report.motion_blocks += 1;
        self.at = Some(to);
    }

    fn feed_word(&mut self, block: &mut String, mm_per_min: f64) {
        let f = self.feed_number(mm_per_min);
        if self.feed.as_deref() != Some(f.as_str()) || !self.p.modal.suppress_feed {
            let _ = write!(block, " F{f}");
        }
        self.feed = Some(f);
    }

    /// An arc, in pieces no more than a half turn long, each by its centre and
    /// its end; a helix about Z by its centre, the angle it turns and how far
    /// it goes down. A helix about X or Y, and a piece too short across to be
    /// written as an arc, as straight moves along it.
    fn arc(&mut self, to: DVec3, arc: &Arc, feed: f64) {
        let Some(from) = self.at else {
            self.line(to, Some(feed));
            return;
        };
        let a = arc.axis;
        let about_z = a.z.abs() >= a.x.abs().max(a.y.abs());
        // Along the axis it turns about, which the plane's normal is.
        let i = if about_z {
            2
        } else if a.y.abs() >= a.x.abs() {
            1
        } else {
            0
        };
        let helix = self.num(to[i]) != self.num(from[i]);
        if helix && !about_z {
            for p in arc.chords(from, to, STRAIGHTENED) {
                self.line(p, Some(feed));
            }
            return;
        }
        let pieces = (arc.sweep.abs() / PI - 1e-9).ceil().max(1.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let n = pieces.min(8.0) as u32;
        let mut at = from;
        for k in 1..=n {
            let end = if k == n {
                to
            } else {
                arc.point_at(from, to, f64::from(k) / f64::from(n))
            };
            let piece = Arc {
                sweep: arc.sweep / f64::from(n),
                ..*arc
            };
            let across = (end - at) - a * (end - at).dot(a);
            if across.length() < SHORTEST_CHORD {
                for p in piece.chords(at, end, STRAIGHTENED) {
                    self.line(p, Some(feed));
                }
            } else {
                self.arc_block(at, end, &piece, feed, helix);
            }
            at = end;
        }
    }

    fn arc_block(&mut self, from: DVec3, to: DVec3, arc: &Arc, feed: f64, helix: bool) {
        let a = arc.axis;
        // The plane's two axes, and whether it turns counter-clockwise about
        // the plane's normal (+Z, +Y or +X).
        let (axes, ccw) = if a.z.abs() >= a.x.abs().max(a.y.abs()) {
            ([('X', 0), ('Y', 1)], arc.sweep * a.z > 0.0)
        } else if a.y.abs() >= a.x.abs() {
            ([('X', 0), ('Z', 2)], arc.sweep * a.y > 0.0)
        } else {
            ([('Y', 1), ('Z', 2)], arc.sweep * a.x > 0.0)
        };
        let centre = arc.centre + a * (from - arc.centre).dot(a);
        let cc = format!(
            "CC {}{} {}{}",
            axes[0].0,
            self.num(centre[axes[0].1]),
            axes[1].0,
            self.num(centre[axes[1].1])
        );
        if self.centre.as_deref() != Some(cc.as_str()) || !self.p.modal.suppress_axes {
            self.block(&cc);
            self.centre = Some(cc);
        }
        let dr = if ccw { "DR+" } else { "DR-" };
        let mut block = if helix {
            // Down the rounded difference, so the rounding never adds up.
            let (z0, z1) = (
                self.p.length(from.z, self.inches),
                self.p.length(to.z, self.inches),
            );
            let dz = z1.parse::<f64>().unwrap_or(0.0) - z0.parse::<f64>().unwrap_or(0.0);
            let places = if self.inches {
                self.p.format.inch_axis_decimals
            } else {
                self.p.format.axis_decimals
            };
            let degrees = arc.sweep.abs().to_degrees() * if ccw { 1.0 } else { -1.0 };
            format!(
                "CP IPA{} IZ{} {dr}",
                signed(self.p.number(degrees, 4)),
                signed(self.p.number(dz, places)),
            )
        } else {
            format!(
                "C {}{} {}{} {dr}",
                axes[0].0,
                self.num(to[axes[0].1]),
                axes[1].0,
                self.num(to[axes[1].1])
            )
        };
        self.feed_word(&mut block, feed);
        self.block(&block);
        self.report.motion_blocks += 1;
        self.at = Some(to);
        self.raised = false;
    }

    /// One hole: its cycle defined, where it is not as the last hole's, and
    /// called there.
    #[allow(clippy::too_many_arguments)]
    fn cycle(
        &mut self,
        kind: CycleKind,
        at: DVec2,
        depth: f64,
        retract: f64,
        peck: Option<f64>,
        dwell: Option<f64>,
        feed: f64,
    ) {
        let top = retract - RETRACT_GAP;
        let clearance = self.clearance.unwrap_or(retract).max(retract);
        let n = |v: f64| self.num(v);
        let s = |v: f64| self.p.seconds(v);
        let lines: Vec<(u32, String, &str)> = match kind {
            CycleKind::Drill | CycleKind::DrillDwell | CycleKind::Peck => vec![
                (200, n(RETRACT_GAP), "SET-UP CLEARANCE"),
                (201, n(depth - top), "DEPTH"),
                (206, self.feed_number(feed), "FEED RATE FOR PLNGNG"),
                (202, n(peck.unwrap_or(top - depth)), "PLUNGING DEPTH"),
                (210, s(0.0), "DWELL TIME AT TOP"),
                (203, n(top), "SURFACE COORDINATE"),
                (204, n(clearance - top), "2ND SET-UP CLEARANCE"),
                (211, s(dwell.unwrap_or(0.0)), "DWELL TIME AT DEPTH"),
            ],
            CycleKind::ChipBreak => vec![
                (200, n(RETRACT_GAP), "SET-UP CLEARANCE"),
                (201, n(depth - top), "DEPTH"),
                (206, self.feed_number(feed), "FEED RATE FOR PLNGNG"),
                (202, n(peck.unwrap_or(top - depth)), "PLUNGING DEPTH"),
                (210, s(0.0), "DWELL TIME AT TOP"),
                (203, n(top), "SURFACE COORDINATE"),
                (204, n(clearance - top), "2ND SET-UP CLEARANCE"),
                (212, n(0.0), "DECREMENT"),
                (213, BREAKS.to_string(), "BREAKS"),
                (205, n(0.0), "MIN. PLUNGING DEPTH"),
                (211, s(0.0), "DWELL TIME AT DEPTH"),
                (208, "MAX".into(), "RETRACTION FEED RATE"),
                (256, n(CHIP_BREAK), "DIST. FOR CHIP BRKNG"),
            ],
            CycleKind::Tap | CycleKind::TapLeft => {
                // A tap's feed is its pitch a turn: the pitch is what the
                // cycle takes, and its sign the thread's hand. To three more
                // places than a length, as a tap's feed is in G-code: rounded
                // to a length's, an inch pitch would gain or lose on the
                // thread every turn.
                let pitch = feed / f64::from(self.rpm.max(1));
                let hand = if kind == CycleKind::Tap { 1.0 } else { -1.0 };
                let places = if self.inches {
                    self.p.format.inch_axis_decimals
                } else {
                    self.p.format.axis_decimals
                };
                let scale = if self.inches { MM_PER_INCH } else { 1.0 };
                let pitch = self
                    .p
                    .number(hand * pitch / scale, places.saturating_add(3));
                vec![
                    (200, n(RETRACT_GAP), "SET-UP CLEARANCE"),
                    (201, n(depth - top), "DEPTH"),
                    (239, pitch, "THREAD PITCH"),
                    (203, n(top), "SURFACE COORDINATE"),
                    (204, n(clearance - top), "2ND SET-UP CLEARANCE"),
                ]
            }
        };
        let (number, name) = match kind {
            CycleKind::Drill | CycleKind::DrillDwell | CycleKind::Peck => (200, "DRILLING"),
            CycleKind::ChipBreak => (203, "UNIVERSAL DRILLING"),
            CycleKind::Tap | CycleKind::TapLeft => (207, "RIGID TAPPING NEW"),
        };
        let mut text = format!("CYCL DEF {number} {name}");
        // Each value signed, as the control writes them, but MAX.
        for (q, value, what) in lines {
            let value = if value == "MAX" { value } else { signed(value) };
            let _ = write!(text, " ~\n  Q{q}={value} ;{what}");
        }
        let definition = Definition(text);
        if self.defined.as_ref() != Some(&definition) {
            self.block(&definition.0);
            self.defined = Some(definition);
        }
        let call = format!("L X{} Y{} R0 FMAX M99", self.num(at.x), self.num(at.y));
        self.block(&call);
        self.report.motion_blocks += 1;
        // It ends back at the clearance height, over the hole.
        self.at = Some(dvec3(at.x, at.y, clearance));
        self.feed = None;
    }
}

/// `+` before a number that has no sign.
fn signed(s: String) -> String {
    if s.starts_with('-') || s.starts_with('+') {
        s
    } else {
        format!("+{s}")
    }
}

/// Where `text`'s custom lines are: the line (from one) of each comment that
/// begins a block of them, with the line of the one that ends it, as
/// [`crate::readback::custom_blocks`] finds them, past each line's number.
pub fn custom_blocks(text: &str, profile: &Profile) -> Vec<(usize, usize)> {
    let (mark, end) = (
        crate::render::custom_mark(profile),
        crate::render::custom_end(profile),
    );
    let lines: Vec<&str> = text.lines().map(unnumbered).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(line) = lines.get(i) {
        let ends = if line.starts_with(&mark) {
            (i + 1..lines.len()).find(|&j| lines.get(j) == Some(&end.as_str()))
        } else {
            None
        };
        match ends {
            Some(j) => {
                out.push((i + 1, j + 1));
                i = j + 1;
            }
            None => i += 1,
        }
    }
    out
}

/// A line without its block number, and the `/` that makes a block one to
/// skip.
fn unnumbered(line: &str) -> &str {
    let t = line.trim_start().trim_start_matches('/');
    let rest = t.trim_start_matches(|c: char| c.is_ascii_digit());
    if rest.len() == t.len() {
        line.trim()
    } else {
        rest.trim()
    }
}

/// What is wrong with a line of custom Klartext, if anything: one that
/// moves the machine (a path function, a cycle, `M91`), changes the tool or
/// ends the program is refused, as custom G-code's are; a call of another
/// program or a label is not read, and warned about.
pub fn concern(line: &str) -> Option<Concern> {
    let code = unnumbered(line);
    let code = code.split(';').next().unwrap_or("").trim();
    let first = code
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    let refused = |why: &str| Some(Concern::Refused(format!("{first} {why}")));
    match first.as_str() {
        "" | "*" => None,
        "L" | "LN" | "LP" | "C" | "CC" | "CR" | "CT" | "CP" | "CTP" | "RND" | "CHF" | "APPR"
        | "DEP" | "FK" | "FL" | "FC" | "FPOL" => {
            refused("moves the machine: custom lines may not move it")
        }
        "TOOL" => refused("changes the tool, which moves the machine"),
        "CYCL" | "TCH" | "PLANE" => {
            refused("is a cycle, which moves the machine or how the program is read")
        }
        "BEGIN" | "END" => refused("begins or ends a program"),
        "CALL" | "LBL" | "FN" | "SQL" => Some(Concern::Unread(format!(
            "{first} calls something that is not read: what it does is not checked"
        ))),
        _ => {
            // M functions, alone or after STOP.
            for word in code.split_whitespace() {
                let w = word.to_ascii_uppercase();
                let Some(m) = w.strip_prefix('M').and_then(|n| n.parse::<u32>().ok()) else {
                    continue;
                };
                match m {
                    2 | 30 | 99 => return refused("ends the program, or calls a cycle"),
                    6 | 91 | 92 | 140 => return refused("moves the machine"),
                    _ => {}
                }
            }
            None
        }
    }
}

/// One block's text, its continuation lines joined, comments left out, and
/// the line (from one) it starts on.
struct Block {
    line: usize,
    words: Vec<String>,
    /// For a cycle's definition, its `Q` parameters by number.
    q: Vec<(u32, Q)>,
}

/// A cycle parameter's value.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Q {
    Number(f64),
    /// `MAX`: as fast as it goes.
    Max,
}

/// The blocks of `text`, its custom lines (between `custom`) left out once
/// checked. A cycle's parameters, `Q200=+2 ;SET-UP CLEARANCE`, are lines of
/// their own with no number, after its `CYCL DEF`; each line but the last
/// may end in `~`, as newer controls write them.
fn blocks(text: &str, custom: &[(usize, usize)]) -> Result<Vec<Block>, Problem> {
    let mut out: Vec<Block> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        if custom.iter().any(|(b, e)| line > *b && line < *e) {
            if let Some(Concern::Refused(why)) = concern(raw) {
                return Err(Problem::at(line, format!("custom lines: {why}")));
            }
            continue;
        }
        let code = raw.split(';').next().unwrap_or("");
        let code = code.trim().trim_end_matches('~').trim();
        let t = raw.trim_start().trim_start_matches('/');
        let numbered = t.starts_with(|c: char| c.is_ascii_digit());
        if numbered {
            out.push(Block {
                line,
                words: unnumbered(code)
                    .split_whitespace()
                    .map(str::to_ascii_uppercase)
                    .collect(),
                q: Vec::new(),
            });
            continue;
        }
        if code.is_empty() {
            if raw.trim().is_empty() {
                continue;
            }
            // A parameter's comment alone, or a block with no number.
            if !raw.trim_start().starts_with(';') {
                return Err(Problem::at(line, "a block with no number"));
            }
            continue;
        }
        let Some(b) = out
            .last_mut()
            .filter(|b| b.words.first().is_some_and(|w| w == "CYCL"))
        else {
            return Err(Problem::at(line, "a block with no number"));
        };
        for word in code.split_whitespace() {
            let bad = || Problem::at(line, format!("{word} is not a cycle parameter"));
            let (k, v) = word
                .strip_prefix('Q')
                .and_then(|w| w.split_once('='))
                .ok_or_else(bad)?;
            let k: u32 = k.parse().map_err(|_| bad())?;
            let v = if v.eq_ignore_ascii_case("MAX") {
                Q::Max
            } else {
                Q::Number(v.parse().map_err(|_| bad())?)
            };
            b.q.push((k, v));
        }
    }
    Ok(out)
}

/// What a Klartext program does, step by step, in millimetres, as
/// [`crate::readback::read`] says of G-code: refused where it says
/// something this cannot read.
pub fn read(text: &str) -> Result<Vec<Step>, Problem> {
    read_around(text, &[])
}

/// [`read`], its custom lines between each pair of `blocks` lines only
/// checked for what they may not do, and [`Action::Custom`] once they end.
pub fn read_around(text: &str, custom: &[(usize, usize)]) -> Result<Vec<Step>, Problem> {
    let mut r = Reader::default();
    let mut ends = custom.iter().map(|(_, e)| *e).collect::<Vec<_>>();
    ends.sort_unstable();
    for b in blocks(text, custom)? {
        // A custom block's end comes before the block after it.
        while let Some(&e) = ends.first() {
            if e > b.line {
                break;
            }
            ends.remove(0);
            r.line = e;
            r.push(Action::Custom);
            r.feed = None;
            r.speed = None;
            r.spin = None;
        }
        r.line = b.line;
        if r.ended {
            return Err(Problem::at(b.line, "a block after END PGM"));
        }
        r.block(&b).map_err(|what| Problem::at(b.line, what))?;
    }
    if !r.ended {
        return Err(Problem::whole("the program has no END PGM"));
    }
    Ok(r.steps)
}

/// A defined cycle: its number and parameters.
#[derive(Default, Clone)]
struct Cycle {
    number: u32,
    q: Vec<(u32, Q)>,
}

impl Cycle {
    fn get(&self, k: u32) -> Option<Q> {
        self.q.iter().rev().find(|(n, _)| *n == k).map(|(_, v)| *v)
    }

    fn number(&self, k: u32) -> Result<f64, String> {
        match self.get(k) {
            Some(Q::Number(v)) => Ok(v),
            Some(Q::Max) => Err(format!(
                "Q{k}=MAX is not understood in cycle {}",
                self.number
            )),
            None => Err(format!("cycle {} has no Q{k}", self.number)),
        }
    }
}

#[derive(Default)]
struct Reader {
    steps: Vec<Step>,
    line: usize,
    /// Millimetres in a unit, once `BEGIN PGM` gives it.
    scale: Option<f64>,
    begun: bool,
    ended: bool,
    feed: Option<f64>,
    speed: Option<f64>,
    at: Position,
    /// The circle centre, by axis; `None` where it gives none.
    centre: Option<[Option<f64>; 3]>,
    cycle: Option<Cycle>,
    spin: Option<SpindleDirection>,
}

/// A signed number after a word's letters, as `X+10.5` holds `+10.5`.
fn value(word: &str, prefix: &str) -> Option<Result<f64, String>> {
    let rest = word.strip_prefix(prefix)?;
    if rest.is_empty() || !rest.starts_with(|c: char| c.is_ascii_digit() || "+-.".contains(c)) {
        return None;
    }
    Some(rest.parse().map_err(|_| format!("{word} is not a number")))
}

fn whole(word: &str, v: f64) -> Result<u32, String> {
    if v.fract() != 0.0 || !(0.0..=f64::from(u32::MAX)).contains(&v) {
        return Err(format!("{word} is not a whole number"));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(v as u32)
}

/// A path block's words gathered.
#[derive(Default)]
struct Words {
    axes: [Option<f64>; 3],
    incremental: [bool; 3],
    rotary: [Option<f64>; 3],
    feed: Option<f64>,
    fmax: bool,
    m: Vec<u32>,
    dr: Option<bool>,
    ipa: Option<f64>,
    max: bool,
}

impl Words {
    fn gather(words: &[String]) -> Result<Self, String> {
        let mut w = Self::default();
        let set = |slot: &mut Option<f64>, v: f64, word: &str| {
            if slot.is_some() {
                return Err(format!("{word}: that axis appears twice"));
            }
            *slot = Some(v);
            Ok(())
        };
        for word in words {
            let word = word.as_str();
            match word {
                "R0" => continue,
                "RL" | "RR" => return Err("radius compensation is not understood".into()),
                "FMAX" => {
                    w.fmax = true;
                    continue;
                }
                "DR+" => {
                    w.dr = Some(true);
                    continue;
                }
                "DR-" => {
                    w.dr = Some(false);
                    continue;
                }
                "MB" => continue,
                "MAX" => {
                    w.max = true;
                    continue;
                }
                _ => {}
            }
            if let Some(v) = value(word, "IPA") {
                w.ipa = Some(v?);
                continue;
            }
            let mut found = false;
            for ((axis, slot), incremental) in ["X", "Y", "Z"]
                .into_iter()
                .zip(w.axes.iter_mut())
                .zip(w.incremental.iter_mut())
            {
                if let Some(v) = value(word, &format!("I{axis}")) {
                    set(slot, v?, word)?;
                    *incremental = true;
                    found = true;
                } else if let Some(v) = value(word, axis) {
                    set(slot, v?, word)?;
                    found = true;
                }
            }
            for (axis, slot) in ["A", "B", "C"].into_iter().zip(w.rotary.iter_mut()) {
                if let Some(v) = value(word, axis) {
                    set(slot, v?, word)?;
                    found = true;
                }
            }
            if found {
                continue;
            }
            if let Some(v) = value(word, "F") {
                w.feed = Some(v?);
            } else if let Some(v) = value(word, "M") {
                w.m.push(whole(word, v?)?);
            } else {
                return Err(format!("{word} is not understood"));
            }
        }
        Ok(w)
    }
}

impl Reader {
    fn push(&mut self, action: Action) {
        self.steps.push(Step {
            line: self.line,
            action,
        });
    }

    fn scale(&self) -> Result<f64, String> {
        self.scale.ok_or_else(|| "a block before BEGIN PGM".into())
    }

    fn block(&mut self, b: &Block) -> Result<(), String> {
        let w = &b.words;
        let first = w.first().map(String::as_str).unwrap_or("");
        let second = w.get(1).map(String::as_str).unwrap_or("");
        if !self.begun && first != "BEGIN" && !first.is_empty() {
            return Err("a block before BEGIN PGM".into());
        }
        match (first, second) {
            // A comment, or a structure item.
            ("", _) | ("*", _) => Ok(()),
            ("BEGIN", "PGM") => {
                if self.begun {
                    return Err("BEGIN PGM twice".into());
                }
                self.begun = true;
                self.scale = Some(match w.get(3).map(String::as_str) {
                    Some("MM") => 1.0,
                    Some("INCH") => MM_PER_INCH,
                    _ => return Err("BEGIN PGM without MM or INCH".into()),
                });
                Ok(())
            }
            ("END", "PGM") => {
                if !self.steps.iter().any(|s| s.action == Action::End) {
                    self.push(Action::End);
                }
                self.ended = true;
                Ok(())
            }
            ("BLK", "FORM") => Ok(()),
            ("TOOL", "CALL") => self.tool_call(w.get(2..).unwrap_or_default()),
            ("TOOL", "DEF") => {
                let t = w.get(2).ok_or("TOOL DEF with no tool")?;
                let t = whole(t, t.parse().map_err(|_| format!("{t} is not a tool"))?)?;
                self.push(Action::Select(t));
                Ok(())
            }
            ("CYCL", "DEF") => self.define(b),
            ("CYCL", "CALL") => {
                let words = Words::gather(w.get(2..).unwrap_or_default())?;
                self.ms_before(&words.m)?;
                self.call()?;
                self.ms_after(&words.m)
            }
            ("STOP", _) => {
                let words = Words::gather(w.get(1..).unwrap_or_default())?;
                self.ms_before(&words.m)?;
                self.ms_after(&words.m)?;
                if !words.m.iter().any(|m| matches!(m, 0 | 1 | 2 | 30)) {
                    self.pause();
                }
                Ok(())
            }
            ("L", _) => self.line(w.get(1..).unwrap_or_default()),
            ("CC", _) => self.circle_centre(w.get(1..).unwrap_or_default()),
            ("C", _) => self.arc(w.get(1..).unwrap_or_default(), false),
            ("CP", _) => self.arc(w.get(1..).unwrap_or_default(), true),
            (m, _) if m.starts_with('M') => {
                let words = Words::gather(w)?;
                if words.axes.iter().any(Option::is_some) || words.feed.is_some() {
                    return Err("an M block with more than M functions".into());
                }
                if words.m.iter().any(|m| matches!(m, 91 | 140)) {
                    return Err("M91 or M140 with nowhere to go".into());
                }
                self.ms_before(&words.m)?;
                if words.m.contains(&99) {
                    self.call()?;
                }
                self.ms_after(&words.m)
            }
            (other, _) => Err(format!("{other} blocks are not understood")),
        }
    }

    fn tool_call(&mut self, w: &[String]) -> Result<(), String> {
        let mut rest = w;
        let mut tool = None;
        if let Some(t) = rest.first()
            && t.chars().all(|c| c.is_ascii_digit())
        {
            tool = Some(whole(
                t,
                t.parse().map_err(|_| format!("{t} is not a tool"))?,
            )?);
            rest = rest.get(1..).unwrap_or_default();
        }
        match rest.first().map(String::as_str) {
            Some("Z") => rest = rest.get(1..).unwrap_or_default(),
            Some(axis @ ("X" | "Y")) => {
                return Err(format!("TOOL CALL along {axis} is not understood"));
            }
            _ => {}
        }
        for word in rest {
            if let Some(s) = value(word, "S") {
                self.speed = Some(s?);
            } else if let Some(f) = value(word, "F") {
                self.feed = Some(self.feed_mm(f?)?);
            } else if word.starts_with("DL") || word.starts_with("DR") {
                return Err(format!("{word}: oversizes in TOOL CALL are not understood"));
            } else {
                return Err(format!("{word} is not understood in TOOL CALL"));
            }
        }
        if let Some(t) = tool {
            self.push(Action::Select(t));
            self.push(Action::Change(t));
            self.at = [None; 3];
        }
        Ok(())
    }

    fn inches(&self) -> bool {
        self.scale == Some(MM_PER_INCH)
    }

    /// A feed word in mm/min: tenths of an inch a minute in inches.
    fn feed_mm(&self, f: f64) -> Result<f64, String> {
        Ok(if self.inches() {
            f * MM_PER_TENTH
        } else {
            f * self.scale()?
        })
    }

    /// The M functions that act as a block begins: the spindle and coolant
    /// on.
    fn ms_before(&mut self, ms: &[u32]) -> Result<(), String> {
        for &m in ms {
            match m {
                3 | 4 | 13 | 14 => {
                    let s = self.speed.ok_or("the spindle started with no speed")?;
                    let rpm = whole(&format!("S{s}"), s.round())?;
                    let dir = if matches!(m, 3 | 13) {
                        SpindleDirection::Clockwise
                    } else {
                        SpindleDirection::CounterClockwise
                    };
                    self.push(Action::SpindleOn { rpm, dir });
                    self.spin = Some(dir);
                    if m > 10 {
                        self.push(Action::Coolant(Coolant::Flood));
                    }
                }
                7 => self.push(Action::Coolant(Coolant::Mist)),
                8 => self.push(Action::Coolant(Coolant::Flood)),
                0 | 1 | 2 | 5 | 9 | 30 | 91 | 99 | 140 => {}
                other => return Err(format!("M{other} is not understood")),
            }
        }
        Ok(())
    }

    /// The M functions that act as a block ends: stops and the end.
    fn ms_after(&mut self, ms: &[u32]) -> Result<(), String> {
        for &m in ms {
            match m {
                5 => {
                    self.push(Action::SpindleOff);
                    self.spin = None;
                }
                9 => self.push(Action::Coolant(Coolant::Off)),
                0 | 1 => self.pause(),
                2 | 30 => self.push(Action::End),
                _ => {}
            }
        }
        Ok(())
    }

    fn pause(&mut self) {
        self.push(Action::Pause);
        self.at = [None; 3];
    }

    fn go(&mut self, to: Position, feed: Option<f64>, arc: Option<Turn>) {
        self.push(Action::Move(Motion {
            from: self.at,
            to,
            feed,
            arc,
            probe: false,
        }));
        self.at = to;
    }

    fn target(&self, w: &Words) -> Result<Position, String> {
        let scale = self.scale()?;
        let mut to = self.at;
        for (((slot, word), incremental), now) in
            to.iter_mut().zip(w.axes).zip(w.incremental).zip(self.at)
        {
            let Some(v) = word else { continue };
            *slot = if incremental {
                match now {
                    Some(now) => Some(now + v * scale),
                    None => return Err("an incremental move from where is not known".into()),
                }
            } else {
                Some(v * scale)
            };
        }
        Ok(to)
    }

    /// The feed a path block moves at: its own, which holds, or the last;
    /// `None` at `FMAX`, for this block only.
    fn path_feed(&mut self, w: &Words) -> Result<Option<f64>, String> {
        if let Some(f) = w.feed {
            if w.fmax {
                return Err("both FMAX and F".into());
            }
            self.feed = Some(self.feed_mm(f)?);
        }
        if w.fmax {
            return Ok(None);
        }
        self.feed
            .map(Some)
            .ok_or_else(|| "a feed move with no feed".into())
    }

    fn line(&mut self, words: &[String]) -> Result<(), String> {
        let w = Words::gather(words)?;
        if w.dr.is_some() || w.ipa.is_some() {
            return Err("arc words in a straight move".into());
        }
        self.ms_before(&w.m)?;
        let has_axes = w.axes.iter().any(Option::is_some);
        if w.m.contains(&140) {
            // Up the tool axis as far as it goes.
            if !w.max || has_axes {
                return Err("only M140 MB MAX, alone, is understood".into());
            }
            self.push(Action::Home([false, false, true]));
            self.at[2] = None;
        } else if w.rotary.iter().any(Option::is_some) {
            if has_axes || !w.fmax {
                return Err(
                    "a rotary turned other than alone at FMAX: only positioning, \
                            a rotary alone, is understood"
                        .into(),
                );
            }
            let at = ['A', 'B', 'C']
                .into_iter()
                .zip(w.rotary)
                .filter_map(|(n, v)| Some((n, v?)))
                .collect();
            self.push(Action::Rotary(at));
        } else if w.m.contains(&91) {
            if w.incremental.iter().any(|i| *i) || !w.fmax {
                return Err("M91 is understood only at FMAX, in absolute".into());
            }
            let scale = self.scale()?;
            let to = w.axes.map(|a| a.map(|v| v * scale));
            self.push(Action::Machine(to));
            for (at, a) in self.at.iter_mut().zip(to) {
                if a.is_some() {
                    *at = None;
                }
            }
        } else if has_axes {
            let feed = self.path_feed(&w)?;
            let to = self.target(&w)?;
            self.go(to, feed, None);
        } else if w.feed.is_some() {
            self.path_feed(&w)?;
        }
        if w.m.contains(&99) {
            self.call()?;
        }
        self.ms_after(&w.m)
    }

    fn circle_centre(&mut self, words: &[String]) -> Result<(), String> {
        let w = Words::gather(words)?;
        if w.feed.is_some() || w.fmax || !w.m.is_empty() || w.dr.is_some() {
            return Err("CC holds only a centre".into());
        }
        let given = w.axes.iter().filter(|a| a.is_some()).count();
        if given != 2 {
            return Err("CC with other than two axes is not understood".into());
        }
        let scale = self.scale()?;
        let mut c = [None; 3];
        for (((slot, word), incremental), now) in
            c.iter_mut().zip(w.axes).zip(w.incremental).zip(self.at)
        {
            let Some(v) = word else { continue };
            *slot = Some(if incremental {
                now.ok_or("an incremental centre from where is not known")? + v * scale
            } else {
                v * scale
            });
        }
        self.centre = Some(c);
        Ok(())
    }

    /// `C` to an end, or `CP` round by an angle, about the centre `CC` last
    /// gave, in the plane its axes name.
    fn arc(&mut self, words: &[String], polar: bool) -> Result<(), String> {
        let w = Words::gather(words)?;
        if w.fmax {
            return Err("an arc at FMAX".into());
        }
        let ccw = w.dr.ok_or("an arc with no DR")?;
        let c = self.centre.ok_or("an arc with no CC")?;
        let normal_axis = c.iter().position(Option::is_none).unwrap_or(2);
        let n = match normal_axis {
            0 => DVec3::X,
            1 => DVec3::Y,
            _ => DVec3::Z,
        };
        self.ms_before(&w.m)?;
        let feed = self.path_feed(&w)?.ok_or("an arc with no feed")?;
        let from = known(&self.at).ok_or("an arc from where is not known")?;
        // The centre in the plane, level with the start along its normal.
        let mut centre = from;
        for (i, v) in c.iter().enumerate() {
            if let Some(v) = v {
                centre[i] = *v;
            }
        }
        // Along the plane's normal: its word, and whether it is incremental.
        let along = w.axes.get(normal_axis).copied().flatten();
        let along_incremental = w.incremental.get(normal_axis).copied().unwrap_or(false);
        let r0 = flat(from - centre, n);
        if r0.length() == 0.0 {
            return Err("an arc that starts at its centre".into());
        }
        let (to, sweep) = if polar {
            let degrees = w.ipa.ok_or("CP without IPA is not understood")?;
            if (degrees > 0.0) != ccw {
                return Err("IPA and DR turn different ways".into());
            }
            if w.axes
                .iter()
                .enumerate()
                .any(|(i, a)| a.is_some() && i != normal_axis)
                || along.is_some() && !along_incremental
            {
                return Err("CP is understood only with IPA and an incremental move \
                            along the plane's normal"
                    .into());
            }
            let sweep = degrees.to_radians();
            let along = along.unwrap_or(0.0) * self.scale()?;
            let end = flat(centre, n) + turned(r0, n, sweep) + n * (from.dot(n) + along);
            (end, sweep)
        } else {
            if w.ipa.is_some() {
                return Err("IPA in a C block".into());
            }
            if along.is_some() {
                return Err("C moving along its plane's normal is not understood".into());
            }
            let to = known(&self.target(&w)?).ok_or("an arc to where is not known")?;
            let r1 = flat(to - centre, n);
            if r1.length() == 0.0 {
                return Err("an arc that ends at its centre".into());
            }
            let angle = n.dot(r0.cross(r1)).atan2(r0.dot(r1));
            let sweep = match (ccw, angle) {
                (true, a) if a <= 0.0 => a + TAU,
                (false, a) if a >= 0.0 => a - TAU,
                (_, a) => a,
            };
            (to, sweep)
        };
        let turn = Turn {
            centre: flat(centre, n) + n * from.dot(n),
            normal: n,
            sweep,
        };
        self.go([Some(to.x), Some(to.y), Some(to.z)], Some(feed), Some(turn));
        self.ms_after(&w.m)
    }

    fn define(&mut self, b: &Block) -> Result<(), String> {
        let number = b.words.get(2).ok_or("CYCL DEF with no cycle")?;
        let number: f64 = number
            .parse()
            .map_err(|_| format!("{number} is not a cycle"))?;
        let cycle = Cycle {
            number: whole(&format!("CYCL DEF {number}"), number)?,
            q: b.q.clone(),
        };
        match cycle.number {
            247 => {
                let row = cycle.number(339)?;
                let row = whole(&format!("Q339={row}"), row)?;
                if !(1..=6).contains(&row) {
                    return Err(format!("datum {row} is not one of G54 to G59's"));
                }
                let n = u8::try_from(53 + row).map_err(|_| "a datum out of range")?;
                self.push(Action::WorkOffset(n));
                self.at = [None; 3];
                Ok(())
            }
            200 | 203 | 207 => {
                self.cycle = Some(cycle);
                Ok(())
            }
            other => Err(format!("cycle {other} is not understood")),
        }
    }

    /// The defined cycle, run where the tool is: the moves its manual gives
    /// it, as the drilling cycles' are read in G-code.
    fn call(&mut self) -> Result<(), String> {
        let c = self
            .cycle
            .clone()
            .ok_or("a cycle called with none defined")?;
        let scale = self.scale()?;
        let (x, y) = (self.at[0], self.at[1]);
        if x.is_none() || y.is_none() {
            return Err("a cycle called at a place not known".into());
        }
        self.at[2].ok_or("a cycle called from a height not known")?;
        let top = c.number(203)? * scale;
        let set_up = top + c.number(200)? * scale;
        let depth = c.number(201)?;
        if depth >= 0.0 {
            return Err("a cycle's depth (Q201) is not below its surface".into());
        }
        let bottom = top + depth * scale;
        let clear = top + c.number(204)? * scale;
        let at = |z: f64| [x, y, Some(z)];
        self.go(at(set_up), None, None);
        match c.number {
            207 => {
                let pitch = c.number(239)? * scale;
                let want = if pitch > 0.0 {
                    SpindleDirection::Clockwise
                } else {
                    SpindleDirection::CounterClockwise
                };
                if self.spin != Some(want) {
                    return Err(format!(
                        "cycle 207 tapping {} without the spindle turning {}",
                        if pitch > 0.0 {
                            "right-hand"
                        } else {
                            "left-hand"
                        },
                        if pitch > 0.0 { "M3" } else { "M4" }
                    ));
                }
                let rpm = self.speed.ok_or("tapping with no speed")?;
                let feed = pitch.abs() * rpm;
                self.go(at(bottom), Some(feed), None);
                self.go(at(set_up), Some(feed), None);
            }
            number => {
                let feed = self.feed_mm(c.number(206)?)?;
                let peck = c.number(202)? * scale;
                if peck <= 0.0 {
                    return Err(format!("cycle {number} with no plunging depth"));
                }
                let (breaks, back) = if number == 203 {
                    if c.number(212)? != 0.0 || c.number(205)? != 0.0 {
                        return Err("cycle 203 with a decrement is not understood".into());
                    }
                    if c.get(208) != Some(Q::Max) {
                        return Err("cycle 203 retracting at a feed is not understood: \
                                    Q208=MAX"
                            .into());
                    }
                    (whole("Q213", c.number(213)?)?, c.number(256)? * scale)
                } else {
                    (0, 0.0)
                };
                let mut drilled = top;
                let mut broken = 0;
                loop {
                    let next = (drilled - peck).max(bottom);
                    self.go(at(next), Some(feed), None);
                    drilled = next;
                    if drilled <= bottom {
                        break;
                    }
                    if broken < breaks {
                        self.go(at((drilled + back).min(set_up)), None, None);
                        broken += 1;
                    } else {
                        // Out of the hole, and back down to what is drilled.
                        self.go(at(set_up), None, None);
                        self.go(at(drilled), None, None);
                        broken = 0;
                    }
                }
                let dwell = c.number(211)?;
                if dwell > 0.0 {
                    self.push(Action::Dwell(dwell));
                }
                self.go(at(set_up), None, None);
            }
        }
        if clear > set_up {
            self.go(at(clear), None, None);
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests;
