//! Rendering an instruction stream into one dialect.
//!
//! The renderer holds the only mutable machine state in the crate: what motion mode,
//! position and feed the machine has actually been told. Modal suppression is
//! decided against *that*, not against what the caller meant, because a word omitted
//! as "unchanged" is only safe if the machine already has the value.

use std::fmt::Write as _;

use std::f64::consts::{FRAC_PI_2, PI};

use camcraft_core::job::{Coolant, DEFAULT_NUMBER, SpindleDirection};
use camcraft_core::path::toolpath::Arc;
use glam::{DVec2, DVec3};

use crate::op::{CycleKind, Op};
use crate::placeholder::{Values, fill};
use crate::profile::{ArcStyle, Dialect, Profile, ProgramUnits, ToolChangeStyle};
use crate::rotary::RotaryAt;

/// Arcs shorter than this across their chord, in millimetres, are written as
/// straight moves, as Fusion's posts write them (`minimumChordLength`): an arc
/// whose ends round to the same point reads as a full circle.
pub(crate) const SHORTEST_CHORD: f64 = 0.25;

/// How far the straight moves written for such an arc may stray from it, in
/// millimetres: Fusion's posts' `tolerance`.
pub(crate) const STRAIGHTENED: f64 = 0.002;

/// What rendering produced besides the text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderReport {
    /// Lines written.
    pub lines: u64,
    /// Motion blocks written.
    pub motion_blocks: u64,
    /// How many lines are written once each instruction is, one for each:
    /// which instruction wrote which line, with `op::lower_counted` which
    /// move.
    pub op_lines: Vec<u64>,
}

/// Render a stream into G-code, `@date@` left empty and `@number@` the
/// default.
pub fn render(ops: &[Op], profile: &Profile, program_name: &str) -> (String, RenderReport) {
    render_dated(ops, profile, program_name, None)
}

/// Render a stream into G-code, posted on `date` (as `2026-11-03`) where it
/// is given, which `@date@` is replaced with.
pub fn render_dated(
    ops: &[Op],
    profile: &Profile,
    program_name: &str,
    date: Option<&str>,
) -> (String, RenderReport) {
    render_program(
        ops,
        profile,
        &Title {
            name: program_name,
            number: DEFAULT_NUMBER,
            date,
        },
    )
}

/// What a program is called, for its placeholders.
#[derive(Debug, Clone, Copy)]
pub struct Title<'a> {
    /// Its name, `@name@`.
    pub name: &'a str,
    /// Its number, `@number@`.
    pub number: u32,
    /// The day it is posted, as `2026-11-03`, `@date@`, where it is given.
    pub date: Option<&'a str>,
}

