//! A whole setup as one program, as Fusion posts one: one header, each
//! operation in order, and one footer.
//!
//! Between two operations the tool is changed only where it changes. The
//! same tool keeps turning, at the next operation's speed if that is
//! another, and the coolant changes only where it does; the tool goes up
//! clear between them all the same, as the profile takes it up, or, with no
//! way of its own, straight up as high as either operation goes, across,
//! and down to where the next begins. Custom G-code between two operations
//! is written once the spindle and coolant are stopped and the tool is
//! clear, and the next operation starts again from nothing after it, its
//! tool changed even when it is the same, as a program of its own would.
//! Where the profile asks for it, the program stops between every two
//! operations for the operator (`M1`), the spindle stopped first. The
//! setup's stock and its work offset are written once, and after each tool
//! change the next operation's other tool is called up, where the profile
//! does that.

use camcraft_core::job::Coolant;
use camcraft_core::path::toolpath::Toolpath;

use crate::op::{LowerError, Op, ProgramSpec, cycles, moves, refuse, rigid_taps, tilt_notes};
use crate::profile::{Profile, TapStyle};

/// One operation of a setup's program: its path, and everything else a
/// program of it alone would need.
#[derive(Debug, Clone, Copy)]
pub struct Section<'a> {
    /// Its linked toolpath.
    pub path: &'a Toolpath,
    /// Its tool, speeds, custom G-code and the rest.
    pub spec: &'a ProgramSpec,
}

/// How a machine goes from one operation to the next.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Joining {
    /// Whether it stops for the operator between them (`M1`).
    pub optional_stop: bool,
    /// Whether the profile takes the tool up clear itself (`safe_retract`).
    pub retracts: bool,
}

impl Joining {
    /// How `profile`'s machine goes from one operation to the next.
    pub fn of(profile: &Profile) -> Self {
        Self {
            optional_stop: profile.optional_stop,
            retracts: !profile.safe_retract.is_empty(),
        }
    }
}

/// An operation of a setup that cannot be posted, and why.
#[derive(Debug, PartialEq, thiserror::Error)]
#[error("{error}")]
pub struct SectionError {
    /// Which, from zero, among the sections.
    pub section: usize,
    /// Why.
    pub error: LowerError,
}

/// For each instruction of a setup's program, which section it belongs to
/// and how many of that section's moves are made once it has run.
pub type Made = Vec<(usize, usize)>;

