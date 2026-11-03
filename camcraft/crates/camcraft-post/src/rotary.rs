//! Rotary axes, for 3+2 machining: turning the part so a setup's tool axis
//! points up the spindle, then cutting it with X, Y and Z alone.
//!
//! A profile names its rotaries from the machine's base out: the first carries
//! the second, which carries the part. Each turns about the machine's X or Y.
//! With every rotary at zero the part sits as modelled, its X, Y and Z the
//! machine's. An angle `θ` on a rotary with `sign` `s` turns the part by `s·θ`
//! right-handed about the machine axis: counter-clockwise, seen from the
//! axis's plus end. A table whose positive angle turns the other way has
//! `sign = -1`.
//!
//! Positioning only: the rotaries go to where they tilt the tool axis up the
//! spindle before the operation cuts, and stay there while it does. Nothing
//! moves on more than three axes at once.

use glam::{DQuat, DVec3};
use serde::Deserialize;

/// The machine axis a rotary turns about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum About {
    /// The machine's X.
    X,
    /// The machine's Y.
    Y,
}

impl About {
    fn vector(self) -> DVec3 {
        match self {
            About::X => DVec3::X,
            About::Y => DVec3::Y,
        }
    }
}

/// One rotary axis.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotaryAxis {
    /// The machine axis it turns about: X for A and Y for B when not said.
    #[serde(default)]
    pub about: Option<About>,
    /// Which way a positive angle turns the part: 1 right-handed about the
    /// machine axis, -1 the other way.
    #[serde(default = "plus")]
    pub sign: i8,
    /// The lowest and highest angle it goes to, in degrees: a full turn
    /// either way when not said.
    #[serde(default = "full_turn")]
    pub limits: [f64; 2],
}

fn plus() -> i8 {
    1
}

fn full_turn() -> [f64; 2] {
    [-360.0, 360.0]
}

impl Default for RotaryAxis {
    fn default() -> Self {
        Self {
            about: None,
            sign: plus(),
            limits: full_turn(),
        }
    }
}

/// A machine's rotary axes: `[rotary]` in a profile, `axes = ["A"]` or
/// `["A", "B"]`, each with a table of its own (`[rotary.A]`) when it is not
/// as [`RotaryAxis`] says by default.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rotary {
    /// Their words, from the machine's base out.
    pub axes: Vec<char>,
    /// A's, when it has one.
    #[serde(rename = "A", default)]
    pub a: Option<RotaryAxis>,
    /// B's.
    #[serde(rename = "B", default)]
    pub b: Option<RotaryAxis>,
    /// C's.
    #[serde(rename = "C", default)]
    pub c: Option<RotaryAxis>,
}

/// Where one rotary goes: its word and its angle, in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RotaryAt {
    /// `A`, `B` or `C`.
    pub axis: char,
    /// Degrees.
    pub degrees: f64,
}

/// A tool axis the rotaries cannot bring up the spindle, as asked.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RotaryError {
    /// No angle of theirs points it up.
    #[error(
        "the rotaries ({axes}) cannot turn the tool axis ({}, {}, {}) of the model up the \
         spindle: it is not square to {about}",
        short(axis.x), short(axis.y), short(axis.z)
    )]
    CannotReach {
        /// The rotaries, as words.
        axes: String,
        /// The machine axes they turn about.
        about: String,
        /// The tool axis, in the model.
        axis: DVec3,
    },
    /// It takes an angle past a rotary's limits.
    #[error(
        "the tool axis takes {} past the machine's limits ({})",
        angles(at),
        limits
    )]
    OutOfLimits {
        /// Where they would have to go.
        at: Vec<RotaryAt>,
        /// Each one's limits, written out.
        limits: String,
    },
    /// Turned up, the setup's X does not lie along the machine's, which a
    /// work offset cannot turn.
    #[error(
        "with the rotaries at {}, the setup's X lies {off:.3}° round from the machine's: \
         set the setup's machine X along ({}, {}, {}) of the model",
        angles(at), short(want.x), short(want.y), short(want.z)
    )]
    Twisted {
        /// Where the rotaries go.
        at: Vec<RotaryAt>,
        /// How far round, in degrees.
        off: f64,
        /// The model direction that would lie along the machine's X.
        want: DVec3,
    },
}

/// A number to four places, its trailing zeros dropped.
pub(crate) fn short(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s == "-0" { "0".into() } else { s }
}