/// Render a stream into G-code, called as `title` says.
pub fn render_program(ops: &[Op], profile: &Profile, title: &Title) -> (String, RenderReport) {
    if profile.dialect == Dialect::Heidenhain {
        return crate::heidenhain::render(ops, profile, title);
    }
    let mut w = Writer::new(profile);
    let mut sections = values(ops, profile, title).into_iter().peekable();
    if let Some((_, v)) = sections.peek() {
        w.values = v.clone();
    }
    let p = w.p;
    if w.siemens {
        w.program_header(title.name);
    }
    for line in &p.header {
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
    for line in &p.footer {
        w.filled(line);
    }
    (w.out, w.report)
}

/// What the placeholders stand for in each operation of a program of `ops`
/// called as `title` says, as its instructions give them, with the
/// instruction each begins at: an operation's run from just after the one
/// before ends, so the custom G-code before it is filled with its values.
/// What an operation does not set again, as a tool kept spinning for the
/// next, is the one before's.
pub(crate) fn values(ops: &[Op], profile: &Profile, title: &Title) -> Vec<(usize, Values)> {
    let mut v = Values {
        name: title.name.to_string(),
        number: title.number,
        date: title.date.map(str::to_string),
        ..Values::default()
    };
    let mut out = Vec::new();
    let mut start = 0;
    // The coolant left on, which an operation that does not turn it again
    // runs with.
    let mut coolant = Coolant::Off;
    for (i, op) in ops.iter().enumerate() {
        match op {
            Op::SectionStart {
                op_name,
                tool,
                strategy,
                wcs,
            } => {
                v.operation.clone_from(op_name);
                v.tool = *tool;
                v.strategy.clone_from(strategy);
                v.wcs = profile.wcs_word(*wcs).unwrap_or_else(|| format!("G{wcs}"));
                v.coolant = coolant;
            }
            Op::ToolChange { comment, .. } => v.tool_comment.clone_from(comment),
            Op::SpindleOn { rpm, dir } => {
                v.rpm = *rpm;
                v.spindle = *dir;
            }
            Op::CoolantOn(c) => {
                v.coolant = *c;
                coolant = *c;
            }
            Op::CoolantOff => coolant = Coolant::Off,
            Op::Units(u) => v.units = *u,
            Op::SectionEnd => {
                out.push((start, v.clone()));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push((start, v));
    out
}

/// The comment custom G-code is written after, which says whose it is:
/// read back, it begins with [`custom_mark`].
pub fn custom_begin(profile: &Profile, name: &str) -> String {
    profile.comment(&format!("custom gcode: {name}"))
}

/// How the comment before custom G-code begins, as `profile` writes it.
pub fn custom_mark(profile: &Profile) -> String {
    format!("{}{}", profile.comment.0, profile.escaped("custom gcode"))
}

/// The comment custom G-code is written before.
pub fn custom_end(profile: &Profile) -> String {
    profile.comment("end custom gcode")
}

struct Writer<'a> {
    p: &'a Profile,
    out: String,
    report: RenderReport,
    line_no: u32,
    // What the machine has actually been told. `None` means "never written".
    motion: Option<&'static str>,
    at: Option<DVec3>,
    feed: Option<String>,
    // Whether the tool is known to be as high as the machine takes it, where it
    // is otherwise unknown: after a safe retract, until it next moves.
    raised: bool,
    // The plane arcs turn in (`G17`, `G18`, `G19`), once one has been written.
    plane: Option<&'static str>,
    // Whether lengths and feeds are written in inches: after a `G20`.
    inches: bool,
    // The tool whose length offset the next Z word takes up, after a tool
    // change, where the profile says to.
    offset: Option<u32>,
    // What the placeholders in the profile's lines stand for.
    values: Values,
    // The rotary's word as last written to it, on a setup wrapped round it.
    angle: Option<String>,
    // Whether feeds are in inverse time (`G93`).
    inverse: bool,
    // Whether it writes a Sinumerik's G-code ([`crate::sinumerik`]).
    siemens: bool,
    // On a Sinumerik, whether the tool's offsets are taken up (`D1`).
    edge: bool,
    // On a Sinumerik, the cycle called after each move (`MCALL`), as
    // written.
    mcall: Option<String>,
    // The height a run of holes starts from and returns to between them.
    clearance: Option<f64>,
}

impl<'a> Writer<'a> {
    fn new(p: &'a Profile) -> Self {
        Self {
            p,
            out: String::new(),
            report: RenderReport::default(),
            line_no: p.numbering.start,
            motion: None,
            at: None,
            feed: None,
            raised: false,
            plane: None,
            inches: false,
            offset: None,
            values: Values::default(),
            angle: None,
            inverse: false,
            siemens: p.dialect == Dialect::Sinumerik,
            edge: false,
            mcall: None,
            clearance: None,
        }
    }

    /// One of the profile's lines, or a custom one, its placeholders filled:
    /// words a person typed made safe to stand in a comment.
    fn filled(&mut self, line: &str) {
        let p = self.p;
        let text = fill(line, &self.values, &|t| p.escaped(t));
        self.raw(&text);
    }

    /// Select `plane` for arcs, unless it is in force.
    fn plane(&mut self, plane: &'static str) -> &'static str {
        if self.plane == Some(plane) {
            ""
        } else {
            self.plane = Some(plane);
            plane
        }
    }

    /// Back to the XY plane, if an arc left another in force: a canned cycle
    /// drills along the selected plane's normal, and the next program expects
    /// `G17`.
    fn level_plane(&mut self) {
        if matches!(self.plane, Some(p) if p != "G17") {
            self.raw("G17");
            self.plane = Some("G17");
        }
    }

    /// Forget where the machine is: something moved it, or may have.
    fn lost(&mut self) {
        self.motion = None;
        self.at = None;
        self.feed = None;
        self.raised = false;
        self.angle = None;
    }

    /// The profile's lines for a step, after which where the machine is is
    /// unknown.
    fn lines(&mut self, lines: &[String]) {
        for line in lines {
            self.filled(line);
        }
        if !lines.is_empty() {
            self.lost();
        }
        if self.siemens && lets_offsets_go(lines) {
            self.edge = false;
        }
    }

    fn raw(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // Blocks are numbered, comments not, as Fusion's are.
        if self.p.numbering.enabled && !text.starts_with(self.p.comment.0.as_str()) {
            let _ = write!(self.out, "N{} ", self.line_no);
            self.line_no = self.line_no.saturating_add(self.p.numbering.increment);
        }
        self.out.push_str(text);
        self.out.push('\n');
        self.report.lines += 1;
    }

    fn comment(&mut self, text: &str) {
        let c = self.p.comment(text);
        self.raw(&c);
    }

    fn op(&mut self, op: &Op) {
        match op {
            Op::SectionStart {
                op_name,
                tool,
                strategy,
                wcs,
            } => {
                let word = self.p.wcs_word(*wcs).unwrap_or_else(|| format!("G{wcs}"));
                self.comment(&format!("{op_name} - {strategy} - T{tool} {word}"));
            }
            Op::SectionEnd => {}
            Op::Units(u) => {
                // A Sinumerik's G710 and G700 set the feeds' units too.
                self.raw(match (u, self.siemens) {
                    (ProgramUnits::Mm, false) => "G21",
                    (ProgramUnits::Inch, false) => "G20",
                    (ProgramUnits::Mm, true) => "G710",
                    (ProgramUnits::Inch, true) => "G700",
                });
                self.inches = *u == ProgramUnits::Inch;
                // Changing units invalidates every remembered coordinate.
                self.at = None;
                self.feed = None;
            }
            Op::WorkOffset(n) => {
                // Lowering lets through only an offset the machine has.
                let s = self.p.wcs_word(*n).unwrap_or_else(|| format!("G{n}"));
                self.raw(&s);
                // Another offset puts the same point at other coordinates.
                self.at = None;
            }
            // G-code has no word for the stock.
            Op::Stock { .. } => {}
            Op::PreSelect(tool) => {
                // A Sinumerik with no changer puts a tool in by its T alone.
                let puts_in = self.siemens && self.p.tool_change == ToolChangeStyle::Pause;
                if self.p.preselect_tool && !puts_in {
                    let s = self.tool_word(*tool);
                    self.raw(&s);
                }
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
                self.raw("M1");
                // The operator may jog anywhere while it waits.
                self.lost();
            }
            Op::ToolChange { tool, comment } => {
                if self.p.length_offset && !self.siemens {
                    self.offset = Some(*tool);
                }
                self.tool_change(*tool, comment);
            }
            Op::SpindleOn { rpm, dir } => {
                let m = match dir {
                    SpindleDirection::Clockwise => "M3",
                    SpindleDirection::CounterClockwise => "M4",
                };
                let s = format!("S{rpm} {m}");
                self.raw(&s);
            }
            Op::SpindleOff => {
                self.level_plane();
                self.raw("M5");
            }
            Op::CoolantOn(c) => self.raw(match c {
                Coolant::Flood => "M8",
                Coolant::Mist => "M7",
                // Not emitted; asked for anyway, off is off.
                Coolant::Off => "M9",
            }),
            Op::CoolantOff => self.raw("M9"),
            Op::Turning {
                to, angle, feed, ..
            } => {
                let g = if feed.is_some() { "G1" } else { "G0" };
                self.turn_block(g, *to, angle, *feed);
            }
            Op::InverseTime(on) => {
                self.raw(if *on { "G93" } else { "G94" });
                self.inverse = *on;
                // An F means something else now: the next is written.
                self.feed = None;
            }
            Op::Rapid { to } => self.motion_block("G0", *to, None),
            Op::Feed { to, feed, .. } => {
                let f = self.p.feed_in(*feed, self.inches);
                self.motion_block("G1", *to, Some(f));
            }
            Op::Arc { to, arc, feed, .. } => self.arc(*to, arc, *feed),
            Op::Cycle {
                kind,
                at,
                depth,
                retract,
                peck,
                dwell,
                feed,
            } => {
                self.level_plane();
                if self.siemens {
                    let hole = sinumerik::Hole {
                        kind: *kind,
                        at: *at,
                        depth: *depth,
                        retract: *retract,
                        peck: *peck,
                        dwell: *dwell,
                        feed: *feed,
                    };
                    self.siemens_cycle(&hole);
                } else {
                    self.cycle(*kind, *at, *depth, *retract, *peck, *dwell, *feed);
                }
            }
            Op::RigidTap { depth, pitch } => {
                self.level_plane();
                let s = format!(
                    "G33.1 Z{} K{}",
                    self.p.length(*depth, self.inches),
                    self.p.pitch(*pitch, self.inches)
                );
                self.raw(&s);
                self.report.motion_blocks += 1;
                // It comes back to where it started, which is where the
                // machine was told it is; but its motion mode is its own,
                // so the next move says its own.
                self.motion = None;
                self.raised = false;
            }
            Op::RigidMode { rpm } => {
                let s = format!("M29 S{rpm}");
                self.raw(&s);
            }
            Op::CycleStart if self.siemens => {
                // Each cycle returns to where the run starts, its RTP.
                self.clearance = self.at.map(|a| a.z);
            }
            Op::CycleStart => {
                self.raw("G98");
                // The cycles move the machine: nothing is suppressed against
                // where it was.
                self.lost();
            }
            Op::CycleEnd if self.siemens => self.siemens_cycle_end(),
            Op::CycleEnd => self.raw("G80"),
            Op::Probe { to, feed } => {
                let f = self.p.feed_in(*feed, self.inches);
                // Taken to be where it was sent, its furthest: where it
                // touched is not known, but the next move leaves it
                // straight up, saying its Z whatever this was.
                self.motion_block("G38.2", *to, Some(f));
            }
            Op::ProbeOpen(name) => {
                let (open, close) = (&self.p.comment.0, &self.p.comment.1);
                let s = format!("{open}PROBEOPEN {name}{close}");
                self.raw(&s);
            }
            Op::ProbeClose => {
                let (open, close) = (&self.p.comment.0, &self.p.comment.1);
                let s = format!("{open}PROBECLOSE{close}");
                self.raw(&s);
            }
            Op::Raw(text) => {
                // Passed through untouched, by definition. Modal state is unknown
                // afterwards, so nothing may be suppressed against it.
                self.raw(text);
                self.lost();
            }
            Op::Comment(text) => self.comment(text),
            Op::Custom(custom) => {
                let begin = custom_begin(self.p, &custom.name);
                self.raw(&begin);
                for line in &custom.lines {
                    self.filled(line);
                }
                let end = custom_end(self.p);
                self.raw(&end);
                // It may have moved nothing, but what it did is not known.
                self.lost();
            }
            Op::Rotary(at) => {
                // The part turns under the tool: only with the tool clear.
                if !self.raised {
                    let p = self.p;
                    self.lines(&p.safe_retract);
                    self.raised = !p.safe_retract.is_empty();
                }
                let words: Vec<String> = at
                    .iter()
                    .map(|a| format!("{}{}", a.axis, self.p.axis(a.degrees)))
                    .collect();
                let s = format!("G0 {}", words.join(" "));
                self.raw(&s);
                self.motion = Some("G0");
                // Where the tool is in the turned work is not known; it is
                // still as high as it goes.
                self.at = None;
                self.angle = None;
            }
            Op::ProgramEnd => {}
        }
    }

    /// Change to `tool`, by changer or by hand, as the profile says: its own
    /// lines, where it gives them, or its style's.
    fn tool_change(&mut self, tool: u32, comment: &str) {
        self.change_tool(tool, comment);
        // A Sinumerik takes up the new tool's length and radius from its
        // first edge, where it does not by itself.
        if self.siemens {
            self.raw("D1");
            self.edge = true;
        }
    }

    fn change_tool(&mut self, tool: u32, comment: &str) {
        if !self.p.tool_change_lines.is_empty() {
            let p = self.p;
            for line in &p.tool_change_lines {
                self.filled(line);
            }
            // A macro or the operator may have moved anything, down included.
            self.lost();
            return;
        }
        match self.p.tool_change {
            ToolChangeStyle::M6 => {
                if !comment.is_empty() {
                    self.comment(comment);
                }
                let s = format!("{} M6", self.tool_word(tool));
                self.raw(&s);
                // A changer moves the machine; the tool is where the change
                // left it. Raised by a safe retract, it is still raised.
                self.at = None;
            }
            ToolChangeStyle::Pause => {
                let msg = if comment.is_empty() {
                    format!("fit tool {tool}")
                } else {
                    format!("fit tool {tool}: {comment}")
                };
                self.comment(&msg);
                let s = self.tool_word(tool);
                self.raw(&s);
                self.raw("M0");
                // The operator may have jogged anywhere, down included.
                self.lost();
            }
        }
    }

    /// Emit one motion block, suppressing only what the machine already has.
    ///
    /// A rapid from nowhere known goes across and down one at a time, never
    /// diagonally: across first when the tool is known to be raised, as high as
    /// it can be, and otherwise down (or up) to its height first, which is
    /// clear of everything since it is the height the path crosses at.
    fn motion_block(&mut self, g: &'static str, to: DVec3, feed: Option<String>) {
        self.offsets();
        if g == "G0" && self.at.is_none() {
            let (x, y, z) = (
                self.p.length(to.x, self.inches),
                self.p.length(to.y, self.inches),
                self.p.length(to.z, self.inches),
            );
            let again = if self.p.modal.suppress_motion {
                ""
            } else {
                "G0 "
            };
            let z = self.z_word(&z);
            let (first, second) = if self.raised {
                (format!("G0 X{x} Y{y}"), format!("{again}{z}"))
            } else {
                (format!("G0 {z}"), format!("{again}X{x} Y{y}"))
            };
            self.raw(&first);
            self.raw(&second);
            self.report.motion_blocks += 2;
            self.motion = Some(g);
            self.at = Some(to);
            self.raised = false;
            return;
        }
        self.raised = false;
        // Round first, then compare. Comparing unrounded values while emitting
        // rounded ones can suppress a word the machine would have seen change.
        let (x, y, z) = (
            self.p.length(to.x, self.inches),
            self.p.length(to.y, self.inches),
            self.p.length(to.z, self.inches),
        );
        let prev = self.at.map(|p| {
            (
                self.p.length(p.x, self.inches),
                self.p.length(p.y, self.inches),
                self.p.length(p.z, self.inches),
            )
        });

        let mut block = String::new();
        let motion_changed = self.motion != Some(g);
        if motion_changed || !self.p.modal.suppress_motion {
            block.push_str(g);
        }

        let push = |axis: char, val: &str, same: bool, block: &mut String| {
            if same && self.p.modal.suppress_axes {
                return;
            }
            if !block.is_empty() {
                block.push(' ');
            }
            let _ = write!(block, "{axis}{val}");
        };

        let same = |a: Option<&String>, b: &str| a.map(|s| s.as_str()) == Some(b);
        let z_same = same(prev.as_ref().map(|p| &p.2), &z);
        // A length offset waits for a Z word; after a tool change, the first
        // block states every axis, so it never waits long.
        let offset = if z_same && self.p.modal.suppress_axes {
            None
        } else {
            self.offset.take()
        };
        if offset.is_some() {
            if !block.is_empty() {
                block.push(' ');
            }
            block.push_str("G43");
        }
        push('X', &x, same(prev.as_ref().map(|p| &p.0), &x), &mut block);
        push('Y', &y, same(prev.as_ref().map(|p| &p.1), &y), &mut block);
        push('Z', &z, z_same, &mut block);
        if let Some(h) = offset {
            let _ = write!(block, " H{h}");
        }

        if let Some(f) = &feed {
            let unchanged = self.feed.as_deref() == Some(f.as_str());
            if !unchanged || !self.p.modal.suppress_feed {
                if !block.is_empty() {
                    block.push(' ');
                }
                let _ = write!(block, "F{f}");
            }
            self.feed = Some(f.clone());
        }

        // Every word suppressed means the machine is already where it is being told
        // to go. Emitting a bare motion word would be meaningless, so skip it.
        if block.is_empty() {
            self.at = Some(to);
            return;
        }

        self.raw(&block);
        self.report.motion_blocks += 1;
        self.motion = Some(g);
        self.at = Some(to);
    }

    /// One block moving the linear axes and a rotary at once, a setup
    /// wrapped round it, written as [`Writer::motion_block`] writes one: a
    /// rapid from nowhere known across and round first when the tool is
    /// raised, else down (or up) first. In inverse time every feed move
    /// says its `F`.
    fn turn_block(&mut self, g: &'static str, to: DVec3, angle: &RotaryAt, feed: Option<f64>) {
        self.offsets();
        let (x, y, z) = (
            self.p.length(to.x, self.inches),
            self.p.length(to.y, self.inches),
            self.p.length(to.z, self.inches),
        );
        let a = self.p.axis(angle.degrees);
        if g == "G0" && self.at.is_none() {
            let z = self.z_word(&z);
            let again = if self.p.modal.suppress_motion {
                ""
            } else {
                "G0 "
            };
            let across = format!("X{x} Y{y} {}{a}", angle.axis);
            let (first, second) = if self.raised {
                (format!("G0 {across}"), format!("{again}{z}"))
            } else {
                (format!("G0 {z}"), format!("{again}{across}"))
            };
            self.raw(&first);
            self.raw(&second);
            self.report.motion_blocks += 2;
            self.motion = Some(g);
            self.at = Some(to);
            self.angle = Some(a);
            self.raised = false;
            return;
        }
        self.raised = false;
        let prev = self.at.map(|p| {
            (
                self.p.length(p.x, self.inches),
                self.p.length(p.y, self.inches),
                self.p.length(p.z, self.inches),
            )
        });
        let suppress = self.p.modal.suppress_axes;
        let mut words: Vec<String> = Vec::new();
        if self.motion != Some(g) || !self.p.modal.suppress_motion {
            words.push(g.to_string());
        }
        let z_same = prev.as_ref().is_some_and(|p| p.2 == z);
        let offset = if z_same && suppress {
            None
        } else {
            self.offset.take()
        };
        if offset.is_some() {
            words.push("G43".into());
        }
        for (letter, v, before) in [
            ('X', &x, prev.as_ref().map(|p| &p.0)),
            ('Y', &y, prev.as_ref().map(|p| &p.1)),
            ('Z', &z, prev.as_ref().map(|p| &p.2)),
        ] {
            if !(suppress && before == Some(v)) {
                words.push(format!("{letter}{v}"));
            }
        }
        if !(suppress && self.angle.as_deref() == Some(a.as_str())) {
            words.push(format!("{}{a}", angle.axis));
        }
        if let Some(h) = offset {
            words.push(format!("H{h}"));
        }
        if let Some(f) = feed {
            // Worked out for the move, so to more places than a feed: a
            // time's four, a rate's two.
            let more = if self.inverse { 4 } else { 2 };
            let f = self
                .p
                .number(f, self.p.format.feed_decimals.saturating_add(more));
            let unchanged = self.feed.as_deref() == Some(f.as_str());
            if self.inverse || !unchanged || !self.p.modal.suppress_feed {
                words.push(format!("F{f}"));
            }
            self.feed = Some(f);
        }
        let moved = words
            .iter()
            .any(|w| !w.starts_with('G') && !w.starts_with('F'));
        self.at = Some(to);
        self.angle = Some(a);
        if !moved {
            return;
        }
        self.raw(&words.join(" "));
        self.report.motion_blocks += 1;
        self.motion = Some(g);
    }

    /// A Z word, taking up the tool's length offset with it when one is
    /// waiting: `G43 Z… H<tool>`, as Fusion writes it.
    fn z_word(&mut self, z: &str) -> String {
        match self.offset.take() {
            Some(h) => format!("G43 Z{z} H{h}"),
            None => format!("Z{z}"),
        }
    }

    /// An arc, or a helix, from where the tool is, in the plane it turns in,
    /// written in pieces no longer than the profile's arcs may be: a half turn
    /// with centre offsets, which cannot be misread even as a full circle, and a
    /// quarter with a radius, whose sign would otherwise have to choose the long
    /// way round. A piece too short across to be written as an arc is written
    /// as straight moves along it.
    fn arc(&mut self, to: DVec3, arc: &Arc, feed: f64) {
        let Some(from) = self.at else {
            // Nothing is known to start it from: straight to its end, rather than
            // round a centre measured from nowhere. The engine never does this.
            let f = self.p.feed_in(feed, self.inches);
            self.motion_block("G1", to, Some(f));
            return;
        };
        let most = match self.arc_style() {
            ArcStyle::Ij => PI,
            ArcStyle::R => FRAC_PI_2,
        };
        let pieces = (arc.sweep.abs() / most - 1e-9).ceil().max(1.0);
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
            let across = (end - at) - arc.axis * (end - at).dot(arc.axis);
            if across.length() < SHORTEST_CHORD {
                let f = self.p.feed_in(feed, self.inches);
                for p in piece.chords(at, end, STRAIGHTENED) {
                    self.motion_block("G1", p, Some(f.clone()));
                }
            } else {
                self.arc_block(at, end, &piece, feed);
            }
            at = end;
        }
    }

    /// One block of an arc no more than a half turn long.
    fn arc_block(&mut self, from: DVec3, to: DVec3, arc: &Arc, feed: f64) {
        self.offsets();
        // Which plane, its two axes, the axis along its normal, and whether the
        // arc turns counter-clockwise seen from that axis's positive end.
        let a = arc.axis;
        let (plane, axes, normal, ccw) = if a.z.abs() >= a.x.abs().max(a.y.abs()) {
            (
                "G17",
                [('X', 'I', 0), ('Y', 'J', 1)],
                ('Z', 2),
                arc.sweep * a.z > 0.0,
            )
        } else if a.y.abs() >= a.x.abs() {
            (
                "G18",
                [('X', 'I', 0), ('Z', 'K', 2)],
                ('Y', 1),
                arc.sweep * a.y > 0.0,
            )
        } else {
            (
                "G19",
                [('Y', 'J', 1), ('Z', 'K', 2)],
                ('X', 0),
                arc.sweep * a.x > 0.0,
            )
        };
        let g = if ccw { "G3" } else { "G2" };
        let centre = arc.centre + a * (from - arc.centre).dot(a);
        let mut block = String::new();
        let plane = self.plane(plane);
        if !plane.is_empty() {
            block.push_str(plane);
            block.push(' ');
        }
        block.push_str(g);
        // Both of the plane's axes, always, as Fusion writes them; the third only
        // where it moves, which makes a helix.
        for (axis, _, i) in axes {
            let _ = write!(block, " {axis}{}", self.p.length(to[i], self.inches));
        }
        let (axis, i) = normal;
        if self.p.length(to[i], self.inches) != self.p.length(from[i], self.inches) {
            let _ = write!(block, " {axis}{}", self.p.length(to[i], self.inches));
        }
        match self.arc_style() {
            ArcStyle::Ij => {
                for (_, word, i) in axes {
                    let _ = write!(
                        block,
                        " {word}{}",
                        self.p.length(centre[i] - from[i], self.inches)
                    );
                }
            }
            ArcStyle::R => {
                let _ = write!(block, " R{}", self.p.length(arc.radius(from), self.inches));
            }
        }
        let f = self.p.feed_in(feed, self.inches);
        if self.feed.as_deref() != Some(f.as_str()) || !self.p.modal.suppress_feed {
            let _ = write!(block, " F{f}");
        }
        self.raw(&block);
        self.report.motion_blocks += 1;
        self.motion = Some(g);
        self.at = Some(to);
        self.feed = Some(f);
        self.raised = false;
    }

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
        let g = match kind {
            CycleKind::Drill => "G81",
            CycleKind::DrillDwell => "G82",
            CycleKind::Peck => "G83",
            CycleKind::ChipBreak => "G73",
            CycleKind::Tap => "G84",
            CycleKind::TapLeft => "G74",
        };
        // Every word every time: a cycle is modal, but a program read by a person
        // should say where each hole is and how deep, whatever came before. In
        // Fusion's order: where, then a peck or a dwell, then the feed.
        let mut block = format!(
            "{g} X{} Y{} Z{} R{}",
            self.p.length(at.x, self.inches),
            self.p.length(at.y, self.inches),
            self.p.length(depth, self.inches),
            self.p.length(retract, self.inches),
        );
        if let Some(q) = peck {
            let _ = write!(block, " Q{}", self.p.length(q, self.inches));
        }
        if let Some(p) = dwell {
            // Seconds, whatever the units.
            let _ = write!(block, " P{}", self.p.dwell(p));
        }
        let f = if matches!(kind, CycleKind::Tap | CycleKind::TapLeft) {
            // A tap's feed is its pitch a turn, so the spindle's speed times
            // the pitch: rounded to a feed's places it would gain or lose on
            // the thread every turn, so it keeps three more.
            self.p.feed_in_finely(feed, self.inches)
        } else {
            self.p.feed_in(feed, self.inches)
        };
        let _ = write!(block, " F{f}");
        self.raw(&block);
        // A cycle leaves the machine in its own modal group and moves the tool.
        self.lost();
    }
}

/// Whether `lines` let the tool's offsets go, a Sinumerik's `D0`.
fn lets_offsets_go(lines: &[String]) -> bool {
    lines.iter().any(|l| {
        crate::custom::words(l)
            .iter()
            .any(|(c, v, _)| *c == 'D' && *v == Some(0.0))
    })
}

mod sinumerik;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests;
