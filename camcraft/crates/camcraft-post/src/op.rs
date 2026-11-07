//! The machine-independent instruction stream.
//!
//! `camcraft-core` produces a [`Toolpath`]; this crate turns it into a sequence of
//! [`Op`], and a profile renders that into one dialect. The engine never sees this
//! type and never sees G-code — the crate graph makes that structural rather than a
//! matter of discipline.
//!
//! # Structure, not just moves
//!
//! The stream carries section boundaries naming the operation, tool, strategy and
//! work offset. Posts need those to write headers, tool-change blocks and setup
//! sheets; a flat list of coordinates leaves every post guessing.
//!
//! # Arcs
//!
//! An arc of the path is an [`Op::Arc`], in whichever of the three planes it
//! turns in. The engine only makes arcs a three-axis machine can cut
//! ([`principal`]), and one that is not is refused rather than guessed at.

use camcraft_core::job::ThreadHand;
use camcraft_core::job::{Coolant, SpindleDirection};
use camcraft_core::path::toolpath::{Arc, MoveKind, MoveRole, Toolpath, principal};
use camcraft_core::strategy::drilling::{Hole, Tap};
use glam::{DVec2, DVec3, dvec3};

use crate::profile::ProgramUnits;
use crate::rotary::{RotaryAt, RotaryError, angles, short};

/// A canned drilling cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleKind {
    /// Simple drill (`G81`).
    Drill,
    /// Drill with a pause at the bottom (`G82`).
    DrillDwell,
    /// Peck drill (`G83`).
    Peck,
    /// Peck drill, backing off only a little between pecks to break the chip
    /// (`G73`).
    ChipBreak,
    /// Tap a right-hand thread, the spindle reversing at the bottom (`G84`).
    Tap,
    /// Tap a left-hand thread, the spindle reversing at the bottom (`G74`).
    TapLeft,
}

/// One machine-independent instruction.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// Beginning of one operation's output.
    SectionStart {
        /// Human name of the operation.
        op_name: String,
        /// Tool number.
        tool: u32,
        /// Strategy that produced it.
        strategy: String,
        /// Work offset in force, 54 for `G54`.
        wcs: u8,
    },
    /// End of the current operation's output.
    SectionEnd,
    /// Set the program units.
    Units(ProgramUnits),
    /// Select a work offset, 54 for `G54`.
    WorkOffset(u8),
    /// The stock, a box from its lowest corner to its highest in the
    /// program's coordinates, in millimetres: for a control that draws the
    /// part as it cuts it (Heidenhain's `BLK FORM`). Nothing moves.
    Stock {
        /// The lowest corner.
        low: DVec3,
        /// The highest corner.
        high: DVec3,
    },
    /// Change to a tool.
    ToolChange {
        /// Tool number.
        tool: u32,
        /// Description for the operator.
        comment: String,
    },
    /// Call up the tool the next program changes to, so the changer has it
    /// ready, where the profile does (`preselect_tool`).
    PreSelect(u32),
    /// Start the spindle.
    SpindleOn {
        /// Speed in rev/min.
        rpm: u32,
        /// Direction: `M3` clockwise, `M4` counter-clockwise.
        dir: SpindleDirection,
    },
    /// Stop the spindle.
    SpindleOff,
    /// Turn coolant on: `M8` flood, `M7` mist. [`lower`] never emits it with
    /// [`Coolant::Off`].
    CoolantOn(Coolant),
    /// Turn coolant off.
    CoolantOff,
    /// Traverse at maximum speed.
    Rapid {
        /// Target position.
        to: DVec3,
    },
    /// Move at a feed rate.
    Feed {
        /// Target position.
        to: DVec3,
        /// Feed in mm/min.
        feed: f64,
        /// Why this move exists, so a profile can vary its output by role.
        role: MoveRole,
    },
    /// Circular interpolation, from where the tool is: an arc, or a helix,
    /// about X, Y or Z.
    Arc {
        /// Target position.
        to: DVec3,
        /// The arc.
        arc: Arc,
        /// Feed in mm/min.
        feed: f64,
        /// Why this move exists.
        role: MoveRole,
    },
    /// Canned cycle: one hole.
    Cycle {
        /// Which cycle.
        kind: CycleKind,
        /// Where the hole is.
        at: DVec2,
        /// Final depth.
        depth: f64,
        /// Retract height.
        retract: f64,
        /// Peck increment, where the cycle uses one.
        peck: Option<f64>,
        /// Seconds to pause at the bottom, where the cycle does.
        dwell: Option<f64>,
        /// Feed in mm/min.
        feed: f64,
    },
    /// Start of a run of canned cycles, each returning to the height it
    /// started from (`G98`).
    CycleStart,
    /// End of a run of canned cycles (`G80`).
    CycleEnd,
    /// Text passed through untouched, for manual NC and probe macros.
    Raw(String),
    /// A Custom G-code operation's lines, written as typed with their
    /// placeholders filled, between two comments that say where they begin
    /// and end. Where the machine is afterwards is not known.
    Custom(CustomGcode),
    /// A comment for the operator, in the profile's way of writing one.
    Comment(String),
    /// Turn the rotaries to where they stand the setup's tool axis up the
    /// spindle, with the tool clear: positioning only, before it cuts.
    Rotary(Vec<RotaryAt>),
    /// Take the tool straight up clear of everything, as the profile does it.
    SafeRetract,
    /// Send the machine home across, as the profile does it.
    Home,
    /// End of program.
    ProgramEnd,
}

