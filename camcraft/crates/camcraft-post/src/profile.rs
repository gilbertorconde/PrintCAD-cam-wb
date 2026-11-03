//! Machine profiles: TOML, not Rust.
//!
//! Adding a machine must never mean recompiling, so a dialect is data. A profile
//! says how to format numbers, whether to suppress modal words, how a tool change is
//! expressed, and what goes at the top and bottom of a file.
//!
//! # Placeholders
//!
//! The lines a profile gives (`header`, `footer`, `safe_retract`, `end_home` and
//! `tool_change_lines`) can name a value of the program they go into as
//! `@name@`, which is replaced with it; `@@` is an `@` itself, and `{name}` the
//! program's name, as a header always had it:
//!
//! | Placeholder | Replaced with |
//! | --- | --- |
//! | `@name@` | the program's name |
//! | `@operation@` | the operation's name |
//! | `@strategy@` | its strategy, as a job file names it |
//! | `@tool@` | the tool's number |
//! | `@tool_comment@` | the tool's description |
//! | `@rpm@` | the spindle's speed, rev/min |
//! | `@spindle@` | `M3` or `M4`: the way the spindle turns |
//! | `@coolant@` | `flood`, `mist` or `off` |
//! | `@coolant_code@` | `M8`, `M7` or `M9` |
//! | `@units@` | `mm` or `inch` |
//! | `@units_code@` | `G21` or `G20` |
//! | `@wcs@` | the work offset, `G54` to `G59` |
//! | `@date@` | the day it was posted, `2026-11-03`, where that is known; else nothing |
//!
//! Words a person typed (the names and the tool's description) are made safe to
//! stand in a comment, as the profile writes comments. A name that is not a
//! placeholder makes the profile invalid, saying which and where
//! ([`crate::placeholder`]).
//!
//! # Travel
//!
//! `[travel]` gives, for any of X, Y and Z, the lowest and highest the
//! program's own coordinates may go, in its units: how far the machine goes
//! each way from the work zero as it is usually set. A program that goes past
//! them is a failed check ([`Profile::travel_problems`]).
//!
//! # Modal suppression tracks what was *emitted*
//!
//! Leaving out an axis word because it has not changed is correct only if the
//! machine already has the value, which means tracking what the renderer actually
//! wrote — not what it intended. The distinction matters at the first move of a
//! program, where nothing has been written yet and every word must appear.
//!
//! # Rounding is a safety property
//!
//! Coordinates are rounded once, and the rounded value is what the modal comparison
//! uses. Comparing unrounded values while emitting rounded ones lets a move be
//! suppressed as "unchanged" when the machine would have seen a different number.

use glam::DVec3;
use serde::Deserialize;

use crate::placeholder;
use crate::rotary::Rotary;

/// How a tool change is expressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChangeStyle {
    /// `T<n> M6`, for a machine with a changer or a macro.
    M6,
    /// A program pause with a comment, for changing by hand.
    Pause,
}

/// How arc centres are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArcStyle {
    /// Incremental centre offsets, `I`/`J`.
    Ij,
    /// Radius, `R`.
    R,
}

/// Number formatting.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Format {
    /// Decimal places for axis words.
    pub axis_decimals: u8,
    /// Decimal places for feed words.
    pub feed_decimals: u8,
    /// Decimal places for axis words in a program written in inches.
    #[serde(default = "four")]
    pub inch_axis_decimals: u8,
    /// Whether to drop trailing zeros, so `10.500` becomes `10.5`.
    #[serde(default)]
    pub strip_trailing_zeros: bool,
    /// Whether a number with its trailing zeros dropped keeps its point: `10.`
    /// rather than `10`, as Fusion writes them.
    #[serde(default)]
    pub keep_point: bool,
}

fn four() -> u8 {
    4
}

/// What units a program is written in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProgramUnits {
    /// Millimetres (`G21`), as the engine works.
    #[default]
    Mm,
    /// Inches (`G20`): every length and feed converted.
    Inch,
}

