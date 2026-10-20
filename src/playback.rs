//! Playing a setup's run back while it or one of its operations is open, as
//! camcraft's app does: a transport under the view (a timeline with a span
//! per operation and its problems marked, buttons to go to the start, the
//! operation before, a step back, play backwards, play, a step on, the next
//! operation and the end, the speed, where playing stops by itself, and the
//! machining clock), and keys for the same. The view draws the material as it
//! stood, the tool where it was, and each toolpath as far as it had gone.
//!
//! The playhead is a time: machining seconds from the setup's start, which
//! the helper works out for every move. The material is what the helper sent
//! for the last moment at or before it.

use std::time::Instant;

use cam_protocol::{Moment, Playback, Run};
use printcad_bench_sdk::api::*;

use crate::panel::{button, choice};

/// How fast it plays, as many machining seconds a second: camcraft's app's.
pub const SPEEDS: [f64; 15] = [
    0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0,
    10000.0,
];

/// The speed playing starts at.
const FIRST_SPEED: f64 = 10.0;

/// The longest step one frame may take, so a stall plays on rather than
/// jumping.
const LONGEST_FRAME: f64 = 1.0 / 15.0;

/// How many steps Shift+Left and Shift+Right take: moves, as camcraft's
/// app steps, or moments while only the run's coarse moments are known.
pub const BIG_STEP_MOVES: usize = 100;
pub const BIG_STEP_MOMENTS: usize = 10;

/// The timeline's drawing size, in its own units.
const WIDTH: f32 = 1000.0;
const HEIGHT: f32 = 44.0;

/// `speed` as a label: `¼×`, `½×`, `10×`.
pub fn speed_label(speed: f64) -> String {
    match speed {
        0.25 => "¼×".into(),
        0.5 => "½×".into(),
        s => format!("{s}×"),
    }
}

/// The next speed up or down from `speed`, kept to [`SPEEDS`].
pub fn faster(speed: f64, up: bool) -> f64 {
    let i = SPEEDS
        .iter()
        .position(|s| *s >= speed)
        .unwrap_or(SPEEDS.len() - 1);
    let j = if up {
        (i + 1).min(SPEEDS.len() - 1)
    } else {
        i.saturating_sub(1)
    };
    SPEEDS.get(j).copied().unwrap_or(speed)
}

/// Seconds as `m:ss`, or `h:mm:ss` past an hour.
pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// How the transport plays, kept while the package runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transport {
    pub speed: f64,
    /// Stop by itself at a failed check or a warning.
    pub stop_at_problems: bool,
    /// Stop by itself at the end of each operation.
    pub stop_at_ops: bool,
}

impl Default for Transport {
    fn default() -> Self {
        Self {
            speed: FIRST_SPEED,
            stop_at_problems: true,
            stop_at_ops: false,
        }
    }
}

/// A setup being played back: where, and whether it plays.
#[derive(Debug, Clone, PartialEq)]
pub struct Playing {
    /// The setup, by id.
    pub setup: String,
    /// The playhead, in machining seconds from the setup's start.
    pub at: f64,
    /// While it plays: when it last moved on, and whether backwards.
    pub running: Option<(Instant, bool)>,
}

/// What a run's timeline holds: when each operation starts, how long the
/// whole is, and where its problems are.
struct Timeline {
    starts: Vec<f64>,
    total: f64,
    marks: Vec<(f64, bool)>,
}

impl Timeline {
    fn of(run: &Run) -> Self {
        let mut starts = Vec::new();
        let mut total = 0.0;
        for op in &run.operations {
            starts.push(total);
            total += op.seconds;
        }
        let marks = run
            .operations
            .iter()
            .flat_map(|o| o.marks.iter().map(|m| (m.seconds, m.fail)))
            .collect();
        Self {
            starts,
            total,
            marks,
        }
    }

    /// The places playing stops by itself.
    fn stops(&self, t: &Transport) -> Vec<f64> {
        let mut out = Vec::new();
        if t.stop_at_problems {
            out.extend(self.marks.iter().map(|(s, _)| *s));
        }
        if t.stop_at_ops {
            out.extend(self.starts.iter().skip(1));
        }
        out.sort_by(f64::total_cmp);
        out
    }

    /// The operation under way at `at`.
    fn op_at(&self, at: f64) -> usize {
        self.starts
            .iter()
            .rposition(|s| *s <= at + 1e-9)
            .unwrap_or(0)
    }
}