/// A Custom G-code operation's text, for the program it is written into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomGcode {
    /// The operation's name.
    pub name: String,
    /// Its lines, as typed: their placeholders are filled as they are
    /// written.
    pub lines: Vec<String>,
}

impl CustomGcode {
    /// A Custom G-code operation called `name` whose text is `text`.
    pub fn new(name: &str, text: &str) -> Self {
        Self {
            name: name.to_string(),
            lines: text.lines().map(|l| l.trim_end().to_string()).collect(),
        }
    }
}

/// Everything a program needs that a toolpath does not carry.
#[derive(Debug, Clone, PartialEq)]
pub struct ProgramSpec {
    /// Operation name.
    pub op_name: String,
    /// Strategy that produced the path.
    pub strategy: String,
    /// Tool number.
    pub tool: u32,
    /// Tool description for the operator.
    pub tool_comment: String,
    /// Work offset, 54 for `G54`; from 60, the machine's extended ones.
    pub wcs: u8,
    /// How many work offsets past `G59` the machine has: its profile's
    /// `extended_offsets`.
    pub extended_offsets: u8,
    /// Spindle speed.
    pub rpm: u32,
    /// Spindle direction.
    pub spin: SpindleDirection,
    /// Coolant; [`Coolant::Off`] leaves it off.
    pub coolant: Coolant,
    /// Program units.
    pub units: ProgramUnits,
    /// The holes of a drilling operation, to write as canned cycles instead of
    /// its moves; only for a profile that takes them.
    pub drill: Option<DrillCycles>,
    /// Whether the path taps threads. A tap must go one pitch a turn all the
    /// way, as the spindle slows, stops and reverses at the bottom, and only a
    /// controller's tapping cycle keeps the feed to the spindle through that:
    /// as moves, the axis stops while the spindle still turns, and the thread
    /// is stripped or the tap snapped. So it is posted only in [`drill`].
    ///
    /// [`drill`]: Self::drill
    pub taps: bool,
    /// The setup's tool axis, a unit vector in the model, when it is not the
    /// model's +Z: the program is in the setup's tilted frame, and says so.
    pub tilt: Option<DVec3>,
    /// Where the machine's rotaries go before it cuts: empty for a machine
    /// without them, whose part is fixtured tilted by hand.
    pub rotary: Vec<RotaryAt>,
    /// The tool the program after this one changes to, if one does: called
    /// up after the change where the profile does (`preselect_tool`).
    pub next_tool: Option<u32>,
    /// The stock, its lowest and highest corners in the program's
    /// coordinates, in millimetres, where it is known: for a control that
    /// draws the part as it cuts it.
    pub stock: Option<(DVec3, DVec3)>,
    /// Custom G-code written before the operation's own start: the Custom
    /// G-code operations just before it in its setup.
    pub before: Vec<CustomGcode>,
    /// Custom G-code written after the operation ends, before the footer:
    /// the Custom G-code operations after the last operation of its setup.
    pub after: Vec<CustomGcode>,
}