/// The instruction stream of one program for all of `sections`, in order,
/// and for each instruction which section it is of and how many of that
/// section's moves are made once it has run ([`crate::op::lower_counted`]
/// says it of one). Refused, saying which section, where any one would be
/// refused alone.
pub fn lower_setup(sections: &[Section], join: Joining) -> Result<(Vec<Op>, Made), SectionError> {
    for (i, s) in sections.iter().enumerate() {
        refuse(s.path, s.spec).map_err(|error| SectionError { section: i, error })?;
    }
    let mut w = Stream::default();
    let Some(first) = sections.first() else {
        return Ok((w.ops, w.made));
    };
    w.push(Op::Units(first.spec.units));
    // The setup's stock, once, for a control that draws it.
    if let Some((low, high)) = first.spec.stock {
        w.push(Op::Stock { low, high });
    }
    for (i, s) in sections.iter().enumerate() {
        let spec = s.spec;
        let before = i.checked_sub(1).and_then(|b| sections.get(b));
        let next = sections.get(i + 1);
        w.section = i;
        w.count = 0;
        // Written once the one before has stopped the spindle and is clear;
        // it may have turned the coolant on.
        w.extend(spec.before.iter().cloned().map(Op::Custom));
        if !spec.before.is_empty() {
            w.push(Op::CoolantOff);
        }
        w.push(Op::SectionStart {
            op_name: spec.op_name.clone(),
            tool: spec.tool,
            strategy: spec.strategy.clone(),
            wcs: spec.wcs,
        });
        match before {
            Some(b) if !restarts(b.spec, spec, join) => {
                // The same tool, still turning: at this one's speed and with
                // its coolant.
                if (b.spec.rpm, b.spec.spin) != (spec.rpm, spec.spin) {
                    if b.spec.spin != spec.spin {
                        w.push(Op::SpindleOff);
                    }
                    w.push(Op::SpindleOn {
                        rpm: spec.rpm,
                        dir: spec.spin,
                    });
                }
                if b.spec.coolant != spec.coolant {
                    if b.spec.coolant != Coolant::Off {
                        w.push(Op::CoolantOff);
                    }
                    if spec.coolant != Coolant::Off {
                        w.push(Op::CoolantOn(spec.coolant));
                    }
                }
            }
            _ => {
                if before.is_none() {
                    w.extend(tilt_notes(spec).into_iter().map(Op::Comment));
                }
                // The one before stopped and went up clear already, unless
                // custom G-code came between.
                if before.is_none() || !spec.before.is_empty() {
                    w.push(Op::SpindleOff);
                    w.push(Op::SafeRetract);
                }
                w.push(Op::ToolChange {
                    tool: spec.tool,
                    comment: spec.tool_comment.clone(),
                });
                // The tool the next change takes, called up ready.
                if let Some(t) = sections
                    .get(i + 1..)
                    .unwrap_or_default()
                    .iter()
                    .map(|n| n.spec.tool)
                    .find(|t| *t != spec.tool)
                {
                    w.push(Op::PreSelect(t));
                }
                // One setup, one work offset: written once, and again only
                // where custom G-code may have changed it.
                let custom =
                    !spec.before.is_empty() || before.is_some_and(|b| !b.spec.after.is_empty());
                if before.is_none_or(|b| custom || b.spec.wcs != spec.wcs) {
                    w.push(Op::WorkOffset(spec.wcs));
                }
                // The part turned once, before the first cut: every
                // operation of a setup stands it the same way.
                if before.is_none() && !spec.rotary.is_empty() {
                    w.push(Op::Rotary(spec.rotary.clone()));
                }
                w.push(Op::SpindleOn {
                    rpm: spec.rpm,
                    dir: spec.spin,
                });
                if spec.coolant != Coolant::Off {
                    w.push(Op::CoolantOn(spec.coolant));
                }
            }
        }

        w.body(s);

        match next {
            Some(n) if !restarts(spec, n.spec, join) => w.link(s, n, join),
            _ => {
                if spec.coolant != Coolant::Off {
                    w.push(Op::CoolantOff);
                }
                w.push(Op::SpindleOff);
                w.push(Op::SafeRetract);
                if next.is_none() {
                    w.push(Op::Home);
                } else if join.optional_stop {
                    w.push(Op::OptionalStop);
                }
                // At the end, or after this one: it may have started the
                // spindle or the coolant, and both are stopped again.
                w.extend(spec.after.iter().cloned().map(Op::Custom));
                if !spec.after.is_empty() {
                    w.push(Op::SpindleOff);
                    w.push(Op::CoolantOff);
                }
            }
        }
        w.push(Op::SectionEnd);
    }
    w.push(Op::ProgramEnd);
    Ok((w.ops, w.made))
}

/// Whether the operation of `next` starts again from nothing after that of
/// `spec`: a tool changed, an optional stop, or custom G-code between them,
/// after which nothing about the machine is known.
fn restarts(spec: &ProgramSpec, next: &ProgramSpec, join: Joining) -> bool {
    spec.tool != next.tool
        || join.optional_stop
        || !spec.after.is_empty()
        || !next.before.is_empty()
}

/// A program's instructions as they are made, each with its section and its
/// moves made.
#[derive(Default)]
struct Stream {
    ops: Vec<Op>,
    made: Made,
    section: usize,
    count: usize,
}

impl Stream {
    fn push(&mut self, op: Op) {
        self.ops.push(op);
        self.made.push((self.section, self.count));
    }

    fn extend(&mut self, ops: impl IntoIterator<Item = Op>) {
        for op in ops {
            self.push(op);
        }
    }

    /// A section's own moves, or its holes' cycles.
    fn body(&mut self, s: &Section) {
        let (mut ops, mut made) = (Vec::new(), Vec::new());
        // Each operation's taps in its post's form, as a program of its
        // own writes them: rigid mode (`M29`) ends with its run of cycles,
        // so each operation's taps take it again.
        match &s.spec.drill {
            Some(d) if d.tap.is_some() && d.tap_style == TapStyle::G331 => {
                rigid_taps(d, s.path, &mut ops, &mut made);
            }
            Some(d) => cycles(d, s.spec.rpm, s.path, &mut ops, &mut made),
            None => moves(s.path, &mut ops, &mut made),
        }
        for (op, n) in ops.into_iter().zip(made) {
            self.count = n;
            self.push(op);
        }
        self.count = s.path.len();
    }