/// The place in `moments` showing time `at`: the last at or before it.
fn moment_index(play: &Playback, at: f64) -> usize {
    play.moments
        .iter()
        .rposition(|m| m.seconds <= at + 1e-9)
        .unwrap_or(0)
}

impl Playing {
    /// Played back from the end, where the run finished.
    pub fn at_end(setup: &str, run: &Run) -> Self {
        Self {
            setup: setup.into(),
            at: Timeline::of(run).total,
            running: None,
        }
    }

    pub fn moment<'a>(&self, play: &'a Playback) -> Option<&'a Moment> {
        play.moments.get(moment_index(play, self.at))
    }

    /// Move on with the time passed, at `transport`'s speed, stopping where
    /// it stops by itself and at either end.
    pub fn tick(&mut self, run: &Run, transport: &Transport) {
        let Some((then, back)) = self.running else {
            return;
        };
        let now = Instant::now();
        let real = now.duration_since(then).as_secs_f64().min(LONGEST_FRAME);
        let line = Timeline::of(run);
        let step = real * transport.speed * if back { -1.0 } else { 1.0 };
        let to = (self.at + step).clamp(0.0, line.total);
        let stops = line.stops(transport);
        // The first stop passed, going either way; one the playhead stands
        // on is not passed.
        let stop = if back {
            stops.iter().rev().find(|&&s| s < self.at && s >= to)
        } else {
            stops.iter().find(|&&s| s > self.at && s <= to)
        };
        match stop {
            Some(&s) => {
                self.at = s;
                self.running = None;
            }
            None => {
                self.at = to;
                let ended = if back { to <= 0.0 } else { to >= line.total };
                self.running = (!ended).then_some((now, back));
            }
        }
    }

    /// Play forward, or backwards; from the start again at the end, and from
    /// the end at the start.
    fn play(&mut self, total: f64, back: bool) {
        if !back && self.at >= total {
            self.at = 0.0;
        }
        if back && self.at <= 0.0 {
            self.at = total;
        }
        self.running = Some((Instant::now(), back));
    }

    /// Stop, and stand at `at`.
    fn stand(&mut self, at: f64, total: f64) {
        self.at = at.clamp(0.0, total);
        self.running = None;
    }
}

/// A transport command, from its button, the timeline, or a key.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    Start,
    PreviousOp,
    StepBack(usize),
    PlayBack,
    PlayPause,
    StepOn(usize),
    NextOp,
    End,
    /// Shuttle: play this way, faster if it already does.
    Shuttle(bool),
    Pause,
    Faster(bool),
    Seek(f64),
}

/// Do `command` to the playback of `setup`'s `run`.
pub fn apply(
    playing: &mut Option<Playing>,
    transport: &mut Transport,
    setup: &str,
    run: &Run,
    command: Command,
    steps: &[f64],
) {
    if run.playback.is_none() {
        return;
    }
    let line = Timeline::of(run);
    let p = playing.get_or_insert_with(|| Playing::at_end(setup, run));
    let total = line.total;
    // The step shown: the last at or before the playhead.
    let here = steps.iter().rposition(|s| *s <= p.at + 1e-9).unwrap_or(0);
    let moment_time = |i: usize| steps.get(i).copied().unwrap_or(total);
    match command {
        Command::Start => p.stand(0.0, total),
        Command::End => p.stand(total, total),
        Command::PreviousOp => {
            let op = line.op_at(p.at);
            let start = line.starts.get(op).copied().unwrap_or(0.0);
            // At an operation's start already, the one before's.
            let to = if p.at - start < 1e-6 && op > 0 {
                line.starts.get(op - 1).copied().unwrap_or(0.0)
            } else {
                start
            };
            p.stand(to, total);
        }
        Command::NextOp => {
            let op = line.op_at(p.at);
            let to = line.starts.get(op + 1).copied().unwrap_or(total);
            p.stand(to, total);
        }
        Command::StepBack(n) => {
            let to = if p.at > moment_time(here) + 1e-9 {
                // Between moments: back to the one shown.
                moment_time(here)
            } else {
                moment_time(here.saturating_sub(n))
            };
            p.stand(to, total);
        }
        Command::StepOn(n) => {
            let last = steps.len().saturating_sub(1);
            p.stand(moment_time((here + n).min(last)), total);
        }
        Command::PlayBack => match p.running {
            Some((_, true)) => p.running = None,
            _ => p.play(total, true),
        },
        Command::PlayPause => match p.running {
            Some(_) => p.running = None,
            None => p.play(total, false),
        },
        Command::Shuttle(back) => match p.running {
            Some((_, b)) if b == back => transport.speed = faster(transport.speed, true),
            _ => p.play(total, back),
        },
        Command::Pause => p.running = None,
        Command::Faster(up) => transport.speed = faster(transport.speed, up),
        Command::Seek(at) => p.stand(at, total),
    }
}