/// A drilling operation's holes, as canned cycles.
#[derive(Debug, Clone, PartialEq)]
pub struct DrillCycles {
    /// The holes, in the order drilled.
    pub holes: Vec<Hole>,
    /// Depth of each peck (`G83`), if it pecks.
    pub peck: Option<f64>,
    /// How far it backs off between pecks to break the chip (`G73`), if it
    /// does rather than backing out. The controller backs off as far as it is
    /// set to: the cycle has no word for it.
    pub chip_break: Option<f64>,
    /// Seconds to pause at the bottom (`G82`), if it does and does not peck.
    pub dwell: Option<f64>,
    /// The thread, if it taps (`G84`, `G74` left-hand); then it does not peck.
    pub tap: Option<Tap>,
    /// Feed down into each hole, in mm/min: a tap's, its pitch a turn.
    pub feed: f64,
    /// The height the tool crosses between holes at, and returns to (`G98`).
    pub clearance: f64,
}

impl Default for ProgramSpec {
    fn default() -> Self {
        Self {
            op_name: "operation".into(),
            strategy: "parallel".into(),
            tool: 1,
            tool_comment: String::new(),
            wcs: 54,
            extended_offsets: 0,
            rpm: 18000,
            spin: SpindleDirection::Clockwise,
            coolant: Coolant::Off,
            units: ProgramUnits::Mm,
            drill: None,
            taps: false,
            tilt: None,
            rotary: Vec::new(),
            next_tool: None,
            stock: None,
            before: Vec::new(),
            after: Vec::new(),
        }
    }
}

/// A toolpath that cannot be turned into a program.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum LowerError {
    /// The path still contains moves the linker was supposed to resolve.
    ///
    /// Refused rather than rendered: a link is "get here somehow", and a post that
    /// guessed would be inventing a traverse nobody planned.
    #[error("toolpath still contains {0} unresolved link(s); run the linker first")]
    UnresolvedLinks(usize),
    /// A work offset the machine has not got.
    #[error(
        "work offset {0} is not one the machine has: 54 to 59 for G54 to G59, then its \
         extended ones"
    )]
    BadWorkOffset(u8),
    /// An arc turns about something other than X, Y or Z, which no three-axis
    /// machine can cut as an arc.
    #[error("the arc to {0:?} is not in the XY, XZ or YZ plane")]
    ArcOutOfPlane(DVec3),
    /// A path that taps, for a controller without the tapping cycle, which is
    /// the only way to keep the feed to the spindle as it reverses at the
    /// bottom ([`ProgramSpec::taps`]).
    #[error(
        "tapping is posted only as the controller's tapping cycle (G84, G74), which keeps \
         the feed to the spindle as it reverses: as plain moves the tap would strip its \
         thread or snap. Post it with a profile that has drill_cycles = true"
    )]
    TapsAsMoves,
    /// A tap whose spindle turns against its thread.
    #[error("a {0:?}-hand tap goes in with the spindle turning the other way")]
    TapTurnsAgainstThread(ThreadHand),
    /// The machine's rotaries cannot stand the setup's tool axis up the
    /// spindle.
    #[error(transparent)]
    Rotary(#[from] RotaryError),
}