/// Line numbering.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Numbering {
    /// Whether to number lines at all.
    pub enabled: bool,
    /// First number.
    #[serde(default = "one")]
    pub start: u32,
    /// Step between numbers.
    #[serde(default = "one")]
    pub increment: u32,
}

fn one() -> u32 {
    1
}

/// Modal behaviour.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Modal {
    /// Omit `G0`/`G1` when the motion mode has not changed.
    pub suppress_motion: bool,
    /// Omit an axis word whose value has not changed.
    pub suppress_axes: bool,
    /// Omit `F` when the feed has not changed.
    pub suppress_feed: bool,
}

/// How far a program's coordinates may go along each axis, in its units: the
/// machine's travel, measured from the work zero as it is set for the work.
/// An axis left out may go anywhere.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Travel {
    /// The lowest and highest X.
    #[serde(default)]
    pub x: Option<[f64; 2]>,
    /// The lowest and highest Y.
    #[serde(default)]
    pub y: Option<[f64; 2]>,
    /// The lowest and highest Z.
    #[serde(default)]
    pub z: Option<[f64; 2]>,
}

impl Travel {
    /// Each axis given, by its letter.
    fn axes(&self) -> impl Iterator<Item = (char, usize, [f64; 2])> {
        [('X', self.x), ('Y', self.y), ('Z', self.z)]
            .into_iter()
            .enumerate()
            .filter_map(|(i, (axis, range))| range.map(|r| (axis, i, r)))
    }
}

/// One machine dialect.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Display name.
    pub name: String,
    /// File extension, without the dot.
    pub extension: String,
    /// How comments are wrapped, as a pair of strings.
    pub comment: (String, String),
    /// The characters a comment may hold, letters in either case; any other is
    /// left out, as Fusion's posts filter them. Absent, any may.
    #[serde(default)]
    pub comment_chars: Option<String>,
    /// Whether comments are written in capitals.
    #[serde(default)]
    pub comment_upper: bool,
    /// Number formatting.
    pub format: Format,
    /// Line numbering.
    pub numbering: Numbering,
    /// Modal suppression.
    pub modal: Modal,
    /// Arc centre style.
    pub arc_style: ArcStyle,
    /// The units its programs are written in.
    #[serde(default)]
    pub units: ProgramUnits,
    /// Tool change style.
    pub tool_change: ToolChangeStyle,
    /// Whether the first move down after a tool change takes up the tool's
    /// length offset, `G43 Z… H<tool>`, as Fusion's posts write it. Off, the
    /// controller applies the offset itself, or has none.
    #[serde(default)]
    pub length_offset: bool,
    /// Whether the controller takes canned drilling cycles (`G81`, `G82`, `G83`,
    /// `G73`, `G84`, `G74`, `G80`). Off, a drilling operation is posted as its
    /// moves, and a tapping one not at all ([`LowerError::TapsAsMoves`]).
    ///
    /// [`LowerError::TapsAsMoves`]: crate::op::LowerError::TapsAsMoves
    #[serde(default)]
    pub drill_cycles: bool,
    /// The slowest and fastest the spindle turns, in rev/min. A tool asked to
    /// turn outside them is warned about. Absent, any speed goes.
    #[serde(default)]
    pub spindle: Option<[u32; 2]>,
    /// How far the program's coordinates may go, in its units: see the
    /// module's "Travel". Absent, they are not checked.
    #[serde(default)]
    pub travel: Option<Travel>,
    /// The machine's rotary axes, for 3+2 machining: a setup tilted is
    /// stood up the spindle by them before it cuts ([`Rotary`]). Absent, a
    /// tilted setup is fixtured tilted by hand.
    #[serde(default)]
    pub rotary: Option<Rotary>,
    /// Lines emitted before anything else. Its placeholders are replaced, as
    /// every block's are (see the module's "Placeholders").
    pub header: Vec<String>,
    /// Lines that change the tool, in place of the `tool_change` style's own:
    /// a macro, or a pause with the machine's own words. They must name the
    /// tool, `@tool@`. After them where the machine is is not known, as after
    /// a change by hand. Empty, the style's own.
    #[serde(default)]
    pub tool_change_lines: Vec<String>,
    /// Lines that take the tool straight up clear of everything, as far as the
    /// machine goes, such as `G28 G91 Z0`: before each tool change and before the
    /// program ends. Empty, the tool is not sent up, and the first move after a
    /// tool change rises to its height before it moves across.
    #[serde(default)]
    pub safe_retract: Vec<String>,
    /// Lines that send the machine home across, such as `G28 G91 X0 Y0`, after
    /// the last safe retract and before the footer.
    #[serde(default)]
    pub end_home: Vec<String>,
    /// Lines emitted at the end.
    pub footer: Vec<String>,
}