/// The transport under the view for `run`'s playback.
pub fn transport_widgets(
    run: &Run,
    playing: Option<&Playing>,
    t: &Transport,
    exact: Option<(usize, usize, usize)>,
) -> Vec<Widget> {
    let Some(play) = run.playback.as_ref() else {
        return Vec::new();
    };
    let line = Timeline::of(run);
    let at = playing.map_or(line.total, |p| p.at);
    let running = playing.and_then(|p| p.running);
    let mut shapes = Vec::new();
    let x = |s: f64| {
        if line.total > 0.0 {
            (s / line.total) as f32 * WIDTH
        } else {
            0.0
        }
    };
    for (i, op) in run.operations.iter().enumerate() {
        let (a, b) = (x(line.starts[i]), x(line.starts[i] + op.seconds));
        shapes.push(DiagramShape::Path {
            points: vec![[a, 12.0], [b, 12.0], [b, 32.0], [a, 32.0]],
            closed: true,
            stroke: DiagramStroke::Outline,
            fill: false,
        });
        // A name where it fits.
        if b - a > 9.0 * op.name.chars().count() as f32 {
            shapes.push(DiagramShape::Text {
                at: [(a + b) / 2.0, 22.0],
                text: op.name.clone(),
                mono: false,
            });
        }
    }
    for (s, fail) in &line.marks {
        shapes.push(DiagramShape::Circle {
            center: [x(*s), 6.0],
            radius: 3.5,
            stroke: if *fail {
                DiagramStroke::Accent
            } else {
                DiagramStroke::Outline
            },
            fill: *fail,
        });
    }
    let h = x(at);
    shapes.push(DiagramShape::Path {
        points: vec![[h, 2.0], [h, HEIGHT - 2.0]],
        closed: false,
        stroke: DiagramStroke::Accent,
        fill: false,
    });
    shapes.push(DiagramShape::Path {
        points: vec![[h - 5.0, HEIGHT], [h + 5.0, HEIGHT], [h, HEIGHT - 7.0]],
        closed: true,
        stroke: DiagramStroke::Accent,
        fill: true,
    });
    let small = |id: &str, label: &str| button(id, label, ButtonStyle::Secondary, true);
    let mut out = vec![
        small("t_start", "⏮"),
        small("t_previous_op", "⏪"),
        small("t_step_back", "◀|"),
        button(
            "t_play_back",
            if running.is_some_and(|(_, back)| back) {
                "⏸"
            } else {
                "◀"
            },
            ButtonStyle::Secondary,
            true,
        ),
        button(
            "t_play",
            if running.is_some_and(|(_, back)| !back) {
                "⏸"
            } else {
                "▶"
            },
            ButtonStyle::Primary,
            true,
        ),
        small("t_step_on", "|▶"),
        small("t_next_op", "⏩"),
        small("t_end", "⏭"),
        Widget::Value {
            label: String::new(),
            value: format!("{} / {}", clock(at), clock(line.total)),
            mono: true,
        },
        choice(
            "t_speed",
            "",
            SPEEDS.iter().map(|s| speed_label(*s)).collect(),
            SPEEDS.iter().position(|s| *s == t.speed).unwrap_or(0),
        ),
        Widget::Toggle {
            id: "t_stop_problems".into(),
            label: "Stop at problems".into(),
            on: t.stop_at_problems,
        },
        Widget::Toggle {
            id: "t_stop_ops".into(),
            label: "Stop after each operation".into(),
            on: t.stop_at_ops,
        },
    ];
    let under_way = exact.or_else(|| {
        play.moments
            .get(moment_index(play, at))
            .map(|m| (m.op, m.moves, m.of))
    });
    if let Some((op, moves, of)) = under_way {
        let name = run.operations.get(op).map_or("", |o| o.name.as_str());
        out.push(Widget::Value {
            label: String::new(),
            value: format!("{name}, move {moves} of {of}"),
            mono: false,
        });
    }
    // Last, so it takes the strip's width on a line of its own.
    out.push(Widget::Diagram {
        id: "timeline".into(),
        width: WIDTH,
        height: HEIGHT,
        shapes,
        dimensions: Vec::new(),
        callouts: Vec::new(),
    });
    out
}

