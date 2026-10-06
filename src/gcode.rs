//! A pocket's toolpath as a G-code program: millimetres, absolute
//! coordinates, the spindle on for the cut. Every run starts from the
//! clearance height over the stock, goes down at the plunge rate, cuts at
//! the feed rate and rises back before the next.

use crate::Pocket;
use crate::path::Toolpath;

/// The program for `path`, the toolpath of the operation `name`.
pub fn write(name: &str, pocket: &Pocket, path: &Toolpath) -> String {
    // Parentheses end a comment early.
    let plain = |text: &str| text.replace(['(', ')'], "");
    let safe = pocket.top + pocket.clearance;
    let floor = path.levels.last().copied().unwrap_or(pocket.top);
    let mut g = String::new();
    let mut line = |text: String| {
        g.push_str(&text);
        g.push('\n');
    };
    line("%".into());
    line(format!("(printCAD CAM: {})", plain(name)));
    line(format!(
        "(tool: {}, diameter {:.3} mm)",
        plain(&pocket.tool),
        pocket.tool_diameter
    ));
    line(format!(
        "(stock top Z{:.3}, floor Z{floor:.3}, {} passes)",
        pocket.top,
        path.levels.len()
    ));
    line("G21 (millimetres)".into());
    line("G90 (absolute)".into());
    line("G17 (XY plane)".into());
    line(format!("G0 Z{safe:.3}"));
    line(format!("S{:.0} M3", pocket.spindle));
    for (pass, z) in path.levels.iter().enumerate() {
        line(format!(
            "(pass {} of {} at Z{z:.3})",
            pass + 1,
            path.levels.len()
        ));
        for run in &path.runs {
            let Some((first, rest)) = run.split_first() else {
                continue;
            };
            line(format!("G0 X{:.3} Y{:.3}", first[0], first[1]));
            line(format!("G1 Z{z:.3} F{:.0}", pocket.plunge));
            let mut moves = rest.iter();
            if let Some(p) = moves.next() {
                line(format!("G1 X{:.3} Y{:.3} F{:.0}", p[0], p[1], pocket.feed));
            }
            for p in moves {
                line(format!("G1 X{:.3} Y{:.3}", p[0], p[1]));
            }
            line(format!("G0 Z{safe:.3}"));
        }
    }
    line("M5".into());
    line("M30".into());
    line("%".into());
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::{Input, plan};

    fn pocket() -> Pocket {
        Pocket {
            sketch: None,
            outline: vec![vec![[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]]],
            top: 2.0,
            depth: 2.5,
            step_down: 1.0,
            tool: "Flat (3) mm".into(),
            tool_diameter: 3.0,
            stepover: 1.2,
            feed: 600.0,
            plunge: 200.0,
            spindle: 16000.0,
            clearance: 5.0,
        }
    }

    /// The program's moves: (G code, X, Y, Z), each axis left out of the
    /// line kept from the line before.
    fn moves(program: &str) -> Vec<(u32, f64, f64, f64)> {
        let (mut x, mut y, mut z) = (f64::NAN, f64::NAN, f64::NAN);
        let mut out = Vec::new();
        for line in program.lines() {
            let code = match line.find('(') {
                Some(i) => &line[..i],
                None => line,
            };
            let mut g = None;
            for word in code.split_whitespace() {
                let (letter, number) = word.split_at(1);
                let value: f64 = match number.parse() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                match letter {
                    "G" if value == 0.0 || value == 1.0 => g = Some(value as u32),
                    "X" => x = value,
                    "Y" => y = value,
                    "Z" => z = value,
                    _ => {}
                }
            }
            if let Some(g) = g {
                out.push((g, x, y, z));
            }
        }
        out
    }

    #[test]
    fn rapids_only_at_clearance_and_cuts_down_to_the_floor() {
        let p = pocket();
        let input = Input {
            loops: p.outline.clone(),
            radius: p.tool_diameter / 2.0,
            stepover: p.stepover,
            top: p.top,
            depth: p.depth,
            step_down: p.step_down,
        };
        let path = plan(&input, |_, _| true).unwrap();
        let program = write("Pocket (1)", &p, &path);
        assert!(program.contains("G21") && program.contains("G90"));
        assert!(program.contains("(printCAD CAM: Pocket 1)"));
        assert!(program.contains("M3") && program.contains("M5") && program.contains("M30"));

        let safe = p.top + p.clearance;
        let moves = moves(&program);
        assert!(!moves.is_empty());
        // The tool rises to the clearance before it goes anywhere.
        assert!(moves[0].0 == 0 && moves[0].1.is_nan() && (moves[0].3 - safe).abs() < 1e-3);
        let mut previous: Option<(f64, f64, f64)> = None;
        let mut deepest = f64::INFINITY;
        for &(g, x, y, z) in &moves {
            match g {
                0 => {
                    // A rapid either rises to the clearance or moves across
                    // at it.
                    assert!((z - safe).abs() < 1e-3, "a rapid at Z{z}");
                }
                _ => {
                    assert!(z <= p.top + 1e-3, "a cut above the stock at Z{z}");
                    if let Some((px, py, pz)) = previous {
                        let across = (x - px).abs() > 1e-9 || (y - py).abs() > 1e-9;
                        assert!(
                            !(across && (z - pz).abs() > 1e-9),
                            "a cut moving in X, Y and Z"
                        );
                    }
                    deepest = deepest.min(z);
                }
            }
            previous = Some((x, y, z));
        }
        assert!(
            (deepest - (p.top - p.depth)).abs() < 1e-3,
            "floor at {deepest}"
        );
        // Each pass cuts at its level, top down.
        let mut levels: Vec<f64> = moves.iter().filter(|m| m.0 == 1).map(|m| m.3).collect();
        levels.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
        assert_eq!(levels.len(), path.levels.len());
        for (cut, level) in levels.iter().zip(&path.levels) {
            assert!((cut - level).abs() < 1e-3);
        }
    }
}