/// A profile that could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    /// The TOML was malformed or missing a field.
    #[error("profile is not valid: {0}")]
    Parse(#[from] toml::de::Error),
    /// The file could not be read.
    #[error("cannot read {path}: {source}")]
    Io {
        /// Which file.
        path: String,
        /// Why.
        source: std::io::Error,
    },
    /// A value was outside its usable range.
    #[error("{field} is not usable: {reason}")]
    BadValue {
        /// Which field.
        field: &'static str,
        /// Why.
        reason: String,
    },
}

/// Millimetres in an inch.
const MM_PER_INCH: f64 = 25.4;

/// The profiles compiled in, by name: each a file of its own in `profiles/`.
const BUILTIN: &[(&str, &str)] = &[
    ("masso", include_str!("../profiles/masso.toml")),
    ("masso-inch", include_str!("../profiles/masso-inch.toml")),
    ("grbl", include_str!("../profiles/grbl.toml")),
    (
        "centroid-avid",
        include_str!("../profiles/centroid-avid.toml"),
    ),
    (
        "centroid-avid-inch",
        include_str!("../profiles/centroid-avid-inch.toml"),
    ),
    ("linuxcnc", include_str!("../profiles/linuxcnc.toml")),
    (
        "linuxcnc-inch",
        include_str!("../profiles/linuxcnc-inch.toml"),
    ),
];

impl Profile {
    /// Parse a profile from TOML.
    pub fn from_toml(text: &str) -> Result<Self, ProfileError> {
        let p: Profile = toml::from_str(text)?;
        p.validate()?;
        Ok(p)
    }