/// The places the transport steps between: every move's end where the
/// moves are known (`times`, from a playback session), else the run's
/// moments.
pub fn steps(run: &Run, times: Option<&[Vec<f32>]>) -> Vec<f64> {
    let mut out: Vec<f64> = match times {
        Some(times) => std::iter::once(0.0)
            .chain(times.iter().flatten().map(|t| f64::from(*t)))
            .collect(),
        None => run
            .playback
            .iter()
            .flat_map(|p| p.moments.iter().map(|m| m.seconds))
            .collect(),
    };
    out.sort_by(f64::total_cmp);
    out.dedup();
    out
}

/// What a transport widget did. Whether it was one.
pub fn transport_event(
    playing: &mut Option<Playing>,
    transport: &mut Transport,
    setup: &str,
    run: &Run,
    event: &PanelEvent,
    steps: &[f64],
) -> bool {
    let command = match event {
        PanelEvent::Button { id } => match id.as_str() {
            "t_start" => Command::Start,
            "t_previous_op" => Command::PreviousOp,
            "t_step_back" => Command::StepBack(1),
            "t_play_back" => Command::PlayBack,
            "t_play" => Command::PlayPause,
            "t_step_on" => Command::StepOn(1),
            "t_next_op" => Command::NextOp,
            "t_end" => Command::End,
            _ => return false,
        },
        PanelEvent::DiagramPress { id, x, .. } if id == "timeline" => {
            Command::Seek(f64::from(x.clamp(0.0, WIDTH) / WIDTH) * Timeline::of(run).total)
        }
        PanelEvent::Choice { id, index } if id == "t_speed" => {
            transport.speed = SPEEDS.get(*index).copied().unwrap_or(transport.speed);
            return true;
        }
        PanelEvent::Toggle { id, on } if id == "t_stop_problems" => {
            transport.stop_at_problems = *on;
            return true;
        }
        PanelEvent::Toggle { id, on } if id == "t_stop_ops" => {
            transport.stop_at_ops = *on;
            return true;
        }
        _ => return false,
    };
    apply(playing, transport, setup, run, command, steps);
    true
}

/// The transport's command for key `key` going down, camcraft's keys: Space
/// plays and pauses; J, K and L shuttle back, pause and forward; Left and
/// Right step (with Shift held, ten moments); Up and Down go to the
/// operation before and after; Home and End; Minus and Equals change the
/// speed.
pub fn key_command(key: &str, shift: bool, big: usize) -> Option<Command> {
    let step = if shift { big } else { 1 };
    Some(match key {
        "Space" => Command::PlayPause,
        "J" => Command::Shuttle(true),
        "K" => Command::Pause,
        "L" => Command::Shuttle(false),
        "ArrowLeft" => Command::StepBack(step),
        "ArrowRight" => Command::StepOn(step),
        "ArrowUp" => Command::PreviousOp,
        "ArrowDown" => Command::NextOp,
        "Home" => Command::Start,
        "End" => Command::End,
        "Minus" => Command::Faster(false),
        "Equals" => Command::Faster(true),
        _ => return None,
    })
}

/// The material at `m`, as a mesh over the grid.
pub fn material_at(play: &Playback, m: &Moment) -> Mesh {
    let positions = play
        .base
        .iter()
        .zip(&m.heights)
        .map(|(b, h)| {
            let z = play.low + play.step * f32::from(*h);
            [
                b[0] + play.axis[0] * z,
                b[1] + play.axis[1] * z,
                b[2] + play.axis[2] * z,
            ]
        })
        .collect();
    Mesh {
        positions,
        indices: play.indices.clone(),
        color: [0.78, 0.66, 0.5],
        opacity: 0.85,
        ..Default::default()
    }
}

/// The tool at `m`: its tip, and its shank up the tool's axis.
pub fn tool_at(frame: &mut Frame, play: &Playback, m: &Moment, color: Rgb) {
    tool_tip(frame, play, m.tool, color);
}

/// The tool with its tip at `tip`, and its shank up the tool's axis.
pub fn tool_tip(frame: &mut Frame, play: &Playback, tip: [f32; 3], color: Rgb) {
    let top = [
        tip[0] + play.axis[0] * 25.0,
        tip[1] + play.axis[1] * 25.0,
        tip[2] + play.axis[2] * 25.0,
    ];
    frame.lines.push(Polyline {
        points: vec![tip, top],
        color,
        width: 4.0,
        ..Default::default()
    });
    frame.marks.push(Mark {
        at: tip,
        color,
        kind: MarkKind::Dot { radius: 5.0 },
    });
}

