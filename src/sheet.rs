//! A setup's sheet: what goes to the shop floor with its program, as
//! Fusion's setup sheet does. The part, its stock and material, where the
//! work zero is and what the work offset holds, how the part sits, the
//! program, each tool and each operation, how long it all takes, and what
//! the checks found: one HTML file, its style inside it, to print.

use std::fmt::Write as _;

use cam_protocol::{Level, OperationRun, Run, ToolUse};
use camcraft_core::job::{Programs, StrategySpec};

use crate::data::{CylinderAxis, SetupData, StockKind, ToolAxis, strategy_label};

/// What a setup's sheet is made from.
pub struct Sheet<'a> {
    /// The setup's name.
    pub name: &'a str,
    pub setup: &'a SetupData,
    /// Its bodies' names.
    pub bodies: Vec<String>,
    /// Its run.
    pub run: &'a Run,
    /// The earlier setup its stock carries on from, by name, when it does.
    pub carried_from: Option<String>,
}

/// The sheet's look: plain, small and black on white, so it prints as it
/// shows, a table never split across a page where it can be kept whole.
const STYLE: &str = "\
body{font:10pt/1.35 system-ui,sans-serif;color:#000;background:#fff;margin:1.5em}
h1{font-size:15pt;margin:0 0 .2em}
h2{font-size:11pt;margin:1.2em 0 .3em;border-bottom:1px solid #000}
.sub{color:#444;margin:0 0 .8em}
.fail{border:2px solid #b00;color:#b00;padding:.3em .5em;font-weight:bold}
.pass{border:1px solid #070;color:#070;padding:.3em .5em}
table{border-collapse:collapse;width:100%;break-inside:avoid}
th,td{border:1px solid #999;padding:.15em .35em;text-align:left;vertical-align:top}
th{background:#eee}
td.n{text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap}
dl{display:grid;grid-template-columns:max-content auto;gap:.1em 1em;margin:0}
dt{color:#444}dd{margin:0}
.note{color:#444;font-size:9pt}
ul{margin:.2em 0;padding-left:1.4em}
@media print{body{margin:0}h2{break-after:avoid}}
";

impl Sheet<'_> {
    /// The sheet, as a whole HTML page.
    pub fn html(&self) -> String {
        let run = self.run;
        let mut out = String::new();
        let title = format!("Setup sheet: {}", self.name);
        let _ = write!(
            out,
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             <title>{}</title>\n<style>\n{STYLE}</style>\n</head>\n<body>\n<h1>{}</h1>\n",
            esc(&title),
            esc(&title)
        );
        let date = run.layout.as_ref().map_or("", |l| l.date.as_str());
        let _ = writeln!(
            out,
            "<p class=\"sub\">{}{}</p>",
            esc(&self.program_line()),
            if date.is_empty() {
                String::new()
            } else {
                format!(" · run {}", esc(date))
            }
        );
        out.push_str(&self.verdict());
        out.push_str(&self.part());
        out.push_str(&self.zero());
        out.push_str(&self.program());
        out.push_str(&self.time());
        out.push_str(&self.tools());
        out.push_str(&self.operations());
        out.push_str(&self.checks());
        out.push_str(
            "<p class=\"note\">Made by the CAM workbench with camcraft. Lengths in mm, \
             feeds in mm/min, speeds in rev/min; Z from the work zero, up the tool. \
             Check the program yourself before it cuts.</p>\n</body>\n</html>\n",
        );
        out
    }

    /// The program's name and number and the post, in a line.
    fn program_line(&self) -> String {
        let post = self.post();
        match &self.run.program {
            Some(p) => format!(
                "{}.{} · program {} · {post}",
                p.name,
                p.extension,
                self.number()
            ),
            None => {
                let n = self
                    .run
                    .operations
                    .iter()
                    .filter(|o| o.program.is_some())
                    .count();
                format!(
                    "{n} program(s), one an operation, numbered from {} · {post}",
                    self.number()
                )
            }
        }
    }

    fn number(&self) -> String {
        format!("{:.0}", self.setup.program_number)
    }

    fn post(&self) -> String {
        self.run
            .layout
            .as_ref()
            .map_or_else(|| self.setup.post.clone(), |l| l.post.clone())
    }

    /// Whether it may be run, at the top where it cannot be missed.
    fn verdict(&self) -> String {
        let failed = self
            .run
            .findings
            .iter()
            .filter(|f| f.level == Level::Fail)
            .count();
        if self.run.passed {
            "<p class=\"pass\">Verified against the material: every check passed.</p>\n".into()
        } else {
            format!(
                "<p class=\"fail\">{failed} check(s) failed: camcraft would not post this \
                 setup. Do not run it.</p>\n"
            )
        }
    }

    /// The part, its stock and its material.
    fn part(&self) -> String {
        let mut rows = vec![("Bodies", self.bodies.join(", "))];
        let material = self.setup.material.trim();
        rows.push((
            "Material",
            if material.is_empty() {
                "not given".into()
            } else {
                material.to_string()
            },
        ));
        if let Some(l) = &self.run.layout {
            rows.push(("Part", size(l.part)));
            rows.push(("Stock", format!("{}: {}", self.stock_kind(), size(l.stock))));
            rows.push(("Stock from the work zero", extents(l.stock)));
        } else {
            rows.push(("Stock", self.stock_kind()));
        }
        section("Part and stock", &list(&rows))
    }

    /// What kind of stock it is, in words.
    fn stock_kind(&self) -> String {
        let s = self.setup;
        match s.stock {
            StockKind::Around => format!(
                "a box round the bodies, {} mm more each side, {} on top, {} below",
                num(s.margin_side),
                num(s.margin_top),
                num(s.margin_below)
            ),
            StockKind::Fixed => "a box of a fixed size".into(),
            StockKind::Cylinder => {
                let along = match s.cylinder_axis {
                    CylinderAxis::Tool => "up the tool",
                    CylinderAxis::X => "along machine X",
                    CylinderAxis::Y => "along machine Y",
                };
                match &self.run.cylinder {
                    Some(c) => {
                        let [a, b] = c.ends;
                        let length = (0..3).map(|i| (b[i] - a[i]).powi(2)).sum::<f64>().sqrt();
                        format!(
                            "a round bar {along}, Ø{} × {} mm",
                            num(2.0 * c.radius),
                            num(length)
                        )
                    }
                    None => format!("a round bar {along}"),
                }
            }
            StockKind::Earlier => format!(
                "what {} leaves after its last operation, turned as this setup sees \
                 the part; its size is the box round that",
                self.carried_from
                    .as_deref()
                    .map_or_else(|| "the setup before".to_string(), |n| format!("setup {n}"))
            ),
        }
    }

    /// Where the work zero is, what the work offset holds, and how the part
    /// sits.
    fn zero(&self) -> String {
        let s = self.setup;
        let mut rows = Vec::new();
        let offset = [s.offset_x, s.offset_y, s.offset_z];
        let mut zero = s.zero.label().to_string();
        if offset.iter().any(|v| *v != 0.0) {
            let _ = write!(zero, ", moved {}", xyz(offset));
        }
        rows.push(("Work zero", zero));
        let mut wcs = s.wcs_word();
        if s.wcs_known {
            let gauge = s.profile().is_some_and(|p| p.length_offset);
            let _ = write!(
                wcs,
                ": the work zero at {} in machine coordinates{}",
                xyz([s.wcs_x, s.wcs_y, s.wcs_z]),
                if gauge {
                    ", Z the spindle's gauge line's (tool lengths by G43)"
                } else {
                    ", Z the tool's tip's"
                }
            );
        } else {
            wcs.push_str(": set it where the work zero is");
        }
        rows.push(("Work offset", wcs));
        let layout = self.run.layout.as_ref();
        let tilt = layout.map(|l| l.tilt.as_slice()).unwrap_or_default();
        let axis = layout.map_or_else(|| s.up(), |l| l.tool_axis);
        let sits = if s.tool_axis == ToolAxis::Square {
            format!(
                "square: the tool up the document's {}, machine X along its {}",
                s.z_axis.label(),
                s.x_axis.label()
            )
        } else {
            format!(
                "tilted ({}): the tool up {} of the document",
                s.tool_axis.label().to_lowercase(),
                direction(axis)
            )
        };
        rows.push(("Tool axis", sits));
        let mut body = list(&rows);
        if !tilt.is_empty() {
            body.push_str("<ul>\n");
            for line in tilt {
                let _ = writeln!(body, "<li>{}</li>", esc(line));
            }
            body.push_str("</ul>\n");
        }
        section("Work zero and orientation", &body)
    }

    /// The program or programs, and the post.
    fn program(&self) -> String {
        let run = self.run;
        let mut rows = vec![("Post", self.post())];
        match &run.program {
            Some(p) => {
                rows.push(("Program", format!("{}.{}", p.name, p.extension)));
                rows.push(("Number", self.number()));
                rows.push(("Lines", p.lines.to_string()));
            }
            None => {
                let names: Vec<String> = run
                    .operations
                    .iter()
                    .filter_map(|o| o.program.as_ref())
                    .map(|p| format!("{}.{}", p.name, p.extension))
                    .collect();
                rows.push((
                    "Programs",
                    if names.is_empty() {
                        "none posted".into()
                    } else {
                        names.join(", ")
                    },
                ));
                if self.setup.programs == Programs::Operation {
                    rows.push(("Numbered from", self.number()));
                }
            }
        }
        section("Program", &list(&rows))
    }

    /// How long it machines, and what that leaves out.
    fn time(&self) -> String {
        let m = &self.run.machining;
        let mut rows = vec![("Moves", duration(m.moves))];
        rows.push((
            "Tool changes",
            match m.tool_change {
                Some(each) => format!(
                    "{} × {} = {}",
                    m.tool_changes,
                    duration(each),
                    duration(f64::from(m.tool_changes) * each)
                ),
                None => format!(
                    "{}, not counted: the post does not say how long",
                    m.tool_changes
                ),
            },
        ));
        rows.push(("Total", duration(m.total())));
        let [x, y, z] = m.rapid;
        let mut body = list(&rows);
        let _ = writeln!(
            body,
            "<p class=\"note\">Moves at their feeds, and rapids at X {}, Y {}, Z {} mm/min{}, \
             none quicker than its axes go. Nothing speeds up or slows down here: a machine \
             does, so it takes longer, the more so the shorter its moves.</p>",
            num(x),
            num(y),
            num(z),
            if m.rapid_given {
                ", as the post gives them"
            } else {
                ", assumed: the post does not say"
            }
        );
        section("Machining time", &body)
    }

    /// Every tool, with what it does.
    fn tools(&self) -> String {
        let run = self.run;
        if run.tools.is_empty() {
            return section("Tools", "<p>None.</p>\n");
        }
        let mut t = String::from(
            "<table>\n<tr><th>T</th><th>Tool</th><th>Kind</th><th>Ø</th><th>Flutes</th>\
             <th>Flute length</th><th>Stickout</th><th>Length</th><th>Holder</th>\
             <th>Operations</th><th>Lowest Z</th><th>Fastest feed</th><th>Speed</th>\
             <th>Time</th></tr>\n",
        );
        for tool in &run.tools {
            let ops: Vec<String> = tool
                .operations
                .iter()
                .filter_map(|&k| run.operations.get(k))
                .map(|o| o.name.clone())
                .collect();
            let _ = writeln!(
                t,
                "<tr><td class=\"n\">{}</td><td>{}</td><td>{}</td><td class=\"n\">{}</td>\
                 <td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td>\
                 <td class=\"n\">{}</td><td>{}</td><td>{}</td><td class=\"n\">{}</td>\
                 <td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>",
                tool.number,
                esc(&tool_name(tool)),
                esc(&kind(tool)),
                num(tool.diameter),
                tool.flutes.map_or_else(dash, |f| f.to_string()),
                tool.flute_length.map_or_else(dash, num),
                tool.stickout.map_or_else(dash, num),
                tool.overall_length.map_or_else(dash, num),
                esc(tool.holder.as_deref().unwrap_or("—")),
                esc(&ops.join(", ")),
                tool.lowest.map_or_else(dash, num),
                num(tool.fastest_feed),
                tool.fastest_rpm,
                duration(tool.seconds),
            );
        }
        t.push_str("</table>\n");
        section("Tools", &t)
    }

    /// Each operation, in order, as it cuts.
    fn operations(&self) -> String {
        let run = self.run;
        let mut t = String::from(
            "<table>\n<tr><th>#</th><th>Operation</th><th>Strategy</th><th>T</th>\
             <th>Speed</th><th>Feed</th><th>Plunge</th><th>Stepover</th><th>Stepdown</th>\
             <th>Stock to leave</th><th>Z from</th><th>Z to</th><th>Time</th></tr>\n",
        );
        for (i, op) in run.operations.iter().enumerate() {
            let _ = writeln!(t, "{}", operation_row(i, op));
        }
        t.push_str("</table>\n");
        let _ = writeln!(
            t,
            "<p class=\"note\">Stock to leave beside, then below. Z from the lowest to the \
             highest the tool's tip goes.</p>"
        );
        section("Operations", &t)
    }

    /// What the checks found, failures first, and each operation's own
    /// warnings.
    fn checks(&self) -> String {
        let run = self.run;
        let mut items = Vec::new();
        for f in run.findings.iter().filter(|f| f.level != Level::Pass) {
            let mark = match f.level {
                Level::Fail => "Failed",
                Level::Warn => "Warning",
                _ => "Note",
            };
            items.push(format!("<b>{mark}</b>: {}", esc(&f.text)));
        }
        for op in &run.operations {
            for w in &op.warnings {
                items.push(format!("<b>Warning</b>, {}: {}", esc(&op.name), esc(w)));
            }
        }
        for w in &run.warnings {
            items.push(format!("<b>Warning</b>: {}", esc(w)));
        }
        if items.is_empty() {
            return section("Checks", "<p>Nothing to say.</p>\n");
        }
        let mut body = String::from("<ul>\n");
        for item in items {
            let _ = writeln!(body, "<li>{item}</li>");
        }
        body.push_str("</ul>\n");
        section("Checks", &body)
    }
}

/// An operation's row of the operations table.
fn operation_row(i: usize, op: &OperationRun) -> String {
    // An operation's own settings go under its name.
    let name = match op.cutting.as_ref().map(|c| c.notes.join("; ")) {
        Some(notes) if !notes.is_empty() => format!(
            "{}<br><span class=\"note\">{}</span>",
            esc(&op.name),
            esc(&notes)
        ),
        _ => esc(&op.name),
    };
    let Some(c) = &op.cutting else {
        let what = match op.made.as_str() {
            "custom-gcode" => op.summary.clone(),
            "disabled" => "turned off: not cut, nor in the program".into(),
            made => format!("{made}: no toolpath"),
        };
        return format!(
            "<tr><td class=\"n\">{}</td><td>{name}</td><td colspan=\"10\">{}</td>\
             <td class=\"n\">{}</td></tr>",
            i + 1,
            esc(&what),
            if op.seconds > 0.0 {
                duration(op.seconds)
            } else {
                dash()
            }
        );
    };
    let strategy =
        StrategySpec::named(&c.strategy).map_or(c.strategy.as_str(), |s| strategy_label(&s));
    let fastest = if c.fastest_feed > c.feed.max(c.plunge_feed) + 1e-9 {
        format!(" (to {})", num(c.fastest_feed))
    } else {
        String::new()
    };
    let [low, high] = c.z.map_or([None, None], |[l, h]| [Some(l), Some(h)]);
    format!(
        "<tr><td class=\"n\">{}</td><td>{name}</td><td>{}</td><td class=\"n\">{}</td>\
         <td class=\"n\">{}</td><td class=\"n\">{}{}</td><td class=\"n\">{}</td>\
         <td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td>\
         <td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>",
        i + 1,
        esc(strategy),
        c.tool_number,
        c.rpm,
        num(c.feed),
        esc(&fastest),
        num(c.plunge_feed),
        c.stepover.map_or_else(dash, num),
        c.stepdown.map_or_else(dash, num),
        c.stock_to_leave
            .map_or_else(dash, |[r, a]| format!("{} / {}", num(r), num(a))),
        low.map_or_else(dash, num),
        high.map_or_else(dash, num),
        duration(op.seconds),
    )
}

/// A tool's name, and its description where it has one.
fn tool_name(t: &ToolUse) -> String {
    if t.comment.trim().is_empty() {
        t.name.clone()
    } else {
        format!("{} ({})", t.name, t.comment.trim())
    }
}

/// A tool's kind, with its corner or angle.
fn kind(t: &ToolUse) -> String {
    match t.kind.as_str() {
        "flat" => "flat end mill".into(),
        "ball" => "ball end mill".into(),
        "bullnose" => format!("bull-nose, r{}", num(t.corner)),
        "cone" => format!("V-bit, {}°", t.angle.map_or_else(dash, num)),
        "drill" => format!("drill, {}°", t.angle.map_or_else(dash, num)),
        "thread-mill" => match t.pitch_range {
            Some([a, b]) if a == b => format!("thread mill, {} mm pitch", num(a)),
            Some([a, b]) => format!("thread mill, {} to {} mm pitch", num(a), num(b)),
            None => "thread mill".into(),
        },
        other => other.into(),
    }
}

/// A section under its heading.
fn section(title: &str, body: &str) -> String {
    format!("<h2>{}</h2>\n{body}", esc(title))
}

/// Labels and their values, as a list.
fn list(rows: &[(&str, String)]) -> String {
    let mut out = String::from("<dl>\n");
    for (label, value) in rows {
        let _ = writeln!(out, "<dt>{}</dt><dd>{}</dd>", esc(label), esc(value));
    }
    out.push_str("</dl>\n");
    out
}

/// A box's size, X by Y by Z.
fn size([low, high]: [[f64; 3]; 2]) -> String {
    format!(
        "{} × {} × {} mm (X × Y × Z)",
        num(high[0] - low[0]),
        num(high[1] - low[1]),
        num(high[2] - low[2])
    )
}

/// Where a box goes along each axis.
fn extents([low, high]: [[f64; 3]; 2]) -> String {
    format!(
        "X {} to {}, Y {} to {}, Z {} to {}",
        num(low[0]),
        num(high[0]),
        num(low[1]),
        num(high[1]),
        num(low[2]),
        num(high[2])
    )
}

fn xyz([x, y, z]: [f64; 3]) -> String {
    format!("X {}, Y {}, Z {}", num(x), num(y), num(z))
}

/// A direction, to three places.
fn direction([x, y, z]: [f64; 3]) -> String {
    let short = |v: f64| {
        let s = format!("{v:.3}");
        if s == "-0.000" { "0.000".into() } else { s }
    };
    format!("({}, {}, {})", short(x), short(y), short(z))
}

/// A number to three places at most, without the zeros after them, and a
/// minus sign that is one.
pub fn num(v: f64) -> String {
    let mut s = format!("{v:.3}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".into();
    }
    s.replace('-', "−")
}

fn dash() -> String {
    "—".into()
}

/// A time in hours, minutes and seconds, as a person reads it: `45 s`,
/// `3 min 05 s`, `1 h 02 min 30 s`.
pub fn duration(seconds: f64) -> String {
    let whole = seconds.max(0.0).round() as u64;
    let (h, m, s) = (whole / 3600, whole / 60 % 60, whole % 60);
    match (h, m) {
        (0, 0) => format!("{s} s"),
        (0, m) => format!("{m} min {s:02} s"),
        (h, m) => format!("{h} h {m:02} min {s:02} s"),
    }
}

/// `text` safe to stand in HTML.
fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cam_protocol::{Cutting, Finding, Layout, Machining, Program};

    /// A setup of two operations with two tools and a Custom G-code
    /// between, run, as the helper answers.
    fn run() -> Run {
        let cut = |name: &str, seconds: f64, cutting: Cutting| OperationRun {
            name: name.into(),
            made: "generated".into(),
            seconds,
            cutting: Some(cutting),
            ..OperationRun::default()
        };
        Run {
            passed: false,
            findings: vec![
                Finding {
                    level: Level::Fail,
                    code: "CC107".into(),
                    text: "finish: rapid through material at move 12".into(),
                    operation: Some(2),
                    at_move: Some(12),
                },
                Finding {
                    level: Level::Pass,
                    code: "CC001".into(),
                    text: "no gouges".into(),
                    operation: None,
                    at_move: None,
                },
            ],
            operations: vec![
                cut(
                    "rough",
                    754.4,
                    Cutting {
                        strategy: "adaptive".into(),
                        tool: "1/4 upcut".into(),
                        tool_number: 1,
                        rpm: 18000,
                        feed: 1500.0,
                        plunge_feed: 500.0,
                        fastest_feed: 1500.0,
                        stepover: Some(0.635),
                        stepdown: Some(6.0),
                        stock_to_leave: Some([0.3, 0.2]),
                        z: Some([-12.0, 15.0]),
                        notes: vec!["rest machining: what the operations before left".into()],
                    },
                ),
                OperationRun {
                    name: "clamps".into(),
                    made: "custom-gcode".into(),
                    summary: "1 line(s) of custom G-code".into(),
                    ..OperationRun::default()
                },
                OperationRun {
                    warnings: vec!["passes left cutting <conventional>".into()],
                    ..cut(
                        "finish",
                        3725.0,
                        Cutting {
                            strategy: "parallel".into(),
                            tool: "1/8 ball".into(),
                            tool_number: 2,
                            rpm: 20000,
                            feed: 1200.0,
                            plunge_feed: 400.0,
                            fastest_feed: 2000.0,
                            stepover: Some(0.25),
                            stepdown: None,
                            stock_to_leave: Some([0.0, 0.0]),
                            z: Some([-11.5, 15.0]),
                            notes: Vec::new(),
                        },
                    )
                },
                cut(
                    "M5 thread",
                    42.0,
                    Cutting {
                        strategy: "thread".into(),
                        tool: "M5 thread mill".into(),
                        tool_number: 3,
                        rpm: 12000,
                        feed: 300.0,
                        plunge_feed: 300.0,
                        fastest_feed: 300.0,
                        stepover: None,
                        stepdown: None,
                        stock_to_leave: None,
                        z: Some([-10.0, 15.0]),
                        notes: vec!["thread 0.8 mm pitch, right-hand".into()],
                    },
                ),
                OperationRun {
                    name: "spare pocket".into(),
                    made: "disabled".into(),
                    ..OperationRun::default()
                },
            ],
            program: Some(Program {
                name: "top".into(),
                extension: "nc".into(),
                post: "GRBL".into(),
                lines: 4211,
                ..Program::default()
            }),
            machining: Machining {
                moves: 4479.4,
                tool_changes: 2,
                tool_change: Some(60.0),
                rapid: [5000.0, 5000.0, 2500.0],
                rapid_given: true,
            },
            tools: vec![
                ToolUse {
                    name: "1/4 upcut".into(),
                    number: 1,
                    kind: "flat".into(),
                    diameter: 6.35,
                    flutes: Some(2),
                    flute_length: Some(22.0),
                    stickout: Some(30.0),
                    holder: Some("ER20 collet".into()),
                    operations: vec![0],
                    lowest: Some(-12.0),
                    fastest_feed: 1500.0,
                    fastest_rpm: 18000,
                    seconds: 754.4,
                    ..ToolUse::default()
                },
                ToolUse {
                    name: "1/8 ball".into(),
                    number: 2,
                    kind: "ball".into(),
                    diameter: 3.175,
                    comment: "Amana 46282".into(),
                    operations: vec![2],
                    lowest: Some(-11.5),
                    fastest_feed: 2000.0,
                    fastest_rpm: 20000,
                    seconds: 3725.0,
                    ..ToolUse::default()
                },
                ToolUse {
                    name: "M5 thread mill".into(),
                    number: 3,
                    kind: "thread-mill".into(),
                    diameter: 3.9,
                    pitch_range: Some([0.8, 0.8]),
                    operations: vec![3],
                    lowest: Some(-10.0),
                    fastest_feed: 300.0,
                    fastest_rpm: 12000,
                    seconds: 42.0,
                    ..ToolUse::default()
                },
            ],
            layout: Some(Layout {
                stock: [[0.0, 0.0, -24.0], [104.0, 64.0, 0.0]],
                part: [[2.0, 2.0, -24.0], [102.0, 62.0, -2.0]],
                tool_axis: [0.0, 0.0, 1.0],
                tilt: Vec::new(),
                post: "GRBL".into(),
                date: "2026-11-07".into(),
                probe: None,
            }),
            ..Run::default()
        }
    }

    fn setup() -> SetupData {
        let mut s = SetupData::new(vec!["body".into()], "grbl", 0.05);
        s.material = "6061 aluminium & <friends>".into();
        s.wcs_known = true;
        (s.wcs_x, s.wcs_y, s.wcs_z) = (250.0, -180.0, -95.0);
        s
    }

    fn sheet() -> String {
        let run = run();
        let setup = setup();
        Sheet {
            name: "Top",
            setup: &setup,
            bodies: vec!["Bracket".into()],
            run: &run,
            carried_from: None,
        }
        .html()
    }

    /// The sheet says what the shop floor needs: the program, the stock and
    /// material, the work offset, each tool and operation and the time, and
    /// that a check failed, before anything else.
    #[test]
    fn a_sheet_says_what_the_shop_floor_needs() {
        let html = sheet();
        let at = |w: &str| {
            html.find(w)
                .unwrap_or_else(|| panic!("no {w:?} in\n{html}"))
        };
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.ends_with("</html>\n"));
        // Self-contained: nothing fetched.
        assert!(!html.contains("src=") && !html.contains("href="));
        // Failed, said first.
        assert!(at("1 check(s) failed") < at("<h2>Part and stock</h2>"));
        at("top.nc · program 1001 · GRBL · run 2026-11-07");
        at("6061 aluminium &amp; &lt;friends&gt;");
        at("104 × 64 × 24 mm (X × Y × Z)");
        at("100 × 60 × 22 mm (X × Y × Z)");
        at("X 0 to 104, Y 0 to 64, Z −24 to 0");
        at("G54: the work zero at X 250, Y −180, Z −95 in machine coordinates");
        // 4479.4 s of moves and two changes of a minute.
        at("<dt>Moves</dt><dd>1 h 14 min 39 s</dd>");
        at("2 × 1 min 00 s = 2 min 00 s");
        at("<dt>Total</dt><dd>1 h 16 min 39 s</dd>");
        at("X 5000, Y 5000, Z 2500 mm/min, as the post gives them");
        // Tools, in order, with what they do.
        assert!(at("1/4 upcut") < at("1/8 ball (Amana 46282)"));
        at("ER20 collet");
        // Operations, each as it cuts, the Custom G-code between.
        assert!(at("<td>Adaptive clearing (3D roughing)</td>") < at("1 line(s) of custom G-code"));
        assert!(at("1 line(s) of custom G-code") < at("<td>Parallel (3D finishing)</td>"));
        at("<td class=\"n\">1200 (to 2000)</td>");
        at("<td class=\"n\">0.3 / 0.2</td>");
        // An operation's own settings under its name; a thread mill's
        // pitch.
        at("rough<br><span class=\"note\">rest machining: what the operations before left</span>");
        at("M5 thread<br><span class=\"note\">thread 0.8 mm pitch, right-hand</span>");
        at("<td>thread mill, 0.8 mm pitch</td>");
        // One turned off is listed as off, taking no time.
        at("spare pocket</td><td colspan=\"10\">turned off: not cut, nor in the program</td>");
        // The checks, without the passes; the operation's own warnings.
        at("<b>Failed</b>: finish: rapid through material at move 12");
        assert!(!html.contains("no gouges"));
        at("<b>Warning</b>, finish: passes left cutting &lt;conventional&gt;");
    }

    /// The shank and holder check, failed, is among the checks, with the
    /// stickout that clears it.
    #[test]
    fn a_holder_that_meets_the_part_is_on_the_sheet() {
        let mut run = run();
        run.findings.insert(
            0,
            Finding {
                level: Level::Fail,
                code: "CC111".into(),
                text: "CC111 1. rough: holder segment 1 (Ø28.00) meets the part on 4 move(s) \
                       from move 40, 4.50 mm into it: needs 24.5 mm stickout"
                    .into(),
                operation: Some(0),
                at_move: Some(39),
            },
        );
        let setup = setup();
        let html = Sheet {
            name: "Top",
            setup: &setup,
            bodies: vec!["Bracket".into()],
            run: &run,
            carried_from: None,
        }
        .html();
        assert!(html.contains("2 check(s) failed"), "{html}");
        assert!(
            html.contains(
                "<b>Failed</b>: CC111 1. rough: holder segment 1 (Ø28.00) meets the part"
            ) && html.contains("needs 24.5 mm stickout"),
            "{html}"
        );
    }

    /// The sheet as it is written: `UPDATE_GOLDEN=1` writes it again, to be
    /// read before it is committed.
    #[test]
    fn a_sheet_reads_as_its_golden_file() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/setup-sheet.html");
        let html = sheet();
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(path, &html).unwrap();
        }
        let golden = std::fs::read_to_string(path).unwrap_or_default();
        assert!(golden == html, "the sheet differs from {path}:\n{html}");
    }

    #[test]
    fn times_read_as_a_person_says_them() {
        assert_eq!(duration(0.0), "0 s");
        assert_eq!(duration(44.6), "45 s");
        assert_eq!(duration(185.0), "3 min 05 s");
        assert_eq!(duration(3750.0), "1 h 02 min 30 s");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(12.5), "12.5");
        assert_eq!(num(-3.0), "−3");
    }

    /// A setup carrying on from an earlier one says which, as its stock.
    #[test]
    fn a_carried_stock_says_where_from() {
        let run = run();
        let mut setup = setup();
        setup.stock = StockKind::Earlier;
        let html = Sheet {
            name: "Bottom",
            setup: &setup,
            bodies: vec!["Bracket".into()],
            run: &run,
            carried_from: Some("Top".into()),
        }
        .html();
        assert!(
            html.contains("what setup Top leaves after its last operation"),
            "{html}"
        );
    }
}