/// Turn a linked toolpath into an instruction stream.
///
/// Refuses a path containing unresolved links. That check lives here rather than in
/// a post because every post would otherwise have to repeat it, and one that forgot
/// would emit a program with a hole in it.
pub fn lower(path: &Toolpath, spec: &ProgramSpec) -> Result<Vec<Op>, LowerError> {
    lower_counted(path, spec).map(|(ops, _)| ops)
}

/// [`lower`], and for each instruction how many of the path's moves are made
/// once it has run: so a line of the program can be found from a move of the
/// path, and a move from a line, as a listing beside a playback needs.
///
/// A canned cycle stands for the moves that drill its hole, so it counts as
/// far as the last of them; the rapid across to the first hole counts none.
pub fn lower_counted(
    path: &Toolpath,
    spec: &ProgramSpec,
) -> Result<(Vec<Op>, Vec<usize>), LowerError> {
    let unresolved = path
        .moves()
        .iter()
        .filter(|m| m.role.is_unplanned())
        .count();
    if unresolved > 0 {
        return Err(LowerError::UnresolvedLinks(unresolved));
    }
    if !(54..=59u16 + u16::from(spec.extended_offsets)).contains(&u16::from(spec.wcs)) {
        return Err(LowerError::BadWorkOffset(spec.wcs));
    }
    if let Some(m) = path
        .moves()
        .iter()
        .find(|m| matches!(m.kind, MoveKind::Arc(a) if !principal(a.axis)))
    {
        return Err(LowerError::ArcOutOfPlane(m.to));
    }
    let tap = spec.drill.as_ref().and_then(|d| d.tap);
    if spec.taps && tap.is_none() {
        return Err(LowerError::TapsAsMoves);
    }
    if let Some(t) = tap
        && t.hand.spindle() != spec.spin
    {
        return Err(LowerError::TapTurnsAgainstThread(t.hand));
    }

    // The profile header owns the program name via `{name}`; repeating it here
    // would print it twice in every file.
    let mut out = Vec::with_capacity(path.len() + 16);
    out.push(Op::Units(spec.units));
    if let Some((low, high)) = spec.stock {
        out.push(Op::Stock { low, high });
    }
    // Custom G-code may have turned the coolant on; the operation's start
    // stops the spindle itself.
    out.extend(spec.before.iter().cloned().map(Op::Custom));
    if !spec.before.is_empty() {
        out.push(Op::CoolantOff);
    }
    out.push(Op::SectionStart {
        op_name: spec.op_name.clone(),
        tool: spec.tool,
        strategy: spec.strategy.clone(),
        wcs: spec.wcs,
    });
    out.extend(tilt_notes(spec).into_iter().map(Op::Comment));
    // The spindle is stopped and the tool raised before any tool change, whatever
    // ran before this program.
    out.push(Op::SpindleOff);
    out.push(Op::SafeRetract);
    out.push(Op::ToolChange {
        tool: spec.tool,
        comment: spec.tool_comment.clone(),
    });
    if let Some(next) = spec.next_tool.filter(|n| *n != spec.tool) {
        out.push(Op::PreSelect(next));
    }
    out.push(Op::WorkOffset(spec.wcs));
    if !spec.rotary.is_empty() {
        out.push(Op::Rotary(spec.rotary.clone()));
    }
    out.push(Op::SpindleOn {
        rpm: spec.rpm,
        dir: spec.spin,
    });
    let coolant = spec.coolant != Coolant::Off;
    if coolant {
        out.push(Op::CoolantOn(spec.coolant));
    }
    let mut made = vec![0; out.len()];

    if let Some(d) = &spec.drill {
        cycles(d, path, &mut out, &mut made);
    } else {
        moves(path, &mut out, &mut made);
    }

    if coolant {
        out.push(Op::CoolantOff);
    }
    out.push(Op::SpindleOff);
    out.push(Op::SafeRetract);
    out.push(Op::Home);
    // And at the end, it may have started the spindle or the coolant: both
    // are stopped again before the program ends.
    out.extend(spec.after.iter().cloned().map(Op::Custom));
    if !spec.after.is_empty() {
        out.push(Op::SpindleOff);
        out.push(Op::CoolantOff);
    }
    out.push(Op::SectionEnd);
    out.push(Op::ProgramEnd);
    made.resize(out.len(), path.len());
    Ok((out, made))
}

