//! Custom G-code: a person's own lines, written into a program where a
//! Custom G-code operation stands in its setup.
//!
//! The lines are passed through as typed, so nothing can check what they do
//! the way the rest of a program is checked. What they may do is kept to
//! what cannot move the machine or change how the rest of the program is
//! read: comments, pauses (`M0`, `M1`), the spindle and coolant, a dwell
//! (`G4`), outputs and other M codes, and words such as `S`, `F`, `T` or `P`.
//! Refused, as a failed check:
//!
//! - an axis word (`X`, `Y`, `Z`, `A`, `B`, `C`, `U`, `V`, `W`): a move;
//! - any G code but `G4`: each moves the machine (`G0`, `G28`, `G53`, a
//!   cycle), or changes how the program's numbers are read (`G91`, `G20`,
//!   `G18`, `G92`, `G10`, `G49`), and a program read from then on that way
//!   cuts somewhere else;
//! - `M6` and `M60`, which move the machine to change a tool or a pallet, and
//!   `M2`, `M30` and `M99`, which end the program before its end.
//!
//! A call of a subprogram or macro (an `O` word, `M98`, `G65`, `G66`, or a
//! user M code from `M100`) can do anything and is not read: warned about.
//!
//! The program forgets where the machine is after the lines, as after a
//! tool change, and stops the spindle and coolant before going on.

/// What is wrong with a line of custom G-code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Concern {
    /// It does something custom G-code may not: refused.
    Refused(String),
    /// It calls something that is not read: warned about.
    Unread(String),
}

/// One word of a line: its letter, in capitals, and its number, or `None`
/// for one given as an expression (`X#1`, `X[2*3]`).
fn words(line: &str) -> Vec<(char, Option<f64>, String)> {
    // Comments left out: `(...)`, and `;` to the line's end.
    let mut code = String::new();
    let mut depth = 0;
    for c in line.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            ';' if depth == 0 => break,
            c if depth == 0 => code.push(c),
            _ => {}
        }
    }
    let chars: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        i += 1;
        if !c.is_ascii_alphabetic() {
            continue;
        }
        // A word is a letter and a number, spaces between allowed; a letter
        // followed by another is part of a name (`call`, `sub`), not a word.
        let mut j = i;
        while chars.get(j).is_some_and(|c| *c == ' ' || *c == '\t') {
            j += 1;
        }
        match chars.get(j) {
            Some(&e @ ('#' | '[')) => {
                out.push((c.to_ascii_uppercase(), None, format!("{c}{e}")));
                i = j + 1;
            }
            Some(d) if d.is_ascii_digit() || matches!(d, '.' | '-' | '+') => {
                let mut number = String::new();
                while let Some(&d) = chars.get(j) {
                    if d.is_ascii_digit() || matches!(d, '.' | '-' | '+') {
                        number.push(d);
                        j += 1;
                    } else {
                        break;
                    }
                }
                let value = number.parse::<f64>().ok();
                let shown = format!("{}{number}", c.to_ascii_uppercase());
                // A sign or a point alone is no number: not a word.
                if value.is_some() {
                    out.push((c.to_ascii_uppercase(), value, shown));
                }
                i = j;
            }
            _ => {}
        }
    }
    out
}

/// What is wrong with `line`, as custom G-code, if anything: the first thing
/// refused in it, or else the first call that is not read.
pub fn concern(line: &str) -> Option<Concern> {
    let mut unread = None;
    for (letter, value, shown) in words(line) {
        let refused = |why: &str| Some(Concern::Refused(format!("{shown} {why}")));
        // Codes are compared whole: `G38.2` is no `G38`.
        let is = |codes: &[f64]| value.is_some_and(|v| codes.iter().any(|c| (v - c).abs() < 1e-6));
        match letter {
            'X' | 'Y' | 'Z' | 'A' | 'B' | 'C' | 'U' | 'V' | 'W' => {
                return refused("moves the machine: custom G-code may not move it");
            }
            'G' if is(&[4.0]) => {}
            'G' if is(&[65.0, 66.0]) => {
                unread.get_or_insert(shown);
            }
            'G' => {
                return refused(
                    "is refused in custom G-code: only G4, a dwell, is let through, as any \
                     other G code moves the machine or changes how the rest of the program \
                     is read",
                );
            }
            'M' if is(&[6.0, 60.0]) => {
                return refused("changes a tool or a pallet, which moves the machine");
            }
            'M' if is(&[2.0, 30.0, 99.0]) => {
                return refused("ends the program before its end");
            }
            'M' if is(&[98.0]) || value.is_none_or(|m| m >= 100.0) => {
                unread.get_or_insert(shown);
            }
            'O' => {
                unread.get_or_insert(shown);
            }
            _ => {}
        }
    }
    unread.map(|shown| {
        Concern::Unread(format!(
            "{shown} calls a subprogram or macro: what it does is not checked"
        ))
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    fn refused(line: &str) -> bool {
        matches!(concern(line), Some(Concern::Refused(_)))
    }

    #[test]
    fn what_cannot_move_the_machine_goes_through() {
        for line in [
            "M0",
            "M1 (check the clamps)",
            "(MSG, turn the part over)",
            "M7",
            "M9",
            "G4 P2.5",
            "M64 P1",
            "S12000 M3",
            "M5",
            "T4",
            "; a comment with X10 in it",
            "",
        ] {
            assert_eq!(concern(line), None, "{line}");
        }
    }

    #[test]
    fn motion_and_modes_are_refused() {
        for line in [
            "G0 Z5",
            "X10",
            "g1x5f100",
            "G28",
            "G53 G0 Z0",
            "G91",
            "G20",
            "G92 X0",
            "G38.2 Z-10 F50",
            "M6 T2",
            "M30",
            "Z#1",
            "A 90",
        ] {
            assert!(refused(line), "{line}: {:?}", concern(line));
        }
        let Some(Concern::Refused(why)) = concern("G0 Z5") else {
            panic!()
        };
        assert!(why.starts_with("G0 "), "{why}");
    }

    /// Words of a name are not words: `call` is no `C` axis.
    #[test]
    fn calls_are_warned_about_not_refused() {
        for line in ["o100 call", "M98 P1000", "M101", "G65 P9810"] {
            assert!(
                matches!(concern(line), Some(Concern::Unread(_))),
                "{line}: {:?}",
                concern(line)
            );
        }
        // A refusal wins over a call beside it.
        assert!(refused("O100 call X5"));
    }
}
