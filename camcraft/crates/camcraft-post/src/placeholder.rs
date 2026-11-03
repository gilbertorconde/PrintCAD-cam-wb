//! `@name@` placeholders in a profile's lines and in custom G-code.
//!
//! A profile's header, footer, safe retract, end home and tool-change lines,
//! and a Custom G-code operation's text, can name a value of the program they
//! go into: `@tool@` is written as the tool's number. `@@` is an `@` itself.
//! The header's old `{name}` is the program's name still.
//!
//! A name that is not one of [`PLACEHOLDERS`] is refused when the profile is
//! loaded, not written as it stands: a block that reads `T@tol@ M6` must not
//! reach the machine as written.

use camcraft_core::job::{Coolant, SpindleDirection};

use crate::profile::ProgramUnits;

/// Every placeholder, and what it is replaced with.
pub const PLACEHOLDERS: &[(&str, &str)] = &[
    ("name", "the program's name"),
    ("operation", "the operation's name"),
    ("strategy", "its strategy, as a job file names it"),
    ("tool", "the tool's number"),
    ("tool_comment", "the tool's description"),
    ("rpm", "the spindle's speed, rev/min"),
    ("spindle", "M3 or M4: the way the spindle turns"),
    ("coolant", "flood, mist or off"),
    ("coolant_code", "M8, M7 or M9"),
    ("units", "mm or inch"),
    ("units_code", "G21 or G20"),
    ("wcs", "the work offset, G54 to G59"),
    (
        "date",
        "the day it was posted, 2026-11-03, where it is known; else nothing",
    ),
];

/// What the placeholders stand for, in one program.
#[derive(Debug, Clone, PartialEq)]
pub struct Values {
    /// The program's name.
    pub name: String,
    /// The operation's name.
    pub operation: String,
    /// Its strategy, as a job file names it.
    pub strategy: String,
    /// The tool's number.
    pub tool: u32,
    /// The tool's description.
    pub tool_comment: String,
    /// The spindle's speed, rev/min.
    pub rpm: u32,
    /// Which way the spindle turns.
    pub spindle: SpindleDirection,
    /// The coolant.
    pub coolant: Coolant,
    /// The program's units.
    pub units: ProgramUnits,
    /// The work offset, 54 for `G54`.
    pub wcs: u8,
    /// The day it was posted, where that is known.
    pub date: Option<String>,
}

impl Default for Values {
    fn default() -> Self {
        Self {
            name: String::new(),
            operation: String::new(),
            strategy: String::new(),
            tool: 0,
            tool_comment: String::new(),
            rpm: 0,
            spindle: SpindleDirection::Clockwise,
            coolant: Coolant::Off,
            units: ProgramUnits::Mm,
            wcs: 54,
            date: None,
        }
    }
}

impl Values {
    /// The value of placeholder `name`, or `None` for one that is not a
    /// placeholder. Words a person typed come through `text`, which makes
    /// them safe to stand in a comment.
    fn get(&self, name: &str, text: &dyn Fn(&str) -> String) -> Option<String> {
        Some(match name {
            "name" => text(&self.name),
            "operation" => text(&self.operation),
            "strategy" => text(&self.strategy),
            "tool" => self.tool.to_string(),
            "tool_comment" => text(&self.tool_comment),
            "rpm" => self.rpm.to_string(),
            "spindle" => match self.spindle {
                SpindleDirection::Clockwise => "M3",
                SpindleDirection::CounterClockwise => "M4",
            }
            .into(),
            "coolant" => match self.coolant {
                Coolant::Flood => "flood",
                Coolant::Mist => "mist",
                Coolant::Off => "off",
            }
            .into(),
            "coolant_code" => match self.coolant {
                Coolant::Flood => "M8",
                Coolant::Mist => "M7",
                Coolant::Off => "M9",
            }
            .into(),
            "units" => match self.units {
                ProgramUnits::Mm => "mm",
                ProgramUnits::Inch => "inch",
            }
            .into(),
            "units_code" => match self.units {
                ProgramUnits::Mm => "G21",
                ProgramUnits::Inch => "G20",
            }
            .into(),
            "wcs" => format!("G{}", self.wcs),
            "date" => self.date.clone().unwrap_or_default(),
            _ => return None,
        })
    }
}

