//! An operation's program beside its playback: the lines round the one the
//! playhead stands on, numbered, that line marked, and a click on a line's
//! number taking the playhead to its move.
//!
//! A program can run to a hundred thousand lines, and the panel is built
//! again every frame while it plays, so only a window of lines round the
//! playhead is shown, found by where each line starts, worked out once for
//! the program; clicking one near its edge moves the window on.

use std::ops::Range;

use cam_protocol::{Program, Run};
use printcad_bench_sdk::api::*;

use crate::panel::group;

/// How many lines are shown before the playhead's, and after it.
const BEFORE: usize = 8;
const AFTER: usize = 16;

/// The listing's id. It stays the same as the window moves, so the panel
/// keeps its sheet (a new one is measured unseen for a frame).
const ID: &str = "listing";

/// Where each line of a program starts, kept for the program last listed.
#[derive(Debug, Default)]
pub struct Lines {
    /// Which text: where it is and how long, while it lives.
    text: (usize, usize),
    starts: Vec<usize>,
    /// The operation and the first line last shown, for a click to find
    /// the line it was on.
    shown: Option<(usize, usize)>,
}

impl Lines {
    /// Where each of `text`'s lines starts, and its end last.
    fn of(&mut self, text: &str) -> &[usize] {
        let key = (text.as_ptr() as usize, text.len());
        if self.text != key || self.starts.is_empty() {
            self.text = key;
            self.starts = std::iter::once(0)
                .chain(text.match_indices('\n').map(|(i, _)| i + 1))
                .filter(|i| *i < text.len())
                .chain(std::iter::once(text.len()))
                .collect();
        }
        &self.starts
    }

    /// `text`'s lines in `range`, without their line ends.
    fn get<'a>(&mut self, text: &'a str, range: Range<usize>) -> Vec<&'a str> {
        let starts = self.of(text);
        range
            .filter_map(|i| {
                let line = text.get(*starts.get(i)?..*starts.get(i + 1)?)?;
                Some(line.trim_end_matches(['\n', '\r']))
            })
            .collect()
    }

    /// How many lines `text` has.
    fn count(&mut self, text: &str) -> usize {
        self.of(text).len() - 1
    }
}

/// The lines to show of `lines`, round line `at`: as many before and after
/// as asked, fewer only at an end, where the window keeps its size.
pub fn window(lines: usize, at: usize, before: usize, after: usize) -> Range<usize> {
    let size = (before + 1 + after).min(lines);
    let start = at.saturating_sub(before).min(lines - size);
    start..start + size
}

/// Where the playhead stands in operation `op`'s program: the line it is on,
/// when it is in that operation; else the program's first line when it has
/// not started yet, or its last when it is done, unmarked.
fn standing(
    program: &Program,
    lines: usize,
    op: usize,
    under_way: Option<(usize, usize)>,
) -> (usize, bool) {
    let last = lines.saturating_sub(1);
    match under_way {
        Some((o, made)) if o == op => match program.line_of(made) {
            Some(line) => (line.min(last), true),
            None => (0, false),
        },
        Some((o, _)) if o > op => (last, false),
        Some(_) => (0, false),
        // Not played back: at its end, as the run left it.
        None => (last, false),
    }
}

/// The listing of operation `op` of `run`, the playhead being in operation
/// `under_way.0` with `under_way.1` of its moves made; `kept` keeps where
/// its lines start.
pub fn widgets(
    run: &Run,
    op: usize,
    under_way: Option<(usize, usize)>,
    kept: &mut Lines,
) -> Vec<Widget> {
    let Some(o) = run.operations.get(op) else {
        return Vec::new();
    };
    let Some(program) = &o.program else {
        return Vec::new();
    };
    let lines = kept.count(&program.text);
    if lines == 0 {
        return Vec::new();
    }
    let (at, marked) = standing(program, lines, op, under_way);
    let shown = window(lines, at, BEFORE, AFTER);
    let rows: Vec<Vec<Cell>> = kept
        .get(&program.text, shown.clone())
        .into_iter()
        .enumerate()
        .map(|(i, text)| {
            let line = shown.start + i;
            // The playhead's line pointed at, and its words set apart.
            let here = marked && line == at;
            vec![
                Cell::Link {
                    text: if here {
                        format!("▶ {}", line + 1)
                    } else {
                        (line + 1).to_string()
                    },
                },
                Cell::Text {
                    text: text.to_string(),
                    mono: !here,
                    strong: here,
                },
            ]
        })
        .collect();
    kept.shown = Some((op, shown.start));
    let mut children = vec![Widget::Value {
        label: o.name.clone(),
        value: if marked {
            format!("line {} of {lines}", at + 1)
        } else {
            format!("{lines} lines")
        },
        mono: false,
    }];
    if program.moves.is_empty() {
        children.push(Widget::Text {
            text: "Generate again to follow the playhead here.".into(),
            mono: false,
        });
    }
    // A sheet, not a table: it scrolls sideways where a long line would
    // widen the panel over the view.
    children.push(Widget::Sheet {
        id: ID.into(),
        columns: vec!["Line".into(), "G-code".into()],
        rows,
    });
    vec![group("Program", children)]
}

