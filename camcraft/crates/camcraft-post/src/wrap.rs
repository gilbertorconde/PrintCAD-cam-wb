//! A setup wrapped round a rotary: its paths, in the bar unrolled, as moves
//! of the machine's linear axes and the rotary at once.
//!
//! The unrolled frame is along the bar, round it (millimetres at the wrap
//! radius) and out from its axis (`camcraft_core::setup::frame::Unrolled`).
//! On the machine the work zero is on the rotary's axis: along it is X for
//! a bar lying along X, turned by A (Y for one along Y, by B), the other
//! linear axis stays at zero, over the axis, and Z is how far out. The
//! rotary's angle is the distance round over the radius, turned so the
//! point is under the tool: an A of `sign` times it, a B of minus that, as
//! a positive B turns the part from Z towards X ([`crate::rotary`]).
//!
//! A controller moves X, Z and the rotary each straight from one end of a
//! block to the other, which is straight in the unrolled frame, so a path
//! straight there is written as it is. Its feed is not: a controller reads
//! `F` along X and Z alone, or with the degrees as lengths, so each move's
//! is written as its profile says ([`RotaryFeed`]): in inverse time, one
//! over the minutes the move takes cutting at its feed along the material,
//! where the tool's tip goes, or in units a minute that take as long.

use camcraft_core::path::toolpath::{MoveKind, MoveRole, Toolpath};
use glam::DVec3;

use crate::op::{LowerError, Op};
use crate::profile::{Profile, ProgramUnits};
use crate::rotary::{About, RotaryAt, RotaryFeed};

/// Millimetres in an inch.
const MM_PER_INCH: f64 = 25.4;

/// How a setup's unrolled paths are posted round a rotary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wrapping {
    /// The rotary's word.
    pub axis: char,
    /// The machine axis it turns about, which the bar lies along.
    pub about: About,
    /// Which way a positive angle turns the part: the profile's sign.
    pub sign: f64,
    /// The radius the paths are unrolled at, in millimetres.
    pub radius: f64,
    /// How its moves are fed.
    pub feed: RotaryFeed,
    /// The lowest and highest angle it may go to, in degrees; `None` for one
    /// that turns on without end.
    pub limits: Option<[f64; 2]>,
}

impl Wrapping {
    /// The rotary of `profile` that turns a bar lying along `about`, for
    /// paths unrolled at `radius`; refused for a machine without one.
    pub fn of(profile: &Profile, about: About, radius: f64) -> Result<Self, LowerError> {
        let refused = || LowerError::NoRotaryFor {
            machine: profile.name.clone(),
            about,
        };
        let rotary = profile.rotary.as_ref().ok_or_else(refused)?;
        let axis = rotary
            .axes
            .iter()
            .copied()
            .find(|&n| rotary.about(n) == Some(about))
            .ok_or_else(refused)?;
        let table = rotary.axis(axis).unwrap_or_default();
        Ok(Self {
            axis,
            about,
            sign: f64::from(table.sign),
            radius,
            feed: rotary.feed,
            limits: (!table.continuous).then_some(table.limits),
        })
    }

    /// Along the bar, round it, and out from its axis, of an unrolled
    /// point.
    fn split(&self, q: DVec3) -> (f64, f64, f64) {
        match self.about {
            About::X => (q.x, q.y, q.z),
            About::Y => (q.y, q.x, q.z),
        }
    }

    /// Where the machine goes for unrolled point `q`: X, Y and Z, and the
    /// rotary's angle in degrees.
    pub fn machine(&self, q: DVec3) -> (DVec3, f64) {
        let (along, round, out) = self.split(q);
        let turn = (round / self.radius).to_degrees();
        match self.about {
            About::X => (DVec3::new(along, 0.0, out), self.sign * turn),
            About::Y => (DVec3::new(0.0, along, out), -self.sign * turn),
        }
    }

    /// The unrolled point the machine is at, at `at` with the rotary at
    /// `degrees`: [`Wrapping::machine`] undone.
    pub fn unrolled(&self, at: DVec3, degrees: f64) -> DVec3 {
        let turn = match self.about {
            About::X => self.sign * degrees,
            About::Y => -self.sign * degrees,
        };
        let round = turn.to_radians() * self.radius;
        match self.about {
            About::X => DVec3::new(at.x, round, at.z),
            About::Y => DVec3::new(round, at.y, at.z),
        }
    }