/// Angles written as words: `A30 B-15`.
pub fn angles(at: &[RotaryAt]) -> String {
    at.iter()
        .map(|a| format!("{}{}", a.axis, short(a.degrees)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// How close to the machine's Z the tool axis must come, and X to its X.
const SQUARE: f64 = 1e-6;

/// Rotaries may differ from the limits by this, in degrees, for rounding.
const SLACK: f64 = 1e-9;

impl Rotary {
    /// A rotary's settings, by its word.
    pub fn axis(&self, name: char) -> Option<RotaryAxis> {
        let given = match name {
            'A' => &self.a,
            'B' => &self.b,
            'C' => &self.c,
            _ => return None,
        };
        Some(given.clone().unwrap_or_default())
    }

    /// The machine axis a rotary turns about.
    fn about(&self, name: char) -> Option<About> {
        self.axis(name)?.about.or(match name {
            'A' => Some(About::X),
            'B' => Some(About::Y),
            _ => None,
        })
    }

    /// What is wrong with it as a profile has it, if anything.
    pub fn problem(&self) -> Option<String> {
        if self.axes.is_empty() || self.axes.len() > 2 {
            return Some("names one rotary or two".into());
        }
        if self.axes.first() == self.axes.get(1) {
            return Some("names the same rotary twice".into());
        }
        for (name, table) in [('A', &self.a), ('B', &self.b), ('C', &self.c)] {
            if table.is_some() && !self.axes.contains(&name) {
                return Some(format!("has a table for {name}, which is not in axes"));
            }
        }
        let mut abouts = Vec::new();
        for &name in &self.axes {
            let Some(axis) = self.axis(name) else {
                return Some(format!("{name:?} is not a rotary: A, B or C"));
            };
            let Some(about) = self.about(name) else {
                return Some(format!("{name}'s about must say x or y"));
            };
            if abouts.contains(&about) {
                return Some("two rotaries turn about the same machine axis".into());
            }
            abouts.push(about);
            if axis.sign != 1 && axis.sign != -1 {
                return Some(format!("{name}'s sign must be 1 or -1"));
            }
            let [low, high] = axis.limits;
            if !(low.is_finite() && high.is_finite() && low <= high) {
                return Some(format!("{name}'s limits must be a lowest and a highest"));
            }
        }
        None
    }

    /// Where the rotaries go to turn `tool`, a setup's tool axis as a unit
    /// vector in the model, up the spindle: within their limits, the least
    /// turning of the ways there are. `x`, the setup's X in the model, must
    /// then lie along the machine's X, since the program's X is the
    /// machine's. A tool axis up the model's Z already leaves them at zero,
    /// and the part may sit any way round on the table, as without them.
    pub fn position(&self, tool: DVec3, x: DVec3) -> Result<Vec<RotaryAt>, RotaryError> {
        let names: Vec<(char, About, RotaryAxis)> = self
            .axes
            .iter()
            .filter_map(|&n| Some((n, self.about(n)?, self.axis(n)?)))
            .collect();
        let abouts: Vec<DVec3> = names.iter().map(|(_, a, _)| a.vector()).collect();
        let cannot = || RotaryError::CannotReach {
            axes: names
                .iter()
                .map(|(n, ..)| n.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            about: names
                .iter()
                .map(|(n, a, _)| format!("{n}'s {a:?}"))
                .collect::<Vec<_>>()
                .join(" or "),
            axis: tool,
        };
        if (tool - DVec3::Z).length() < SQUARE {
            return Ok(names
                .iter()
                .map(|(n, ..)| RotaryAt {
                    axis: *n,
                    degrees: 0.0,
                })
                .collect());
        }
        // Each way there, as turns of the part about each axis, in radians.
        let ways: Vec<Vec<f64>> = match abouts.as_slice() {
            [u] => {
                if tool.dot(*u).abs() > SQUARE {
                    return Err(cannot());
                }
                vec![vec![signed_angle(tool, DVec3::Z, *u)]]
            }
            [u1, u2] => {
                // The second turns the tool axis round u2 onto where the first
                // turns Z back to: the same height along u2.
                let k = u1.cross(DVec3::Z).dot(*u2);
                let h = tool.dot(*u2);
                if h.abs() > 1.0 + SQUARE {
                    return Err(cannot());
                }
                let first = (-h * k).clamp(-1.0, 1.0).asin();
                [first, std::f64::consts::PI - first]
                    .into_iter()
                    .map(|a1| {
                        let back = DQuat::from_axis_angle(*u1, -a1) * DVec3::Z;
                        let flat = |v: DVec3| v - *u2 * v.dot(*u2);
                        let a2 = if flat(tool).length() < SQUARE {
                            0.0
                        } else {
                            signed_angle(flat(tool), flat(back), *u2)
                        };
                        vec![a1, a2]
                    })
                    .collect()
            }
            _ => return Err(cannot()),
        };
        let turned = |way: &[f64], v: DVec3| {
            way.iter()
                .zip(&abouts)
                .rev()
                .fold(v, |v, (a, u)| DQuat::from_axis_angle(*u, *a) * v)
        };
        // As angles on each rotary, the nearest zero within its limits.
        let mut best: Option<(f64, Vec<f64>, Vec<RotaryAt>)> = None;
        let mut nearest: Option<Vec<RotaryAt>> = None;
        for way in &ways {
            if (turned(way, tool) - DVec3::Z).length() > SQUARE {
                continue;
            }
            let raw: Vec<RotaryAt> = way
                .iter()
                .zip(&names)
                .map(|(a, (n, _, axis))| RotaryAt {
                    axis: *n,
                    degrees: wrapped(a.to_degrees() * f64::from(axis.sign)),
                })
                .collect();
            let within: Option<Vec<RotaryAt>> = raw
                .iter()
                .zip(&names)
                .map(|(at, (_, _, axis))| {
                    within_limits(at.degrees, axis.limits).map(|d| RotaryAt { degrees: d, ..*at })
                })
                .collect();
            nearest.get_or_insert(raw);
            if let Some(at) = within {
                let cost: f64 = at.iter().map(|a| a.degrees.abs()).sum();
                if best.as_ref().is_none_or(|(c, ..)| cost < *c - SLACK) {
                    best = Some((cost, way.clone(), at));
                }
            }
        }
        let Some((_, way, at)) = best else {
            return Err(match nearest {
                Some(at) => RotaryError::OutOfLimits {
                    at,
                    limits: names
                        .iter()
                        .map(|(n, _, a)| {
                            format!("{n} {} to {}", short(a.limits[0]), short(a.limits[1]))
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                },
                None => cannot(),
            });
        };
        let along = turned(&way, x);
        let off = along.y.atan2(along.x).to_degrees();
        if off.abs() > SQUARE.to_degrees() {
            // The model direction the turned part puts along the machine's X.
            let want = way
                .iter()
                .zip(&abouts)
                .fold(DVec3::X, |v, (a, u)| DQuat::from_axis_angle(*u, -*a) * v);
            return Err(RotaryError::Twisted { at, off, want });
        }
        Ok(at)
    }
}

/// The angle turning `from` onto `to` about `about`, both square to it.
fn signed_angle(from: DVec3, to: DVec3, about: DVec3) -> f64 {
    about.dot(from.cross(to)).atan2(from.dot(to))
}

/// Degrees in (-180, 180].
fn wrapped(d: f64) -> f64 {
    let w = d.rem_euclid(360.0);
    if w > 180.0 { w - 360.0 } else { w }
}

/// `d`, or a whole turn more or less, the nearest zero within `limits`.
fn within_limits(d: f64, [low, high]: [f64; 2]) -> Option<f64> {
    [d, d - 360.0, d + 360.0, d - 720.0, d + 720.0]
        .into_iter()
        .filter(|c| *c >= low - SLACK && *c <= high + SLACK)
        .min_by(|a, b| a.abs().total_cmp(&b.abs()))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use glam::dvec3;

    fn one(sign: i8, limits: [f64; 2]) -> Rotary {
        Rotary {
            axes: vec!['A'],
            a: Some(RotaryAxis {
                about: None,
                sign,
                limits,
            }),
            b: None,
            c: None,
        }
    }

    fn table() -> Rotary {
        Rotary {
            axes: vec!['A', 'B'],
            a: None,
            b: None,
            c: None,
        }
    }

    fn tilt(degrees: f64) -> DVec3 {
        let (s, c) = degrees.to_radians().sin_cos();
        dvec3(0.0, -s, c)
    }

    /// Tilted 30° about X, toward -Y, the part turns back 30° about X to
    /// stand it up: A-30 one way round, A30 the other.
    #[test]
    fn an_a_axis_stands_a_tilt_about_x_up() {
        let at = one(1, full_turn()).position(tilt(30.0), DVec3::X).unwrap();
        assert_eq!(at.len(), 1);
        assert_eq!(at[0].axis, 'A');
        assert!((at[0].degrees + 30.0).abs() < 1e-9, "{at:?}");
        let at = one(-1, full_turn()).position(tilt(30.0), DVec3::X).unwrap();
        assert!((at[0].degrees - 30.0).abs() < 1e-9, "{at:?}");
        // Up already: at zero.
        let at = one(1, [10.0, 20.0]).position(DVec3::Z, DVec3::Y).unwrap();
        assert_eq!(at[0].degrees, 0.0);
    }

    #[test]
    fn what_an_a_axis_cannot_reach_is_said() {
        let a = one(1, [-20.0, 20.0]);
        let e = a.position(tilt(30.0), DVec3::X).unwrap_err();
        assert!(matches!(e, RotaryError::OutOfLimits { .. }), "{e}");
        assert!(
            e.to_string().contains("A-30") && e.to_string().contains("-20 to 20"),
            "{e}"
        );
        let e = a.position(dvec3(0.5, 0.0, 1.0).normalize(), DVec3::X);
        assert!(matches!(e, Err(RotaryError::CannotReach { .. })), "{e:?}");
        // X turned off the machine's is said, with the X that would do.
        let (s, c) = 30f64.to_radians().sin_cos();
        let x = dvec3(1.0, c, s).normalize();
        match one(1, full_turn()).position(tilt(30.0), x) {
            Err(RotaryError::Twisted { off, want, .. }) => {
                assert!((off.abs() - 45.0).abs() < 1e-6, "{off}");
                assert!((want - DVec3::X).length() < 1e-9, "{want}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// An A and B table stands any tool axis up, the angles taking it there
    /// within their limits, and says the X they leave.
    #[test]
    fn an_a_and_b_table_stands_any_axis_up() {
        let t = table();
        let tool = dvec3(0.3, -0.4, 0.8).normalize();
        let e = t.position(tool, DVec3::X).unwrap_err();
        let RotaryError::Twisted { want, .. } = e else {
            panic!("{e}")
        };
        let at = t.position(tool, want).unwrap();
        let turn = |at: &[RotaryAt], v: DVec3| {
            let a = DQuat::from_rotation_x(at[0].degrees.to_radians());
            let b = DQuat::from_rotation_y(at[1].degrees.to_radians());
            a * (b * v)
        };
        assert!((turn(&at, tool) - DVec3::Z).length() < 1e-9);
        assert!((turn(&at, want) - DVec3::X).length() < 1e-9);
        assert!(at.iter().all(|a| a.degrees.abs() <= 90.0), "{at:?}");
        // The B alone, for a tilt about Y.
        let lean = dvec3(0.5, 0.0, 1.0).normalize();
        let at = t.position(lean, dvec3(1.0, 0.0, -0.5).normalize()).unwrap();
        assert!(at[0].degrees.abs() < 1e-9, "{at:?}");
        assert!(
            (at[1].degrees + 0.5f64.atan().to_degrees()).abs() < 1e-9,
            "{at:?}"
        );
    }

    #[test]
    fn a_rotary_table_is_read_and_checked() {
        let r: Rotary = toml::from_str(
            "axes = [\"A\", \"B\"]\n[A]\nsign = -1\nlimits = [-30, 120]\n[B]\nabout = \"y\"\n",
        )
        .unwrap();
        assert_eq!(r.problem(), None);
        assert_eq!(r.axis('A').unwrap().limits, [-30.0, 120.0]);
        assert_eq!(r.about('B'), Some(About::Y));
        for (bad, why) in [
            ("axes = [\"A\", \"A\"]", "twice"),
            ("axes = [\"C\"]", "about"),
            ("axes = [\"A\"]\n[B]\nsign = 1", "not in axes"),
            (
                "axes = [\"A\", \"B\"]\n[B]\nabout = \"x\"",
                "same machine axis",
            ),
            ("axes = [\"A\"]\n[A]\nlimits = [10, -10]", "limits"),
            ("axes = [\"A\"]\n[A]\nsign = 2", "sign"),
            ("axes = []", "one rotary or two"),
        ] {
            let r: Rotary = toml::from_str(bad).unwrap();
            assert!(
                r.problem().is_some_and(|p| p.contains(why)),
                "{bad}: {:?}",
                r.problem()
            );
        }
    }
}