/// The operation and the moves made at the line a click on the listing
/// chose, if `event` is one, `kept` having shown it.
pub fn clicked(run: &Run, event: &PanelEvent, kept: &Lines) -> Option<(usize, usize)> {
    let PanelEvent::Select { id, index } = event else {
        return None;
    };
    if id != ID {
        return None;
    }
    let (op, start) = kept.shown?;
    let program = run.operations.get(op)?.program.as_ref()?;
    Some((op, program.made_by(start + index)?))
}

#[cfg(test)]
mod tests {
    use cam_protocol::OperationRun;

    use super::*;

    #[test]
    fn the_window_keeps_its_size_and_stays_inside() {
        assert_eq!(window(100, 50, 8, 16), 42..67);
        assert_eq!(window(100, 3, 8, 16), 0..25, "at the start");
        assert_eq!(window(100, 99, 8, 16), 75..100, "at the end");
        assert_eq!(window(10, 5, 8, 16), 0..10, "a short program, whole");
        assert_eq!(window(100_000, 99_990, 8, 16), 99_975..100_000);
    }

    /// One operation of a header line, three moves (the second in two
    /// lines) and a footer line.
    fn run() -> Run {
        let text = "G21\nG0 Z5\nG1 X1\nG2 X2\nG1 X3\nG1 X4\nM30\n";
        Run {
            operations: vec![OperationRun {
                name: "Pocket".into(),
                program: Some(Program {
                    lines: 7,
                    moves: vec![0, 1, 2, 3, 3, 4, 4],
                    text: text.into(),
                    ..Program::default()
                }),
                ..OperationRun::default()
            }],
            ..Run::default()
        }
    }

    /// The sheet's rows as text, and which is marked.
    fn sheet(widgets: &[Widget]) -> (Vec<Vec<String>>, Option<usize>) {
        let Some(Widget::Group { children, .. }) = widgets.first() else {
            panic!("no group");
        };
        let rows = children
            .iter()
            .find_map(|w| match w {
                Widget::Sheet { id, rows, .. } if id == ID => Some(rows.clone()),
                _ => None,
            })
            .unwrap();
        let text = |c: &Cell| match c {
            Cell::Link { text } | Cell::Text { text, .. } => text.clone(),
            _ => String::new(),
        };
        let marked = rows
            .iter()
            .position(|r| matches!(&r[1], Cell::Text { strong: true, .. }));
        (
            rows.iter().map(|r| r.iter().map(text).collect()).collect(),
            marked,
        )
    }

    #[test]
    fn lines_are_found_by_where_they_start() {
        let mut kept = Lines::default();
        let text = "G21\r\nG0 Z5\n\nM30";
        assert_eq!(kept.count(text), 4);
        assert_eq!(kept.get(text, 0..4), vec!["G21", "G0 Z5", "", "M30"]);
        assert_eq!(kept.get(text, 3..9), vec!["M30"], "no further than the end");
        let ended = String::from("G21\nM30\n");
        assert_eq!(kept.count(&ended), 2, "the last line end starts none");
        assert_eq!(kept.count(""), 0);
    }

    #[test]
    fn the_playheads_line_is_marked() {
        let run = run();
        let mut kept = Lines::default();
        let (rows, marked) = sheet(&widgets(&run, 0, Some((0, 3)), &mut kept));
        assert_eq!(rows[0], vec!["1".to_string(), "G21".to_string()]);
        assert_eq!(marked, Some(3), "the first of move 3's lines");
        assert_eq!(rows[3][0], "▶ 4");
        // In another operation, nothing is marked.
        let (_, marked) = sheet(&widgets(&run, 0, Some((1, 3)), &mut kept));
        assert_eq!(marked, None);
    }

    #[test]
    fn a_click_finds_its_lines_move() {
        let run = run();
        let mut kept = Lines::default();
        let click = |kept: &Lines, id: &str, index| {
            let event = PanelEvent::Select {
                id: id.into(),
                index,
            };
            clicked(&run, &event, kept)
        };
        assert_eq!(click(&kept, ID, 0), None, "nothing shown yet");
        widgets(&run, 0, Some((0, 0)), &mut kept);
        assert_eq!(click(&kept, ID, 4), Some((0, 3)));
        assert_eq!(click(&kept, ID, 9), None, "past the end");
        assert_eq!(click(&kept, "operations", 0), None);
        // From the window's start, where it was shown.
        kept.shown = Some((0, 2));
        assert_eq!(click(&kept, ID, 3), Some((0, 4)));
        kept.shown = Some((1, 0));
        assert_eq!(click(&kept, ID, 0), None, "no such operation");
    }
}