/// What the program's header says of a tilted setup: which way its tool
/// axis is, how it is stood up the spindle, and that the coordinates are in
/// the tilted frame. Nothing for one up the model's Z.
fn tilt_notes(spec: &ProgramSpec) -> Vec<String> {
    let Some(axis) = spec.tilt else {
        return Vec::new();
    };
    let degrees = axis.z.clamp(-1.0, 1.0).acos().to_degrees();
    let mut out = vec![format!(
        "tool axis {} {} {} of the model, tilted {} deg from its Z",
        short(axis.x),
        short(axis.y),
        short(axis.z),
        short(degrees)
    )];
    if spec.rotary.is_empty() {
        out.push("fixture the part tilted so this axis points up the spindle".into());
    } else {
        out.push(format!(
            "{} stands it up the spindle; positioning only",
            angles(&spec.rotary)
        ));
    }
    out.push("coordinates are in the tilted frame: set the work offset in it".into());
    out.push("or use the controller's tilted-plane feature where it has one".into());
    out
}

/// Each hole as one canned cycle: across at the clearance height to the first,
/// then each cycle returning there (`G98`) before the next, and `G80` after.
/// `made` gets, for each, how many of `path`'s moves it stands for: those
/// down to its hole and back out, found in order.
fn cycles(d: &DrillCycles, path: &Toolpath, out: &mut Vec<Op>, made: &mut Vec<usize>) {
    let Some(first) = d.holes.first() else {
        return;
    };
    out.push(Op::Rapid {
        to: dvec3(first.at.x, first.at.y, d.clearance),
    });
    out.push(Op::CycleStart);
    let mut count = made.last().copied().unwrap_or(0);
    made.resize(out.len(), count);
    let kind = match (d.tap, d.peck, d.chip_break, d.dwell) {
        (Some(t), ..) => match t.hand {
            ThreadHand::Right => CycleKind::Tap,
            ThreadHand::Left => CycleKind::TapLeft,
        },
        (None, Some(_), Some(_), _) => CycleKind::ChipBreak,
        (None, Some(_), None, _) => CycleKind::Peck,
        (None, None, _, Some(_)) => CycleKind::DrillDwell,
        (None, None, _, None) => CycleKind::Drill,
    };
    let moves = path.moves();
    let over = |i: usize, at: DVec2| {
        moves
            .get(i)
            .is_some_and(|m| m.to.truncate().distance(at) < 1e-6)
    };
    for h in &d.holes {
        // The first move over the hole after the last hole's, then on while
        // the moves stay over it.
        if let Some(i) = (count..moves.len()).find(|&i| over(i, h.at)) {
            let mut end = i;
            while over(end + 1, h.at) {
                end += 1;
            }
            count = end + 1;
        }
        out.push(Op::Cycle {
            kind,
            at: h.at,
            depth: h.bottom,
            retract: h.retract(),
            peck: if matches!(kind, CycleKind::Peck | CycleKind::ChipBreak) {
                d.peck
            } else {
                None
            },
            dwell: if kind == CycleKind::DrillDwell {
                d.dwell
            } else {
                None
            },
            feed: d.feed,
        });
        made.push(count);
    }
    out.push(Op::CycleEnd);
    made.push(count);
}

/// The lowest and highest the tool's tip goes along `path`, an arc's bulge
/// included, in its coordinates; `None` for a path with no moves.
pub fn extents(path: &Toolpath) -> Option<(DVec3, DVec3)> {
    let first = path.moves().first()?.to;
    let (mut low, mut high) = (first, first);
    let mut at = first;
    for m in path.moves() {
        for p in m.chords(at, 0.001) {
            low = low.min(p);
            high = high.max(p);
        }
        at = m.to;
    }
    Some((low, high))
}