/// One piece of a line: as written, or a placeholder.
enum Piece<'a> {
    Text(&'a str),
    Named(&'a str),
}

/// `line` in pieces, or what is wrong with it: an `@` never closed.
fn pieces(line: &str) -> Result<Vec<Piece<'_>>, String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some((before, after)) = rest.split_once('@') {
        out.push(Piece::Text(before));
        let Some((name, next)) = after.split_once('@') else {
            return Err(format!(
                "an @ is never closed in {line:?}: a placeholder is @name@, and @@ is an @ itself"
            ));
        };
        out.push(Piece::Named(name));
        rest = next;
    }
    out.push(Piece::Text(rest));
    Ok(out)
}

/// What is wrong with `line`'s placeholders, if anything: one that is not a
/// placeholder, or an `@` never closed.
pub fn problem(line: &str) -> Option<String> {
    let pieces = match pieces(line) {
        Ok(p) => p,
        Err(why) => return Some(why),
    };
    pieces.iter().find_map(|p| match p {
        Piece::Named(name) if !name.is_empty() && !known(name) => Some(format!(
            "@{name}@ is not a placeholder; there are {}",
            PLACEHOLDERS
                .iter()
                .map(|(n, _)| format!("@{n}@"))
                .collect::<Vec<_>>()
                .join(", ")
        )),
        _ => None,
    })
}

fn known(name: &str) -> bool {
    PLACEHOLDERS.iter().any(|(n, _)| *n == name)
}

/// `line` with its placeholders replaced by `values`, words a person typed
/// made safe by `text`, and `{name}` replaced with the program's name, as a
/// header always had it. A line with a problem ([`problem`]) is written as it
/// stands: a profile or a job is checked for them before it gets here.
pub fn fill(line: &str, values: &Values, text: &dyn Fn(&str) -> String) -> String {
    let Ok(pieces) = pieces(line) else {
        return line.to_string();
    };
    let mut out = String::with_capacity(line.len());
    for p in pieces {
        match p {
            Piece::Text(t) => out.push_str(&t.replace("{name}", &text(&values.name))),
            Piece::Named("") => out.push('@'),
            Piece::Named(name) => match values.get(name, text) {
                Some(v) => out.push_str(&v),
                None => {
                    out.push('@');
                    out.push_str(name);
                    out.push('@');
                }
            },
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn values() -> Values {
        Values {
            name: "top-01-rough".into(),
            operation: "Rough (top)".into(),
            strategy: "adaptive".into(),
            tool: 3,
            tool_comment: "6mm flat".into(),
            rpm: 18000,
            spindle: SpindleDirection::CounterClockwise,
            coolant: Coolant::Mist,
            units: ProgramUnits::Inch,
            wcs: 55,
            date: Some("2026-11-03".into()),
        }
    }

    #[test]
    fn every_placeholder_is_replaced() {
        let v = values();
        let same = |s: &str| s.to_string();
        let all: String = PLACEHOLDERS
            .iter()
            .map(|(n, _)| format!("@{n}@"))
            .collect::<Vec<_>>()
            .join("|");
        assert_eq!(problem(&all), None);
        assert_eq!(
            fill(&all, &v, &same),
            "top-01-rough|Rough (top)|adaptive|3|6mm flat|18000|M4|mist|M7|inch|G20|G55|2026-11-03"
        );
    }

    #[test]
    fn braces_and_doubled_ats_read_as_they_always_did() {
        let v = values();
        let same = |s: &str| s.to_string();
        assert_eq!(fill("({name})", &v, &same), "(top-01-rough)");
        assert_eq!(fill("(mail a@@b)", &v, &same), "(mail a@b)");
        assert_eq!(fill("T@tool@ M6", &v, &same), "T3 M6");
    }

    /// Typed words go through the comment's escaping; numbers do not need it.
    #[test]
    fn typed_words_are_made_safe() {
        let v = values();
        let safe = |s: &str| s.replace('(', "[").replace(')', "]");
        assert_eq!(fill("(@operation@)", &v, &safe), "(Rough [top])");
    }

    #[test]
    fn unknown_and_unclosed_placeholders_are_problems() {
        let unknown = problem("T@tol@ M6").unwrap();
        assert!(unknown.contains("@tol@ is not a placeholder"), "{unknown}");
        assert!(
            unknown.contains("@tool@"),
            "it lists the known ones: {unknown}"
        );
        let open = problem("(mail me@home)").unwrap();
        assert!(open.contains("never closed"), "{open}");
        assert_eq!(problem("(no placeholders)"), None);
    }

    /// With no date given, it is left out, not written as a placeholder.
    #[test]
    fn a_date_not_known_is_nothing() {
        let v = Values {
            date: None,
            ..values()
        };
        assert_eq!(fill("(posted @date@)", &v, &|s| s.to_string()), "(posted )");
    }
}