    /// How far the tool's tip goes along the material from unrolled `a` to
    /// `b`: round it at the tip's distance out, not at the radius.
    pub fn length(&self, a: DVec3, b: DVec3) -> f64 {
        let (a0, r0, z0) = self.split(a);
        let (a1, r1, z1) = self.split(b);
        let out = 0.5 * (z0 + z1).max(0.0);
        let round = (r1 - r0) / self.radius * out;
        ((a1 - a0).powi(2) + round * round + (z1 - z0).powi(2)).sqrt()
    }

    /// The `F` a move from unrolled `from` to `to` at `feed` mm/min along
    /// the material is written with, in a program in `units`; `None` for a
    /// move that goes nowhere.
    pub fn feed_word(&self, from: DVec3, to: DVec3, feed: f64, units: ProgramUnits) -> Option<f64> {
        let length = self.length(from, to);
        if length <= 1e-9 || feed <= 0.0 {
            return None;
        }
        let minutes = length / feed;
        Some(match self.feed {
            RotaryFeed::InverseTime => 1.0 / minutes,
            RotaryFeed::Degrees => {
                let scale = match units {
                    ProgramUnits::Mm => 1.0,
                    ProgramUnits::Inch => 1.0 / MM_PER_INCH,
                };
                let ((m0, d0), (m1, d1)) = (self.machine(from), self.machine(to));
                let linear = (m1 - m0) * scale;
                (linear.length_squared() + (d1 - d0).powi(2)).sqrt() / minutes
            }
        })
    }

    /// The feed in mm/min along the material a move from unrolled `from` to
    /// `to` written with `f` takes: [`Wrapping::feed_word`] undone.
    pub fn feed_of(&self, from: DVec3, to: DVec3, f: f64, units: ProgramUnits) -> Option<f64> {
        let unit = self.feed_word(from, to, 1.0, units)?;
        Some(f / unit)
    }
}

/// `path`'s moves round the rotary, as instructions: in inverse time, where
/// the rotary is fed so, from the first to the last. An arc is written as
/// straight moves along it, within `tolerance`. `made` as for
/// [`crate::op::lower_counted`].
pub(crate) fn moves(
    path: &Toolpath,
    w: &Wrapping,
    units: ProgramUnits,
    out: &mut Vec<Op>,
    made: &mut Vec<usize>,
) {
    let inverse = w.feed == RotaryFeed::InverseTime;
    if inverse {
        out.push(Op::InverseTime(true));
        made.push(made.last().copied().unwrap_or(0));
    }
    let mut at: Option<DVec3> = None;
    for (i, m) in path.moves().iter().enumerate() {
        let ends = match (m.kind, at) {
            (MoveKind::Arc(_), Some(from)) => m.chords(from, 0.001),
            _ => vec![m.to],
        };
        let last = ends.len().saturating_sub(1);
        for (j, to) in ends.into_iter().enumerate() {
            let (machine, degrees) = w.machine(to);
            let angle = RotaryAt {
                axis: w.axis,
                degrees,
            };
            let op = match (m.role, m.feed, at) {
                (MoveRole::Rapid, _, _) => Some(Op::Turning {
                    to: machine,
                    angle,
                    feed: None,
                    role: m.role,
                }),
                (role, Some(feed), Some(from)) => {
                    w.feed_word(from, to, feed, units).map(|f| Op::Turning {
                        to: machine,
                        angle,
                        feed: Some(f),
                        role,
                    })
                }
                // A feed from where the tool is not known, as the first
                // move, goes there at rapid: never at an `F` worked out from
                // nowhere.
                (_, _, None) => Some(Op::Turning {
                    to: machine,
                    angle,
                    feed: None,
                    role: MoveRole::Rapid,
                }),
                (_, None, _) => None,
            };
            if let Some(op) = op {
                out.push(op);
                made.push(if j == last { i + 1 } else { i });
            }
            at = Some(to);
        }
        at = Some(m.to);
    }
    if inverse {
        out.push(Op::InverseTime(false));
        made.push(path.len());
    }
}

