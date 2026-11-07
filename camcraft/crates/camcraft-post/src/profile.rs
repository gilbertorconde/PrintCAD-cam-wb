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
//! | `@wcs@` | the work offset, `G54` to `G59`, or an extended one |
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
//! machine goes, in the program's units. With `coordinates = "machine"`
//! they are its own coordinates, as `G53` moves in, and a program is placed
//! on the machine by where its work zero is there (what its work offset
//! holds) and, with length offsets, the tool's length from the gauge line on
//! Z ([`Placement`]); a program whose work zero is not known there is not
//! checked, and says so. Without, as profiles had it first, they are the
//! program's own coordinates: how far it goes each way from the work zero as
//! it is usually set. A program that goes past them is a failed check
//! ([`Profile::travel_check`]). In machine coordinates, a `G53` move in the
//! profile's own blocks (a `safe_retract` of `G53 G0 Z0`) must be within
//! them too, or the profile is not valid.
//!
//! # Work offsets
//!
//! A setup's work offset is `G54` to `G59` (54 to 59), and from 60 the
//! profile's `extended_offsets` in order, written as it gives them:
//! LinuxCNC's `["G59.1", "G59.2", "G59.3"]`, or a Fanuc's `"G54.1 P1"` on.
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

use camcraft_core::job::LAST_WCS;

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

/// The language a program is written in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dialect {
    /// G-code, ISO 6983 as Fanuc, LinuxCNC and the rest write it.
    #[default]
    Iso,
    /// Heidenhain's conversational Klartext (`.h`): `L X+10 R0 F500`, arcs
    /// by `CC` and `C`, drilling by `CYCL DEF` ([`crate::heidenhain`]).
    Heidenhain,
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

/// What a [`Travel`]'s numbers are measured from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Coordinates {
    /// The work zero as it is usually set: the program's own coordinates.
    #[default]
    Work,
    /// The machine's own, as `G53` moves in.
    Machine,
}

/// How far the machine goes along each axis, in the program's units: in its
/// own coordinates, or from the work zero as it is set for the work. An axis
/// left out may go anywhere.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Travel {
    /// What the ranges are measured from: the work zero unless said.
    #[serde(default)]
    pub coordinates: Coordinates,
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

/// Where a program stands on the machine, for its travel in machine
/// coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Placement {
    /// The work zero in machine coordinates, in millimetres: what the work
    /// offset holds. Z is the gauge line's with length offsets, else the
    /// tool's tip's. `None` when not known.
    pub zero: Option<DVec3>,
    /// The tool's length from the spindle's gauge line, in millimetres,
    /// added to Z when the profile takes length offsets. `None` when not
    /// known.
    pub tool_length: Option<f64>,
}

/// How a program keeps to a machine's travel.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TravelCheck {
    /// Each axis it goes past, saying how far: a failed check.
    pub past: Vec<String>,
    /// What could not be checked, and why: a note.
    pub unchecked: Vec<String>,
}