fn moves(path: &Toolpath, out: &mut Vec<Op>, made: &mut Vec<usize>) {
    for (i, m) in path.moves().iter().enumerate() {
        let op = match (m.kind, m.role, m.feed) {
            (MoveKind::Linear, MoveRole::Rapid, _) => Op::Rapid { to: m.to },
            (MoveKind::Linear, role, Some(feed)) => Op::Feed {
                to: m.to,
                feed,
                role,
            },
            (MoveKind::Arc(arc), role, Some(feed)) => Op::Arc {
                to: m.to,
                arc,
                feed,
                role,
            },
            // A feeding move with no feed cannot be constructed: Toolpath::push
            // rejects it. Skipping is unreachable, and silently emitting a rapid
            // here would be the dangerous choice.
            _ => continue,
        };
        out.push(op);
        made.push(i + 1);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp, clippy::panic)]
mod tests {
    use super::*;
    use glam::dvec3;

    fn linked_path() -> Toolpath {
        let mut t = Toolpath::new();
        t.rapid_to(dvec3(0.0, 0.0, 10.0)).unwrap();
        t.feed_to(dvec3(0.0, 0.0, 0.0), MoveRole::Plunge, 300.0)
            .unwrap();
        t.feed_to(dvec3(10.0, 0.0, 0.0), MoveRole::Cut, 1200.0)
            .unwrap();
        t
    }

    #[test]
    fn wraps_the_path_in_a_program() {
        let ops = lower(&linked_path(), &ProgramSpec::default()).unwrap();
        assert_eq!(ops.first(), Some(&Op::Units(ProgramUnits::Mm)));
        assert!(ops.contains(&Op::WorkOffset(54)));
        assert!(ops.contains(&Op::SpindleOff));
        assert_eq!(ops.last(), Some(&Op::ProgramEnd));
        assert!(matches!(
            ops.iter().find(|o| matches!(o, Op::SectionStart { .. })),
            Some(Op::SectionStart {
                tool: 1,
                wcs: 54,
                ..
            })
        ));
    }

    /// A link is "get here somehow". A post that rendered one would be inventing a
    /// traverse nobody planned, so lowering refuses instead.
    #[test]
    fn refuses_a_path_with_unresolved_links() {
        let mut t = linked_path();
        t.feed_to(dvec3(5.0, 5.0, 0.0), MoveRole::Link, 100.0)
            .unwrap();
        assert_eq!(
            lower(&t, &ProgramSpec::default()),
            Err(LowerError::UnresolvedLinks(1))
        );
    }

    #[test]
    fn rejects_an_impossible_work_offset() {
        let spec = ProgramSpec {
            wcs: 99,
            ..Default::default()
        };
        assert_eq!(
            lower(&linked_path(), &spec),
            Err(LowerError::BadWorkOffset(99))
        );
        // Past G59, only as many as the machine has.
        let g59_1 = ProgramSpec {
            wcs: 60,
            ..Default::default()
        };
        assert_eq!(
            lower(&linked_path(), &g59_1),
            Err(LowerError::BadWorkOffset(60))
        );
        let ops = lower(
            &linked_path(),
            &ProgramSpec {
                extended_offsets: 3,
                ..g59_1
            },
        )
        .unwrap();
        assert!(ops.contains(&Op::WorkOffset(60)));
    }

    #[test]
    fn moves_keep_their_role_and_feed() {
        let ops = lower(&linked_path(), &ProgramSpec::default()).unwrap();
        let feeds: Vec<_> = ops
            .iter()
            .filter_map(|o| match o {
                Op::Feed { role, feed, .. } => Some((*role, *feed)),
                _ => None,
            })
            .collect();
        assert_eq!(
            feeds,
            vec![(MoveRole::Plunge, 300.0), (MoveRole::Cut, 1200.0)]
        );
        assert_eq!(
            ops.iter().filter(|o| matches!(o, Op::Rapid { .. })).count(),
            1
        );
    }