#[cfg(test)]
mod tests {
    use cam_protocol::{Mark as Problem, OperationRun};

    use super::*;

    /// Two operations of 10 s and 30 s, a failure at 25 s, and a moment
    /// every 5 s.
    fn run() -> Run {
        let op = |name: &str, seconds: f64, marks: Vec<Problem>| OperationRun {
            name: name.into(),
            seconds,
            marks,
            ..OperationRun::default()
        };
        Run {
            operations: vec![
                op("Face", 10.0, Vec::new()),
                op(
                    "Profile",
                    30.0,
                    vec![Problem {
                        moves: 7,
                        seconds: 25.0,
                        fail: true,
                    }],
                ),
            ],
            playback: Some(Playback {
                base: vec![[0.0; 3]],
                axis: [0.0, 0.0, 1.0],
                indices: Vec::new(),
                low: -10.0,
                step: 0.5,
                moments: (0..=8)
                    .map(|i| Moment {
                        op: usize::from(i > 2),
                        seconds: f64::from(i) * 5.0,
                        heights: vec![20],
                        ..Moment::default()
                    })
                    .collect(),
            }),
            ..Run::default()
        }
    }

    #[test]
    fn the_buttons_go_where_camcrafts_do() {
        let run = run();
        let (mut p, mut t) = (None, Transport::default());
        let mut go = |c| {
            let steps = steps(&run, None);
            apply(&mut p, &mut t, "s", &run, c, &steps);
            p.as_ref().unwrap().at
        };
        assert_eq!(go(Command::End), 40.0);
        assert_eq!(go(Command::PreviousOp), 10.0, "the operation's start");
        assert_eq!(go(Command::PreviousOp), 0.0, "then the one before's");
        assert_eq!(go(Command::NextOp), 10.0);
        assert_eq!(go(Command::StepOn(1)), 15.0);
        assert_eq!(go(Command::StepOn(100)), 40.0, "held to the last moment");
        assert_eq!(go(Command::Seek(12.0)), 12.0);
        assert_eq!(go(Command::StepBack(1)), 10.0, "to the moment shown first");
        assert_eq!(go(Command::StepBack(1)), 5.0);
        assert_eq!(go(Command::Start), 0.0);
        assert_eq!(go(Command::Faster(true)), 0.0);
        assert_eq!(t.speed, 20.0);
    }

    #[test]
    fn playing_stops_at_a_problem_and_at_the_end() {
        let run = run();
        let t = Transport {
            speed: 10_000.0,
            ..Transport::default()
        };
        let mut p = Playing {
            setup: "s".into(),
            at: 20.0,
            running: Some((Instant::now() - std::time::Duration::from_millis(50), false)),
        };
        p.tick(&run, &t);
        assert_eq!((p.at, p.running), (25.0, None), "at the failure");
        p.running = Some((Instant::now() - std::time::Duration::from_millis(50), false));
        p.tick(&run, &t);
        assert_eq!((p.at, p.running), (40.0, None), "on past it, to the end");
        // Backwards, from the end, it stops at the failure again.
        p.running = Some((Instant::now() - std::time::Duration::from_millis(50), true));
        p.tick(&run, &t);
        assert_eq!(p.at, 25.0);
    }

    #[test]
    fn camcrafts_keys_and_speeds() {
        assert_eq!(key_command("Space", false, 100), Some(Command::PlayPause));
        assert_eq!(
            key_command("ArrowLeft", true, 100),
            Some(Command::StepBack(100))
        );
        assert_eq!(key_command("Q", false, 100), None);
        assert_eq!(faster(10_000.0, true), 10_000.0);
        assert_eq!(faster(0.25, false), 0.25);
        assert_eq!(speed_label(0.25), "¼×");
        assert_eq!(clock(3725.0), "1:02:05");
        assert_eq!(clock(65.0), "1:05");
    }

    #[test]
    fn the_material_stands_where_the_heights_say() {
        let run = run();
        let play = run.playback.as_ref().unwrap();
        let mesh = material_at(play, &play.moments[0]);
        assert_eq!(mesh.positions, vec![[0.0, 0.0, 0.0]]);
    }
}