/// One machine dialect.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Display name.
    pub name: String,
    /// The language its programs are written in: G-code unless it says
    /// otherwise.
    #[serde(default)]
    pub dialect: Dialect,
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
    /// Whether the tool the next program changes to is called up to the
    /// changer's ready place after the change, `T<n>` (Heidenhain's `TOOL
    /// DEF <n>`), so the next change is quicker. Where no next tool is known,
    /// nothing is written.
    #[serde(default)]
    pub preselect_tool: bool,
    /// Whether a pause's `P` word is in milliseconds, a whole number, as
    /// Fanuc reads it in `G4` and `G82`; otherwise in seconds.
    #[serde(default)]
    pub dwell_milliseconds: bool,
    /// The slowest and fastest the spindle turns, in rev/min. A tool asked to
    /// turn outside them is warned about. Absent, any speed goes.
    #[serde(default)]
    pub spindle: Option<[u32; 2]>,
    /// How far the machine goes, in the program's units: see the module's
    /// "Travel". Absent, it is not checked.
    #[serde(default)]
    pub travel: Option<Travel>,
    /// The work offsets it has past `G59`, as it writes each: see the
    /// module's "Work offsets".
    #[serde(default)]
    pub extended_offsets: Vec<String>,
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
    ("fanuc", include_str!("../profiles/fanuc.toml")),
    ("fanuc-inch", include_str!("../profiles/fanuc-inch.toml")),
    ("haas", include_str!("../profiles/haas.toml")),
    ("haas-inch", include_str!("../profiles/haas-inch.toml")),
    ("mach", include_str!("../profiles/mach.toml")),
    ("mach-inch", include_str!("../profiles/mach-inch.toml")),
    ("heidenhain", include_str!("../profiles/heidenhain.toml")),
    (
        "heidenhain-inch",
        include_str!("../profiles/heidenhain-inch.toml"),
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
        if self.extended_offsets.len() > usize::from(LAST_WCS - 59) {
            return Err(ProfileError::BadValue {
                field: "extended_offsets",
                reason: format!("at most {} past G59", LAST_WCS - 59),
            });
        }
        for word in &self.extended_offsets {
            let ok = word.starts_with(['G', 'g'])
                && word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ' '));
            if !ok {
                return Err(ProfileError::BadValue {
                    field: "extended_offsets",
                    reason: format!("{word:?} is not a work offset's G code, as \"G59.1\""),
                });
            }
        }
        if let Some(why) = self.machine_moves_past_travel() {
            return Err(ProfileError::BadValue {
                field: "travel",
                reason: why,
            });
        }
        if self.dialect == Dialect::Heidenhain {
            // Every block is numbered, from 0 by ones, the program's first
            // line `BEGIN PGM`: the control reads nothing else.
            if !self.numbering.enabled || self.numbering.start != 0 || self.numbering.increment != 1
            {
                return Err(ProfileError::BadValue {
                    field: "numbering",
                    reason: "a Heidenhain program numbers every block from 0 by 1: \
                             enabled = true, start = 0, increment = 1"
                        .into(),
                });
            }
            if self.comment != (";".to_string(), String::new()) {
                return Err(ProfileError::BadValue {
                    field: "comment",
                    reason: "a Heidenhain comment runs from \";\" to the line's end: \
                             comment = [\";\", \"\"]"
                        .into(),
                });
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

    /// The first `G53` move of the profile's own blocks that goes past its
    /// travel in machine coordinates, said; `None` when there is none, or
    /// the travel is from the work zero, which a `G53` move does not know.
    fn machine_moves_past_travel(&self) -> Option<String> {
        let travel = self
            .travel
            .filter(|t| t.coordinates == Coordinates::Machine)?;
        for (field, lines) in self.blocks() {
            for (i, line) in lines.iter().enumerate() {
                let words = crate::custom::words(line);
                // G53, or in Klartext M91: a move in machine coordinates.
                let (letter, code) = match self.dialect {
                    Dialect::Iso => ('G', 53.0),
                    Dialect::Heidenhain => ('M', 91.0),
                };
                let g53 = words
                    .iter()
                    .any(|(c, v, _)| *c == letter && *v == Some(code));
                if !g53 {
                    continue;
                }
                for (axis, _, [low, high]) in travel.axes() {
                    let to = words
                        .iter()
                        .find_map(|(c, v, _)| (*c == axis).then_some(*v).flatten());
                    if let Some(v) = to.filter(|v| *v < low || *v > high) {
                        return Some(format!(
                            "{field} line {}, a G53 move, sends {axis} to {v}, outside the \
                             machine's travel of {low} to {high}",
                            i + 1
                        ));
                    }
                }
            }
        }
        None
    }

    /// The word that selects work offset `wcs` (54 for `G54`), if this
    /// machine has it: `G54` to `G59`, then its `extended_offsets`.
    pub fn wcs_word(&self, wcs: u8) -> Option<String> {
        match wcs {
            54..=59 => Some(format!("G{wcs}")),
            _ => self
                .extended_offsets
                .get(usize::from(wcs.checked_sub(60)?))
                .cloned(),
        }
    }

    /// Every work offset this machine has, by its number and its word.
    pub fn work_offsets(&self) -> Vec<(u8, String)> {
        (54..=LAST_WCS)
            .map_while(|n| self.wcs_word(n).map(|w| (n, w)))
            .collect()
    }

    /// How a program that goes from `low` to `high`, in millimetres in its
    /// own coordinates, keeps to this machine's `travel`, placed on it by
    /// `place` when the travel is in machine coordinates: each axis it goes
    /// past, saying how far, and what could not be checked.
    pub fn travel_check(&self, low: DVec3, high: DVec3, place: &Placement) -> TravelCheck {
        let mut out = TravelCheck::default();
        let Some(travel) = &self.travel else {
            return out;
        };
        let (scale, unit) = match self.units {
            ProgramUnits::Mm => (1.0, "mm"),
            ProgramUnits::Inch => (1.0 / MM_PER_INCH, "in"),
        };
        let n = |v: f64| self.length(v / scale, self.units == ProgramUnits::Inch);
        let mut shift = DVec3::ZERO;
        let mut skip_z = false;
        let frame = match travel.coordinates {
            Coordinates::Work => "from the work zero".to_string(),
            Coordinates::Machine => {
                let Some(zero) = place.zero else {
                    out.unchecked.push(format!(
                        "{}'s travel is in machine coordinates, and where the work offset \
                         is on the machine is not given: not checked",
                        self.name
                    ));
                    return out;
                };
                shift = zero;
                let mut frame = "in machine coordinates".to_string();
                if self.length_offset {
                    match place.tool_length {
                        Some(l) => {
                            shift.z += l;
                            frame = format!(
                                "{frame}, the tool {} {unit} from the gauge line",
                                n(l * scale)
                            );
                        }
                        None => {
                            skip_z = travel.z.is_some();
                            if skip_z {
                                out.unchecked.push(
                                    "Z is not checked: the tool's length from the gauge line \
                                     is not known (give its stickout or overall length)"
                                        .into(),
                                );
                            }
                        }
                    }
                }
                frame
            }
        };
        let (low, high) = ((low + shift) * scale, (high + shift) * scale);
        // Within the places it is written to is within.
        let slack = 0.5
            * 10f64.powi(-i32::from(match self.units {
                ProgramUnits::Mm => self.format.axis_decimals,
                ProgramUnits::Inch => self.format.inch_axis_decimals,
            }));
        for (axis, i, [min, max]) in travel.axes() {
            if skip_z && axis == 'Z' {
                continue;
            }
            let under = min - low[i];
            let over = high[i] - max;
            let how = match (under > slack, over > slack) {
                (false, false) => continue,
                (true, false) => format!("{} {unit} under", n(under)),
                (false, true) => format!("{} {unit} over", n(over)),
                (true, true) => format!("{} {unit} under and {} {unit} over", n(under), n(over)),
            };
            out.past.push(format!(
                "{axis} goes from {} to {} {unit} {frame}, {how} {}'s travel of {} to {} {unit}",
                n(low[i]),
                n(high[i]),
                self.name,
                n(min),
                n(max)
            ));
        }
        out
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

    /// Format a pause of `seconds` for a `P` word: in milliseconds where the
    /// profile says, else in seconds.
    pub fn dwell(&self, seconds: f64) -> String {
        if self.dwell_milliseconds {
            format!("{:.0}", seconds * 1000.0)
        } else {
            self.seconds(seconds)
        }
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

    /// Format a value to `decimals` places, as the profile formats numbers.
    pub fn number(&self, v: f64, decimals: u8) -> String {
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

    /// Each industrial profile in inches is its own file, and the millimetre
    /// one in everything but its name, units and how finely it writes a feed.
    #[test]
    fn each_inch_profile_is_its_millimetre_one_in_inches() {
        for (mm, name, feed_decimals) in [
            ("fanuc", "Fanuc (inch)", 1),
            ("haas", "Haas (NGC, inch)", 2),
            ("mach", "Mach3 / Mach4 (inch)", 1),
            ("heidenhain", "Heidenhain TNC (Klartext, inch)", 1),
        ] {
            let p = Profile::builtin(mm).unwrap().unwrap();
            let inch = Profile::builtin(&format!("{mm}-inch")).unwrap().unwrap();
            let expected = Profile {
                name: name.into(),
                units: ProgramUnits::Inch,
                format: Format {
                    feed_decimals,
                    ..p.format.clone()
                },
                ..p
            };
            assert_eq!(inch, expected, "{mm}");
        }
    }

    /// A Heidenhain profile numbers every block from 0 by 1 and comments to
    /// the line's end, or the control could not read its programs.
    #[test]
    fn a_heidenhain_profile_numbers_from_nought_and_comments_with_a_semicolon() {
        let text = include_str!("../profiles/heidenhain.toml");
        assert!(Profile::from_toml(text).is_ok());
        for (from, to, field) in [
            ("start     = 0", "start     = 1", "numbering"),
            ("enabled   = true", "enabled   = false", "numbering"),
            (
                "comment   = [\";\", \"\"]",
                "comment   = [\"(\", \")\"]",
                "comment",
            ),
        ] {
            assert!(text.contains(from), "{from}");
            let e = Profile::from_toml(&text.replace(from, to)).unwrap_err();
            assert!(
                matches!(e, ProfileError::BadValue { field: f, .. } if f == field),
                "{to}: {e}"
            );
        }
    }

    /// In Klartext a move in machine coordinates is `M91`'s: one of the
    /// profile's own outside a machine travel makes it invalid, as G53's does.
    #[test]
    fn a_heidenhain_m91_move_is_kept_inside_the_machine_travel() {
        let text = include_str!("../profiles/heidenhain.toml");
        let travel = "\n[travel]\ncoordinates = \"machine\"\nz = [-300, 0]\n";
        assert!(Profile::from_toml(&format!("{text}{travel}")).is_ok());
        let high = text.replace("L Z+0 R0 FMAX M91", "L Z+50 R0 FMAX M91");
        let e = Profile::from_toml(&format!("{high}{travel}")).unwrap_err();
        assert!(
            e.to_string().contains("outside the machine's travel"),
            "{e}"
        );
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
    /// how far past, in the program's units; an axis not given goes
    /// anywhere.
    #[test]
    fn travel_is_checked_axis_by_axis() {
        use glam::dvec3;
        let p = grbl_with("[travel]\nx = [-10, 300]\nz = [-60, 40]").unwrap();
        let at = Placement::default();
        let check = |p: &Profile, low, high| p.travel_check(low, high, &at).past;
        assert!(check(&p, dvec3(-10.0, -500.0, -60.0), dvec3(300.0, 900.0, 40.0)).is_empty());
        // Within the places it is written to is within.
        assert!(check(&p, dvec3(-10.0004, 0.0, 0.0), dvec3(0.0, 0.0, 0.0)).is_empty());
        let past = check(&p, dvec3(-10.0, 0.0, -75.5), dvec3(320.0, 0.0, 10.0));
        assert_eq!(past.len(), 2, "{past:?}");
        assert_eq!(
            past[0],
            "X goes from -10 to 320 mm from the work zero, 20 mm over GRBL's travel of \
             -10 to 300 mm"
        );
        assert!(
            past[1].starts_with("Z goes from -75.5 to 10 mm from the work zero, 15.5 mm under"),
            "{}",
            past[1]
        );
        let both = check(&p, dvec3(-12.0, 0.0, 0.0), dvec3(301.0, 0.0, 0.0));
        assert!(both[0].contains("2 mm under and 1 mm over"), "{both:?}");
        let mut inch = p.clone();
        inch.units = ProgramUnits::Inch;
        inch.travel = Some(Travel {
            x: Some([0.0, 10.0]),
            ..Travel::default()
        });
        let past = check(&inch, dvec3(0.0, 0.0, 0.0), dvec3(279.4, 0.0, 0.0));
        assert!(
            past[0].contains("to 11 in from the work zero, 1 in over"),
            "{past:?}"
        );
        let none = Profile::builtin("grbl").unwrap().unwrap();
        assert_eq!(
            none.travel_check(DVec3::splat(-1e6), DVec3::splat(1e6), &at),
            TravelCheck::default()
        );
    }

    /// In machine coordinates, a program is placed on the machine by where
    /// its work zero is there, and on Z by the tool's length when the
    /// controller takes length offsets; not knowing either is said, not
    /// passed.
    #[test]
    fn machine_travel_is_checked_from_the_work_offset() {
        use glam::dvec3;
        let travel =
            "[travel]\ncoordinates = \"machine\"\nx = [0, 300]\ny = [-200, 0]\nz = [-150, 0]";
        let grbl = grbl_with(travel).unwrap();
        let (low, high) = (dvec3(-5.0, 0.0, -20.0), dvec3(100.0, 50.0, 15.0));
        // Not known where the work zero is: a note, nothing failed.
        let unknown = grbl.travel_check(low, high, &Placement::default());
        assert!(unknown.past.is_empty());
        assert!(
            unknown.unchecked[0].contains("machine coordinates")
                && unknown.unchecked[0].contains("not checked"),
            "{unknown:?}"
        );
        // G54 at X10 Y-100 Z-60: X -5 to 100 is 5 to 110, inside; Y 0 to 50
        // is -100 to -50, inside; Z -20 to 15 is -80 to -45. GRBL takes no
        // length offset, so the tool's length is not added.
        let mut at = Placement {
            zero: Some(dvec3(10.0, -100.0, -60.0)),
            tool_length: Some(50.0),
        };
        assert_eq!(grbl.travel_check(low, high, &at), TravelCheck::default());
        // Moved 3 mm left, X goes past its lowest.
        at.zero = Some(dvec3(2.0, -100.0, -60.0));
        let past = grbl.travel_check(low, high, &at).past;
        assert_eq!(
            past,
            [
                "X goes from -3 to 102 mm in machine coordinates, 3 mm under GRBL's travel of \
              0 to 300 mm"
            ]
        );
        // With length offsets the gauge line is the tool's length above the
        // tip: Z -80 to -45 is -30 to 5 with a 50 mm tool, past the top.
        let mut g43 = grbl.clone();
        g43.length_offset = true;
        at.zero = Some(dvec3(10.0, -100.0, -60.0));
        let past = g43.travel_check(low, high, &at).past;
        assert_eq!(
            past,
            [
                "Z goes from -30 to 5 mm in machine coordinates, the tool 50 mm from the gauge \
              line, 5 mm over GRBL's travel of -150 to 0 mm"
            ]
        );
        // Its length not known, Z is not checked, and that is said.
        at.tool_length = None;
        let check = g43.travel_check(low, high, &at);
        assert!(check.past.is_empty());
        assert!(
            check.unchecked[0].starts_with("Z is not checked"),
            "{check:?}"
        );
    }

    /// A `G53` move of the profile's own goes where the machine goes in
    /// machine coordinates: outside its travel, the profile is not valid.
    #[test]
    fn a_profiles_own_machine_moves_keep_to_its_travel() {
        let machine = "[travel]\ncoordinates = \"machine\"\nz = [-150, -1]";
        let e = grbl_with(machine).unwrap_err();
        assert!(
            e.to_string()
                .contains("safe_retract line 1, a G53 move, sends Z to 0"),
            "{e}"
        );
        assert!(grbl_with("[travel]\ncoordinates = \"machine\"\nz = [-150, 0]").is_ok());
        // From the work zero, G53 is somewhere else.
        assert!(grbl_with("[travel]\nz = [-150, -1]").is_ok());
        let parked = "end_home = [\"G53 G0 X-5 Y0 (park)\"]\n\
                      [travel]\ncoordinates = \"machine\"\nx = [0, 300]";
        let e = grbl_with(parked).unwrap_err();
        assert!(e.to_string().contains("end_home line 1"), "{e}");
    }

    /// A work offset past G59 is the profile's, as it writes it; one it has
    /// not got is none.
    #[test]
    fn extended_work_offsets_are_the_profiles() {
        let grbl = Profile::builtin("grbl").unwrap().unwrap();
        assert_eq!(grbl.wcs_word(54).as_deref(), Some("G54"));
        assert_eq!(grbl.wcs_word(59).as_deref(), Some("G59"));
        assert_eq!(grbl.wcs_word(60), None);
        assert_eq!(grbl.wcs_word(53), None);
        assert_eq!(grbl.work_offsets().len(), 6);
        let linuxcnc = Profile::builtin("linuxcnc").unwrap().unwrap();
        assert_eq!(linuxcnc.wcs_word(60).as_deref(), Some("G59.1"));
        assert_eq!(linuxcnc.wcs_word(62).as_deref(), Some("G59.3"));
        assert_eq!(linuxcnc.wcs_word(63), None);
        assert_eq!(linuxcnc.work_offsets().len(), 9);
        let fanuc = grbl_with("extended_offsets = [\"G54.1 P1\", \"G54.1 P2\"]").unwrap();
        assert_eq!(fanuc.wcs_word(61).as_deref(), Some("G54.1 P2"));
        for bad in ["\"\"", "\"M7\"", "\"G59.1 (x)\""] {
            assert!(
                grbl_with(&format!("extended_offsets = [{bad}]")).is_err(),
                "{bad}"
            );
        }
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