    /// From the end of `s` to above where `n` begins, the same tool turning:
    /// up clear as the profile goes, or else straight up as high as either
    /// goes, and across there, so `n`'s first move goes only down.
    fn link(&mut self, s: &Section, n: &Section, join: Joining) {
        if join.retracts {
            self.push(Op::SafeRetract);
            return;
        }
        let (Some(from), Some(to)) = (s.path.end(), n.path.moves().first().map(|m| m.to)) else {
            return;
        };
        let high = crate::op::extents(s.path).map_or(from.z, |(_, h)| h.z);
        let z = high.max(from.z).max(to.z);
        if from.z < z {
            self.push(Op::Rapid {
                to: from.truncate().extend(z),
            });
        }
        if from.truncate() != to.truncate() {
            self.push(Op::Rapid {
                to: to.truncate().extend(z),
            });
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use camcraft_core::job::SpindleDirection;
    use camcraft_core::path::toolpath::MoveRole;
    use glam::dvec3;

    use super::*;
    use crate::op::lower;

    fn path(x: f64) -> Toolpath {
        let mut t = Toolpath::new();
        t.rapid_to(dvec3(x, 0.0, 10.0)).unwrap();
        t.feed_to(dvec3(x, 0.0, 0.0), MoveRole::Plunge, 300.0)
            .unwrap();
        t.feed_to(dvec3(x + 10.0, 0.0, 0.0), MoveRole::Cut, 1200.0)
            .unwrap();
        t.feed_to(dvec3(x + 10.0, 0.0, 5.0), MoveRole::Retract, 900.0)
            .unwrap();
        t
    }

    fn spec(name: &str, tool: u32, rpm: u32) -> ProgramSpec {
        ProgramSpec {
            op_name: name.into(),
            tool,
            rpm,
            ..ProgramSpec::default()
        }
    }

    fn count(ops: &[Op], f: impl Fn(&Op) -> bool) -> usize {
        ops.iter().filter(|o| f(o)).count()
    }

    /// Three operations on two tools: one units, one end, a section each,
    /// two tool changes, and the second tool's speed changed without
    /// stopping it.
    #[test]
    fn a_tool_is_changed_only_where_it_changes() {
        let (a, b, c) = (path(0.0), path(20.0), path(40.0));
        let specs = [spec("rough", 1, 18000), spec("finish", 2, 18000), {
            spec("chamfer", 2, 12000)
        }];
        let sections: Vec<Section> = [&a, &b, &c]
            .into_iter()
            .zip(&specs)
            .map(|(path, spec)| Section { path, spec })
            .collect();
        let join = Joining {
            optional_stop: false,
            retracts: true,
        };
        let (ops, made) = lower_setup(&sections, join).unwrap();
        assert_eq!(ops.len(), made.len());
        assert_eq!(count(&ops, |o| matches!(o, Op::Units(_))), 1);
        assert_eq!(count(&ops, |o| *o == Op::ProgramEnd), 1);
        assert_eq!(count(&ops, |o| *o == Op::Home), 1);
        assert_eq!(count(&ops, |o| matches!(o, Op::SectionStart { .. })), 3);
        let changes: Vec<u32> = ops
            .iter()
            .filter_map(|o| match o {
                Op::ToolChange { tool, .. } => Some(*tool),
                _ => None,
            })
            .collect();
        assert_eq!(changes, [1, 2]);
        // Tool 2 called up after tool 1 goes in; after it, none other.
        let called: Vec<&Op> = ops
            .iter()
            .filter(|o| matches!(o, Op::PreSelect(_)))
            .collect();
        assert_eq!(called, [&Op::PreSelect(2)]);
        // One setup, one work offset.
        assert_eq!(count(&ops, |o| matches!(o, Op::WorkOffset(54))), 1);
        let speeds: Vec<u32> = ops
            .iter()
            .filter_map(|o| match o {
                Op::SpindleOn { rpm, .. } => Some(*rpm),
                _ => None,
            })
            .collect();
        assert_eq!(speeds, [18000, 18000, 12000]);
        // Stopped before each change and at the end, not between the two
        // operations of tool 2.
        assert_eq!(count(&ops, |o| *o == Op::SpindleOff), 3);
        // Up clear between every two, and at the start and the end.
        assert_eq!(count(&ops, |o| *o == Op::SafeRetract), 4);
        assert!(!ops.contains(&Op::OptionalStop));
    }

    /// Each instruction says whose moves it makes, in order: every move of
    /// every section counted once, by its own section.
    #[test]
    fn each_instruction_counts_its_sections_moves() {
        let (a, b) = (path(0.0), path(20.0));
        let specs = [spec("a", 1, 18000), spec("b", 1, 18000)];
        let sections = [
            Section {
                path: &a,
                spec: &specs[0],
            },
            Section {
                path: &b,
                spec: &specs[1],
            },
        ];
        let (ops, made) = lower_setup(&sections, Joining::default()).unwrap();
        assert!(made.windows(2).all(|w| w[0] <= w[1]), "{made:?}");
        let motion: Vec<(usize, usize)> = ops
            .iter()
            .zip(&made)
            .filter(|(o, _)| matches!(o, Op::Feed { .. }))
            .map(|(_, m)| *m)
            .collect();
        assert_eq!(motion, [(0, 2), (0, 3), (0, 4), (1, 2), (1, 3), (1, 4)]);
        assert_eq!(made.last(), Some(&(1, 4)));
        // With no way up of its own, straight up and across above both,
        // so the next starts straight down.
        let start = ops
            .iter()
            .position(|o| matches!(o, Op::SectionStart { op_name, .. } if op_name == "b"))
            .unwrap();
        let links: Vec<&Op> = ops[..start]
            .iter()
            .rev()
            .take_while(|o| matches!(o, Op::Rapid { .. } | Op::SectionEnd))
            .collect();
        assert_eq!(
            links,
            [
                &Op::SectionEnd,
                &Op::Rapid {
                    to: dvec3(20.0, 0.0, 10.0)
                },
                &Op::Rapid {
                    to: dvec3(10.0, 0.0, 10.0)
                },
            ]
        );
    }

    /// Asked for, the program stops between every two operations, the
    /// spindle stopped first, and starts the same tool again after.
    #[test]
    fn an_optional_stop_comes_between_every_two() {
        let (a, b) = (path(0.0), path(20.0));
        let specs = [spec("a", 1, 18000), spec("b", 1, 18000)];
        let sections = [
            Section {
                path: &a,
                spec: &specs[0],
            },
            Section {
                path: &b,
                spec: &specs[1],
            },
        ];
        let join = Joining {
            optional_stop: true,
            retracts: true,
        };
        let (ops, _) = lower_setup(&sections, join).unwrap();
        let stop = ops.iter().position(|o| *o == Op::OptionalStop).unwrap();
        assert_eq!(ops[stop - 2], Op::SpindleOff);
        assert_eq!(ops[stop - 1], Op::SafeRetract);
        assert_eq!(count(&ops, |o| *o == Op::OptionalStop), 1);
        assert_eq!(count(&ops, |o| matches!(o, Op::ToolChange { .. })), 2);
    }

    /// One section is the program [`lower`] makes of it alone.
    #[test]
    fn one_section_is_its_own_program() {
        let a = path(0.0);
        let spec = ProgramSpec {
            coolant: Coolant::Flood,
            spin: SpindleDirection::CounterClockwise,
            ..spec("a", 4, 9000)
        };
        let (ops, _) = lower_setup(
            &[Section {
                path: &a,
                spec: &spec,
            }],
            Joining::default(),
        )
        .unwrap();
        assert_eq!(ops, lower(&a, &spec).unwrap());
    }

    /// A section that cannot be posted is named.
    #[test]
    fn a_section_refused_is_named() {
        let a = path(0.0);
        let good = spec("a", 1, 18000);
        let bad = ProgramSpec {
            wcs: 99,
            ..good.clone()
        };
        let e = lower_setup(
            &[
                Section {
                    path: &a,
                    spec: &good,
                },
                Section {
                    path: &a,
                    spec: &bad,
                },
            ],
            Joining::default(),
        )
        .unwrap_err();
        assert_eq!(e.section, 1);
        assert_eq!(e.error, LowerError::BadWorkOffset(99));
    }
}