/// `path` moved round by whole turns of `period` so it begins within half a
/// turn of `near`, a distance round: the same cuts, where the rotary takes
/// them, by the shorter way there.
pub fn wound(path: &Toolpath, about: About, period: f64, near: f64) -> Toolpath {
    let Some(first) = path.moves().first() else {
        return path.clone();
    };
    let at = match about {
        About::X => first.to.y,
        About::Y => first.to.x,
    };
    let turns = ((at - near) / period).round();
    if turns == 0.0 {
        return path.clone();
    }
    let shift = match about {
        About::X => DVec3::Y,
        About::Y => DVec3::X,
    } * (-turns * period);
    path.shifted(shift)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;
    use glam::dvec3;

    fn a() -> Wrapping {
        Wrapping {
            axis: 'A',
            about: About::X,
            sign: 1.0,
            radius: 20.0,
            feed: RotaryFeed::InverseTime,
            limits: None,
        }
    }

    /// A quarter of the way round the bar is a quarter turn of A, and back.
    #[test]
    fn round_the_bar_is_the_rotarys_angle() {
        let w = a();
        let q = dvec3(5.0, std::f64::consts::PI * 10.0, 19.0);
        let (at, deg) = w.machine(q);
        assert_eq!(at, dvec3(5.0, 0.0, 19.0));
        assert!((deg - 90.0).abs() < 1e-12);
        assert!((w.unrolled(at, deg) - q).length() < 1e-12);
        // The other way round on a table turning the other way.
        let w = Wrapping { sign: -1.0, ..a() };
        assert!((w.machine(q).1 + 90.0).abs() < 1e-12);
        // B turns the part from Z towards X: minus the angle round.
        let w = Wrapping {
            axis: 'B',
            about: About::Y,
            ..a()
        };
        let (at, deg) = w.machine(dvec3(std::f64::consts::PI * 10.0, 5.0, 19.0));
        assert_eq!(at, dvec3(0.0, 5.0, 19.0));
        assert!((deg + 90.0).abs() < 1e-12);
    }

    /// Inverse time is one over the minutes the move takes along the
    /// material at the tip: round the bar at 15 out, not at the radius.
    #[test]
    fn a_move_round_is_fed_in_inverse_time_at_the_tip() {
        let w = a();
        // A quarter turn at 15 out: a quarter of 30π round.
        let (from, to) = (
            dvec3(0.0, 0.0, 15.0),
            dvec3(0.0, 10.0 * std::f64::consts::PI, 15.0),
        );
        let length = 7.5 * std::f64::consts::PI;
        assert!((w.length(from, to) - length).abs() < 1e-9);
        let f = w.feed_word(from, to, 600.0, ProgramUnits::Mm).unwrap();
        assert!((f - 600.0 / length).abs() < 1e-9, "{f}");
        assert!((w.feed_of(from, to, f, ProgramUnits::Mm).unwrap() - 600.0).abs() < 1e-9);
        // In degrees a minute: 90 degrees in the minutes it takes.
        let w = Wrapping {
            feed: RotaryFeed::Degrees,
            ..a()
        };
        let f = w.feed_word(from, to, 600.0, ProgramUnits::Mm).unwrap();
        assert!((f - 90.0 / (length / 600.0)).abs() < 1e-9, "{f}");
        // Along X alone it is the feed.
        let f = w
            .feed_word(
                dvec3(0.0, 3.0, 15.0),
                dvec3(10.0, 3.0, 15.0),
                600.0,
                ProgramUnits::Mm,
            )
            .unwrap();
        assert!((f - 600.0).abs() < 1e-9);
    }

    #[test]
    fn a_path_is_wound_to_start_near() {
        let mut p = Toolpath::new();
        let period = 40.0 * std::f64::consts::PI;
        p.rapid_to(dvec3(0.0, 3.0 * period + 1.0, 30.0)).unwrap();
        let q = wound(&p, About::X, period, 0.0);
        assert!((q.moves()[0].to.y - 1.0).abs() < 1e-9);
        let q = wound(&p, About::X, period, period);
        assert!((q.moves()[0].to.y - period - 1.0).abs() < 1e-9);
    }
}