    /// Load a profile from a file.
    pub fn load(path: impl AsRef<std::path::Path>) -> Result<Self, ProfileError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| ProfileError::Io {
            path: path.display().to_string(),
            source,
        })?;
        Self::from_toml(&text)
    }

    /// One of the compiled-in profiles, by name.
    pub fn builtin(name: &str) -> Option<Result<Self, ProfileError>> {
        let name = name.to_ascii_lowercase();
        BUILTIN
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, text)| Self::from_toml(text))
    }

    /// Names of the compiled-in profiles.
    pub fn builtin_names() -> impl Iterator<Item = &'static str> {
        BUILTIN.iter().map(|(name, _)| *name)
    }

    fn validate(&self) -> Result<(), ProfileError> {
        // 9 places is past f64's useful precision at machine scale; past that the
        // output is noise dressed as precision.
        if self.format.axis_decimals > 9 {
            return Err(ProfileError::BadValue {
                field: "format.axis_decimals",
                reason: format!("{} exceeds 9", self.format.axis_decimals),
            });
        }
        if self.format.inch_axis_decimals > 9 {
            return Err(ProfileError::BadValue {
                field: "format.inch_axis_decimals",
                reason: format!("{} exceeds 9", self.format.inch_axis_decimals),
            });
        }
        if self.format.feed_decimals > 9 {
            return Err(ProfileError::BadValue {
                field: "format.feed_decimals",
                reason: format!("{} exceeds 9", self.format.feed_decimals),
            });
        }
        if self.numbering.enabled && self.numbering.increment == 0 {
            return Err(ProfileError::BadValue {
                field: "numbering.increment",
                reason: "zero would repeat the same line number forever".into(),
            });
        }
        if self.extension.is_empty() {
            return Err(ProfileError::BadValue {
                field: "extension",
                reason: "must not be empty".into(),
            });
        }
        if self.comment.0.is_empty() {
            return Err(ProfileError::BadValue {
                field: "comment",
                reason: "a comment needs something to open it".into(),
            });
        }
        if self.comment_chars.as_deref() == Some("") {
            return Err(ProfileError::BadValue {
                field: "comment_chars",
                reason: "would leave every comment empty; leave it out to allow any character"
                    .into(),
            });
        }
        if let Some(rotary) = &self.rotary {
            if let Some(why) = rotary.problem() {
                return Err(ProfileError::BadValue {
                    field: "rotary",
                    reason: why,
                });
            }
            // The part turns under the tool: it must be sent clear first.
            if self.safe_retract.is_empty() {
                return Err(ProfileError::BadValue {
                    field: "rotary",
                    reason: "rotaries need a safe_retract, to send the tool clear before the \
                             part turns under it"
                        .into(),
                });
            }
        }
        if let Some([low, high]) = self.spindle
            && low > high
        {
            return Err(ProfileError::BadValue {
                field: "spindle",
                reason: format!("its slowest, {low}, is faster than its fastest, {high}"),
            });
        }
        if let Some(travel) = &self.travel {
            for (axis, _, [low, high]) in travel.axes() {
                if !(low.is_finite() && high.is_finite()) || low >= high {
                    return Err(ProfileError::BadValue {
                        field: "travel",
                        reason: format!(
                            "{} = [{low}, {high}] is not a range: give its lowest, then its highest",
                            axis.to_ascii_lowercase()
                        ),
                    });
                }
            }
        }
        for (field, lines) in self.blocks() {
            for (i, line) in lines.iter().enumerate() {
                if let Some(why) = placeholder::problem(line) {
                    return Err(ProfileError::BadValue {
                        field,
                        reason: format!("line {}: {why}", i + 1),
                    });
                }
            }
        }
        if !self.tool_change_lines.is_empty()
            && !self.tool_change_lines.iter().any(|l| l.contains("@tool@"))
        {
            return Err(ProfileError::BadValue {
                field: "tool_change_lines",
                reason: "they never say which tool: put @tool@ in them, as `T@tool@ M6`".into(),
            });
        }
        Ok(())
    }

    /// Each block of lines the profile gives, by its field.
    fn blocks(&self) -> [(&'static str, &[String]); 5] {
        [
            ("header", &self.header),
            ("tool_change_lines", &self.tool_change_lines),
            ("safe_retract", &self.safe_retract),
            ("end_home", &self.end_home),
            ("footer", &self.footer),
        ]
    }

    /// What is wrong with a program that goes from `low` to `high`, in
    /// millimetres in its own coordinates, on this machine: each axis it
    /// takes past the `travel`, saying how far it goes and how far it may.
    pub fn travel_problems(&self, low: DVec3, high: DVec3) -> Vec<String> {
        let Some(travel) = &self.travel else {
            return Vec::new();
        };
        let (scale, unit) = match self.units {
            ProgramUnits::Mm => (1.0, "mm"),
            ProgramUnits::Inch => (1.0 / MM_PER_INCH, "in"),
        };
        let (low, high) = (low * scale, high * scale);
        // Within the places it is written to is within.
        let slack = 0.5
            * 10f64.powi(-i32::from(match self.units {
                ProgramUnits::Mm => self.format.axis_decimals,
                ProgramUnits::Inch => self.format.inch_axis_decimals,
            }));
        travel
            .axes()
            .filter(|&(_, i, [min, max])| low[i] < min - slack || high[i] > max + slack)
            .map(|(axis, i, [min, max])| {
                let n = |v: f64| self.length(v / scale, self.units == ProgramUnits::Inch);
                format!(
                    "{axis} goes from {} to {} {unit}, past {}'s travel of {} to {} {unit} \
                     from the work zero",
                    n(low[i]),
                    n(high[i]),
                    self.name,
                    n(min),
                    n(max)
                )
            })
            .collect()
    }

    /// What is wrong with turning the spindle at `rpm` on this machine, if
    /// anything: a speed outside its `spindle` range.
    pub fn spindle_problem(&self, rpm: u32) -> Option<String> {
        let [low, high] = self.spindle?;
        (rpm < low || rpm > high).then(|| {
            format!(
                "the spindle is asked for {rpm} rev/min, outside {}'s {low} to {high}",
                self.name
            )
        })
    }

    /// Format a value for an axis word.
    pub fn axis(&self, v: f64) -> String {
        self.number(v, self.format.axis_decimals)
    }

    /// Format a length in millimetres for an axis word, in inches if `inches`.
    pub fn length(&self, mm: f64, inches: bool) -> String {
        if inches {
            self.number(mm / MM_PER_INCH, self.format.inch_axis_decimals)
        } else {
            self.axis(mm)
        }
    }

    /// Format a feed in mm/min for a feed word, in in/min if `inches`.
    pub fn feed_in(&self, mm_per_min: f64, inches: bool) -> String {
        self.feed(if inches {
            mm_per_min / MM_PER_INCH
        } else {
            mm_per_min
        })
    }

    /// Format a feed in mm/min for a feed word, in in/min if `inches`, to
    /// three more places than other feeds: a tap's, which has to keep to its
    /// pitch.
    pub fn feed_in_finely(&self, mm_per_min: f64, inches: bool) -> String {
        let v = if inches {
            mm_per_min / MM_PER_INCH
        } else {
            mm_per_min
        };
        self.number(v, self.format.feed_decimals.saturating_add(3))
    }

    /// Format a value for a feed word.
    pub fn feed(&self, v: f64) -> String {
        self.number(v, self.format.feed_decimals)
    }

    /// Format a time in seconds for a dwell word: to the millisecond, as
    /// Fusion's posts write one, whatever the feed's places.
    pub fn seconds(&self, v: f64) -> String {
        self.number(v, 3)
    }

    fn number(&self, v: f64, decimals: u8) -> String {
        let mut s = format!("{v:.*}", decimals as usize);
        // "-0.000" is the same position as "0.000" and reads like an error.
        if s.starts_with('-') && s[1..].chars().all(|c| c == '0' || c == '.') {
            s.remove(0);
        }
        if self.format.strip_trailing_zeros && s.contains('.') {
            while s.ends_with('0') {
                s.pop();
            }
            if s.ends_with('.') && !self.format.keep_point {
                s.pop();
            }
        }
        s
    }

    /// Wrap text as a comment in this dialect.
    ///
    /// Both delimiters are substituted, not just the closing one. A stray close
    /// would end the comment early and leave the rest of the line to be executed;
    /// a stray open leaves an unbalanced comment that several controllers reject or
    /// swallow the next block into. Square brackets keep the text readable.
    pub fn comment(&self, text: &str) -> String {
        format!("{}{}{}", self.comment.0, self.escaped(text), self.comment.1)
    }

    /// `text` made safe to stand inside a comment: its delimiters substituted, as
    /// [`comment`](Self::comment) says, and control characters, a newline above
    /// all, made spaces. An empty delimiter, as a comment to the line's end has
    /// for its close, is nothing to substitute. Then in capitals, and without
    /// what the profile's comments may not hold, where it says so.
    pub fn escaped(&self, text: &str) -> String {
        let mut safe = text.to_string();
        for (delimiter, with) in [(&self.comment.0, "["), (&self.comment.1, "]")] {
            if !delimiter.is_empty() {
                safe = safe.replace(delimiter.as_str(), with);
            }
        }
        if self.comment_upper {
            safe = safe.to_uppercase();
        }
        let allowed = |c: char| {
            self.comment_chars
                .as_deref()
                .is_none_or(|chars| chars.chars().any(|a| a.eq_ignore_ascii_case(&c)))
        };
        safe.chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .filter(|&c| allowed(c))
            .collect()
    }
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

    #[test]
    fn both_builtins_parse() {
        for name in Profile::builtin_names() {
            let p = Profile::builtin(name).unwrap().unwrap();
            assert!(!p.name.is_empty());
            assert!(!p.extension.is_empty());
        }
        assert!(Profile::builtin("nonesuch").is_none());
    }

    /// Masso in inches is its own file, and the Masso profile in everything but
    /// its name and units.
    #[test]
    fn masso_inch_is_masso_in_inches() {
        let masso = Profile::builtin("masso").unwrap().unwrap();
        let inch = Profile::builtin("masso-inch").unwrap().unwrap();
        let expected = Profile {
            name: "Masso (inch)".into(),
            units: ProgramUnits::Inch,
            ..masso
        };
        assert_eq!(inch, expected);
    }

    /// The Avid profile in inches is its own file, and the millimetre one in
    /// everything but its name, units and feed, which Fusion's Avid post writes
    /// to a tenth in inches and to a whole number in millimetres.
    #[test]
    fn centroid_avid_inch_is_centroid_avid_in_inches() {
        let mm = Profile::builtin("centroid-avid").unwrap().unwrap();
        let inch = Profile::builtin("centroid-avid-inch").unwrap().unwrap();
        let expected = Profile {
            name: "Centroid CNC12 (Avid CNC, inch)".into(),
            units: ProgramUnits::Inch,
            format: Format {
                feed_decimals: 1,
                ..mm.format.clone()
            },
            ..mm
        };
        assert_eq!(inch, expected);
    }

    /// LinuxCNC in inches is the millimetre profile in everything but its name
    /// and units.
    #[test]
    fn linuxcnc_inch_is_linuxcnc_in_inches() {
        let mm = Profile::builtin("linuxcnc").unwrap().unwrap();
        let inch = Profile::builtin("linuxcnc-inch").unwrap().unwrap();
        let expected = Profile {
            name: "LinuxCNC (inch)".into(),
            units: ProgramUnits::Inch,
            ..mm
        };
        assert_eq!(inch, expected);
    }

    /// A speed outside the machine's range is a problem; inside it, or with no
    /// range given, none.
    #[test]
    fn spindle_speeds_are_checked_against_the_range() {
        let mut p = Profile::builtin("grbl").unwrap().unwrap();
        assert_eq!(p.spindle_problem(100_000), None, "no range, any speed");
        p.spindle = Some([8000, 24000]);
        assert_eq!(p.spindle_problem(8000), None);
        assert_eq!(p.spindle_problem(24000), None);
        let slow = p.spindle_problem(5000).unwrap();
        assert!(
            slow.contains("5000") && slow.contains("8000 to 24000"),
            "{slow}"
        );
        assert!(p.spindle_problem(30000).is_some());
        p.spindle = Some([24000, 8000]);
        assert!(p.validate().is_err(), "slowest above fastest");
    }

    /// Avid as Fusion's Avid post writes it: no trailing zeros and no bare
    /// point, and comments in capitals with only what that post lets through.
    #[test]
    fn centroid_avid_writes_numbers_and_comments_as_fusion_does() {
        let p = Profile::builtin("centroid-avid").unwrap().unwrap();
        assert_eq!(p.axis(10.0), "10");
        assert_eq!(p.axis(2.25), "2.25");
        assert_eq!(p.feed(1200.4), "1200");
        assert_eq!(p.length(25.4 * 3.46084, true), "3.4608");
        assert_eq!(p.comment("6mm ball (carbide) #2"), "(6MM BALL CARBIDE 2)");
        assert_eq!(p.comment("rough: top/side"), "(ROUGH TOPSIDE)");
    }

    /// Masso as Fusion's post writes it: rounded to the places given, trailing
    /// zeros dropped and the point kept; inches to four places.
    #[test]
    fn rounds_to_the_stated_precision() {
        let p = Profile::builtin("masso").unwrap().unwrap();
        assert_eq!(p.axis(1.23456789), "1.235");
        assert_eq!(p.axis(10.0), "10.");
        assert_eq!(p.axis(2.2), "2.2");
        assert_eq!(p.feed(150.0), "150.");
        assert_eq!(p.length(25.4 * 3.46084, true), "3.4608");
        assert_eq!(p.feed_in(3810.0, true), "150.");
        let mut plain = p.clone();
        plain.format.strip_trailing_zeros = false;
        assert_eq!(plain.axis(10.0), "10.000");
        let grbl = Profile::builtin("grbl").unwrap().unwrap();
        assert_eq!(grbl.axis(10.0), "10", "GRBL drops the point too");
        let inch = Profile::builtin("masso-inch").unwrap().unwrap();
        assert_eq!(inch.units, ProgramUnits::Inch);
    }

    /// A negative zero is the same position as zero and reads like a fault.
    #[test]
    fn never_emits_negative_zero() {
        let p = Profile::builtin("masso").unwrap().unwrap();
        assert_eq!(p.axis(-0.0), "0.");
        assert_eq!(p.axis(-0.0001), "0.");
        assert_eq!(p.axis(-0.001), "-0.001");
    }

    /// A stray close-paren ends the comment and leaves the rest of the line to be
    /// executed; a stray open leaves it unbalanced. Neither may survive.
    #[test]
    fn comments_cannot_escape_themselves() {
        let p = Profile::builtin("masso").unwrap().unwrap();
        let c = p.comment("tool (6mm) ball");
        assert_eq!(c.matches('(').count(), 1, "only the opening delimiter: {c}");
        assert_eq!(c.matches(')').count(), 1, "only the closing delimiter: {c}");
        assert!(c.starts_with('(') && c.ends_with(')'));
        assert!(c.contains("[6mm]"), "text should stay readable: {c}");
    }

    /// A newline inside a comment would end the block and execute the remainder.
    #[test]
    fn comments_cannot_contain_newlines() {
        let p = Profile::builtin("masso").unwrap().unwrap();
        let c = p.comment(&format!("first{}M3 S30000", char::from(10)));
        assert!(!c.contains(char::from(10)), "newline survived: {c:?}");
    }

    /// A comment to the line's end has no close to substitute; substituting
    /// nothing would put a bracket between every character.
    #[test]
    fn a_comment_to_the_line_end_reads_as_written() {
        let mut p = Profile::builtin("grbl").unwrap().unwrap();
        p.comment = (";".into(), String::new());
        assert_eq!(p.comment("fit tool 3; then go"), ";fit tool 3[ then go");
    }

    /// Filtering keeps letters in whichever case they were written, and a
    /// newline, made a space, survives only where a space may.
    #[test]
    fn comment_chars_match_either_case() {
        let mut p = Profile::builtin("masso").unwrap().unwrap();
        p.comment_chars = Some("abc ".into());
        assert_eq!(
            p.comment(&format!("AbCd{}cab!", char::from(10))),
            "(AbC cab)"
        );
        p.comment_chars = Some("abc".into());
        assert_eq!(p.comment(&format!("ab{}c", char::from(10))), "(abc)");
    }

    #[test]
    fn rejects_unusable_values() {
        let bad = r#"
            name = "x"
            extension = "nc"
            comment = ["(", ")"]
            arc_style = "ij"
            tool_change = "m6"
            header = []
            footer = []
            [format]
            axis_decimals = 20
            feed_decimals = 1
            [numbering]
            enabled = false
            [modal]
            suppress_motion = true
            suppress_axes = true
            suppress_feed = true
        "#;
        assert!(matches!(
            Profile::from_toml(bad),
            Err(ProfileError::BadValue {
                field: "format.axis_decimals",
                ..
            })
        ));
    }

    /// GRBL with `extra` TOML added, which a profile's own blocks or tables
    /// may be.
    fn grbl_with(extra: &str) -> Result<Profile, ProfileError> {
        let grbl = include_str!("../profiles/grbl.toml");
        // Top-level keys go before the first table.
        let (top, tables) = grbl.split_at(grbl.find("[format]").unwrap());
        Profile::from_toml(&format!("{top}\n{extra}\n{tables}"))
    }

    /// A placeholder that is not one makes the profile invalid, saying which
    /// block, which line, and which name; known ones and `{name}` load.
    #[test]
    fn an_unknown_placeholder_is_refused_saying_where() {
        let ok = grbl_with("tool_change_lines = [\"(fit @tool_comment@)\", \"T@tool@ M6\"]");
        assert!(ok.is_ok(), "{ok:?}");
        let e = grbl_with("end_home = [\"G28 G91 X0 Y0\", \"(done @nmae@)\"]").unwrap_err();
        let said = e.to_string();
        assert!(
            said.starts_with("end_home is not usable: line 2: @nmae@ is not a placeholder"),
            "{said}"
        );
        let e = grbl_with("end_home = [\"G28 G91 X0 Y0 (me@home)\"]").unwrap_err();
        assert!(e.to_string().contains("never closed"), "{e}");
    }

    /// Its own tool change must say which tool, or every program would cut
    /// with whatever was in.
    #[test]
    fn a_tool_change_of_ones_own_names_the_tool() {
        let e = grbl_with("tool_change_lines = [\"M6\"]").unwrap_err();
        assert!(
            matches!(
                e,
                ProfileError::BadValue {
                    field: "tool_change_lines",
                    ..
                }
            ),
            "{e}"
        );
    }

    /// Travel inside its range is fine, outside it is said, axis by axis,
    /// in the program's units; an axis not given goes anywhere.
    #[test]
    fn travel_is_checked_axis_by_axis() {
        use glam::dvec3;
        let p = grbl_with("[travel]\nx = [-10, 300]\nz = [-60, 40]").unwrap();
        assert!(
            p.travel_problems(dvec3(-10.0, -500.0, -60.0), dvec3(300.0, 900.0, 40.0))
                .is_empty()
        );
        // Within the places it is written to is within.
        assert!(
            p.travel_problems(dvec3(-10.0004, 0.0, 0.0), dvec3(0.0, 0.0, 0.0))
                .is_empty()
        );
        let past = p.travel_problems(dvec3(-10.0, 0.0, -75.5), dvec3(320.0, 0.0, 10.0));
        assert_eq!(past.len(), 2, "{past:?}");
        assert!(
            past[0].starts_with("X goes from -10 to 320 mm, past GRBL's travel of -10 to 300 mm"),
            "{}",
            past[0]
        );
        assert!(
            past[1].starts_with("Z goes from -75.5 to 10 mm"),
            "{}",
            past[1]
        );
        let mut inch = p.clone();
        inch.units = ProgramUnits::Inch;
        inch.travel = Some(Travel {
            x: Some([0.0, 10.0]),
            ..Travel::default()
        });
        let past = inch.travel_problems(dvec3(0.0, 0.0, 0.0), dvec3(279.4, 0.0, 0.0));
        assert!(past[0].contains("to 11 in, past"), "{past:?}");
        let none = Profile::builtin("grbl").unwrap().unwrap();
        assert!(
            none.travel_problems(DVec3::splat(-1e6), DVec3::splat(1e6))
                .is_empty()
        );
    }

    #[test]
    fn a_travel_must_be_a_range() {
        for bad in ["x = [300, 0]", "y = [1, 1]", "z = [nan, 3]"] {
            let e = grbl_with(&format!("[travel]\n{bad}")).unwrap_err();
            assert!(
                matches!(
                    e,
                    ProfileError::BadValue {
                        field: "travel",
                        ..
                    }
                ),
                "{bad}: {e}"
            );
        }
        assert!(grbl_with("[travel]\nq = [0, 1]").is_err(), "no Q axis");
    }

    #[test]
    fn strip_trailing_zeros_is_per_profile() {
        let masso = Profile::builtin("masso").unwrap().unwrap();
        let grbl = Profile::builtin("grbl").unwrap().unwrap();
        // Whatever each chooses, it must be self-consistent.
        for p in [&masso, &grbl] {
            let s = p.axis(10.5);
            if p.format.strip_trailing_zeros {
                assert!(!s.ends_with('0'), "{} kept a trailing zero: {s}", p.name);
            } else {
                assert_eq!(
                    s.split('.').nth(1).map(str::len),
                    Some(p.format.axis_decimals as usize)
                );
            }
        }
    }
}