    #[test]
    fn each_instruction_counts_the_moves_made() {
        let path = linked_path();
        let (ops, made) = lower_counted(&path, &ProgramSpec::default()).unwrap();
        assert_eq!(ops.len(), made.len());
        let motion: Vec<usize> = ops
            .iter()
            .zip(&made)
            .filter(|(o, _)| matches!(o, Op::Rapid { .. } | Op::Feed { .. }))
            .map(|(_, n)| *n)
            .collect();
        assert_eq!(motion, vec![1, 2, 3], "one a move, in order");
        assert_eq!(made.first(), Some(&0), "nothing before the first");
        assert_eq!(made.last(), Some(&path.len()), "all of them after");
        assert!(made.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn a_cycle_counts_the_moves_that_drill_its_hole() {
        let mut path = Toolpath::new();
        for x in [0.0, 10.0] {
            path.rapid_to(dvec3(x, 0.0, 5.0)).unwrap();
            path.feed_to(dvec3(x, 0.0, -3.0), MoveRole::Plunge, 100.0)
                .unwrap();
            path.feed_to(dvec3(x, 0.0, 5.0), MoveRole::Retract, 300.0)
                .unwrap();
        }
        let hole = |x: f64| Hole {
            at: DVec2::new(x, 0.0),
            top: 0.0,
            bottom: -3.0,
            radius: 2.0,
        };
        let spec = ProgramSpec {
            drill: Some(DrillCycles {
                holes: vec![hole(0.0), hole(10.0)],
                peck: None,
                chip_break: None,
                dwell: None,
                tap: None,
                feed: 100.0,
                clearance: 5.0,
            }),
            ..ProgramSpec::default()
        };
        let (ops, made) = lower_counted(&path, &spec).unwrap();
        let cycles: Vec<usize> = ops
            .iter()
            .zip(&made)
            .filter(|(o, _)| matches!(o, Op::Cycle { .. }))
            .map(|(_, n)| *n)
            .collect();
        assert_eq!(cycles, vec![3, 6]);
    }

    /// Holes drilled by the engine, each way it drills them, linked across at
    /// the clearance height as the linker links them: the path, and the
    /// cycles that stand for it.
    fn drilled(
        peck: Option<f64>,
        chip_break: Option<f64>,
        tap: Option<Tap>,
    ) -> (Toolpath, ProgramSpec) {
        use camcraft_core::strategy::drilling::{DrillParams, drill};
        let hole = |x: f64| Hole {
            at: DVec2::new(x, 0.0),
            top: 0.0,
            bottom: -7.0,
            radius: 2.0,
        };
        let holes = [hole(0.0), hole(10.0), hole(25.0)];
        let params = DrillParams {
            peck,
            chip_break,
            feed: 150.0,
            retract_feed: 600.0,
            tap,
        };
        let drilled = drill(&holes, &params).unwrap();
        let mut path = Toolpath::new();
        for m in drilled.moves() {
            if m.role == MoveRole::Link {
                path.rapid_to(dvec3(m.to.x, m.to.y, 15.0)).unwrap();
                path.rapid_to(m.to).unwrap();
            } else {
                path.feed_to(m.to, m.role, m.feed.unwrap()).unwrap();
            }
        }
        let spec = ProgramSpec {
            drill: Some(DrillCycles {
                holes: holes.to_vec(),
                peck,
                chip_break,
                dwell: None,
                tap,
                feed: 150.0,
                clearance: 15.0,
            }),
            taps: tap.is_some(),
            spin: tap.map_or(SpindleDirection::Clockwise, |t| t.hand.spindle()),
            ..ProgramSpec::default()
        };
        (path, spec)
    }

    /// Whatever the cycle, each counts the moves down its hole and back out:
    /// the last of them out of each hole, and all the path's by the end.
    #[test]
    fn every_cycle_counts_the_moves_of_its_hole() {
        let right = Tap {
            pitch: 1.25,
            hand: ThreadHand::Right,
        };
        let left = Tap {
            hand: ThreadHand::Left,
            ..right
        };
        for (peck, chip_break, tap, want) in [
            (None, None, None, CycleKind::Drill),
            (Some(2.0), None, None, CycleKind::Peck),
            (Some(2.0), Some(0.3), None, CycleKind::ChipBreak),
            (None, None, Some(right), CycleKind::Tap),
            (None, None, Some(left), CycleKind::TapLeft),
        ] {
            let (path, spec) = drilled(peck, chip_break, tap);
            let (ops, made) = lower_counted(&path, &spec).unwrap();
            let cycles: Vec<(CycleKind, usize)> = ops
                .iter()
                .zip(&made)
                .filter_map(|(o, n)| match o {
                    Op::Cycle { kind, .. } => Some((*kind, *n)),
                    _ => None,
                })
                .collect();
            assert!(cycles.iter().all(|(k, _)| *k == want), "{cycles:?}");
            // Each hole's last move is its way out, up to its R plane.
            let outs: Vec<usize> = path
                .moves()
                .iter()
                .enumerate()
                .filter(|(i, m)| {
                    m.role == MoveRole::Retract
                        && path
                            .moves()
                            .get(i + 1)
                            .is_none_or(|n| n.role == MoveRole::Rapid)
                })
                .map(|(i, _)| i + 1)
                .collect();
            let counts: Vec<usize> = cycles.iter().map(|(_, n)| *n).collect();
            assert_eq!(counts, outs, "{want:?}");
            assert_eq!(counts.last(), Some(&path.len()), "{want:?}");
        }
    }

    /// Tapping is refused as moves, and with the spindle turning against
    /// the thread.
    #[test]
    fn a_tap_is_posted_only_as_its_cycle_and_its_way_round() {
        let tap = Tap {
            pitch: 1.0,
            hand: ThreadHand::Left,
        };
        let (path, spec) = drilled(None, None, Some(tap));
        assert!(lower(&path, &spec).is_ok());
        let moves = ProgramSpec {
            drill: None,
            ..spec.clone()
        };
        assert_eq!(lower(&path, &moves), Err(LowerError::TapsAsMoves));
        let backwards = ProgramSpec {
            spin: SpindleDirection::Clockwise,
            ..spec
        };
        assert_eq!(
            lower(&path, &backwards),
            Err(LowerError::TapTurnsAgainstThread(ThreadHand::Left))
        );
    }

    /// A path's extents take in an arc's bulge, not only its ends.
    #[test]
    fn extents_take_in_an_arcs_bulge() {
        use camcraft_core::path::toolpath::Arc;
        let mut t = linked_path();
        // Half a turn clockwise from (10, 0) round (5, 0) to (0, 0), out to
        // y = -5.
        t.arc_to(
            dvec3(0.0, 0.0, 0.0),
            Arc {
                centre: dvec3(5.0, 0.0, 0.0),
                axis: DVec3::Z,
                sweep: -std::f64::consts::PI,
            },
            MoveRole::Cut,
            1200.0,
        )
        .unwrap();
        let (low, high) = extents(&t).unwrap();
        assert!((low.y + 5.0).abs() < 0.01, "{low}");
        assert_eq!((low.x, high.x, low.z, high.z), (0.0, 10.0, 0.0, 10.0));
        assert_eq!(extents(&Toolpath::new()), None);
    }

    #[test]
    fn coolant_is_paired() {
        let spec = ProgramSpec {
            coolant: Coolant::Flood,
            ..Default::default()
        };
        let ops = lower(&linked_path(), &spec).unwrap();
        assert!(ops.contains(&Op::CoolantOn(Coolant::Flood)));
        assert!(ops.contains(&Op::CoolantOff));

        let dry = lower(&linked_path(), &ProgramSpec::default()).unwrap();
        assert!(!dry.iter().any(|o| matches!(o, Op::CoolantOn(_))));
        assert!(!dry.contains(&Op::CoolantOff));
    }
}
