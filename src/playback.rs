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
//!
//! A setup of several parts plays as its program cuts them: the timeline is
//! its sections, each operation at each part in the program's order, the
//! tool going from part to part between, and each part's material is cut as
//! far as the program has cut that part.

use std::time::Instant;

use cam_protocol::{Moment, Playback, Run, Section};
use printcad_bench_sdk::api::*;

use printcad_bench_sdk::panel as pb;

use crate::panel::{button, toggle};

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

/// What a run's timeline holds: its sections (each operation at each
/// part), how long the whole is, and where its problems are.
struct Timeline {
    sections: Vec<Section>,
    total: f64,
    marks: Vec<(f64, bool)>,
}

impl Timeline {
    fn of(run: &Run) -> Self {
        let sections = run.cut();
        let total = sections
            .iter()
            .map(|s| s.start + s.seconds)
            .fold(0.0, f64::max);
        // An operation's problems at each part it is cut at.
        let mut marks: Vec<(f64, bool)> = sections
            .iter()
            .flat_map(|s| {
                run.operations
                    .get(s.op)
                    .into_iter()
                    .flat_map(|o| o.marks.iter())
                    .map(|m| (s.start + m.seconds, m.fail))
            })
            .collect();
        marks.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self {
            sections,
            total,
            marks,
        }
    }

    /// When section `s` starts; the end past the last.
    fn start(&self, s: usize) -> f64 {
        self.sections.get(s).map_or(self.total, |s| s.start)
    }

    /// The places playing stops by itself.
    fn stops(&self, t: &Transport) -> Vec<f64> {
        let mut out = Vec::new();
        if t.stop_at_problems {
            out.extend(self.marks.iter().map(|(s, _)| *s));
        }
        if t.stop_at_ops {
            out.extend(self.sections.iter().skip(1).map(|s| s.start));
        }
        out.sort_by(f64::total_cmp);
        out
    }

    /// The section under way at `at`: the last started by then.
    fn section_at(&self, at: f64) -> usize {
        self.sections
            .iter()
            .rposition(|s| s.start <= at + 1e-9)
            .unwrap_or(0)
    }
}

/// The operation section `section` of `run` cuts, and at which part.
pub fn section_of(run: &Run, section: usize) -> Option<Section> {
    run.cut().get(section).copied()
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

    /// Each of `parts` parts' material as it stands: the last moment of
    /// that part at or before the playhead, else the start (the first
    /// moment), before the part is cut.
    pub fn materials<'a>(&self, play: &'a Playback, parts: usize) -> Vec<&'a Moment> {
        let here = moment_index(play, self.at);
        let Some(start) = play.moments.first() else {
            return Vec::new();
        };
        (0..parts.max(1))
            .map(|p| {
                play.moments[..=here.min(play.moments.len() - 1)]
                    .iter()
                    .rfind(|m| m.part == p)
                    .unwrap_or(start)
            })
            .collect()
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
            let s = line.section_at(p.at);
            let start = line.start(s).min(total);
            // At a section's start already, the one before's.
            let to = if p.at - start < 1e-6 && s > 0 {
                line.start(s - 1)
            } else {
                start
            };
            p.stand(to, total);
        }
        Command::NextOp => {
            let s = line.section_at(p.at);
            p.stand(line.start(s + 1), total);
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
    if run.playback.is_none() {
        return Vec::new();
    }
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
    let several = run.placements.len() > 1;
    for (i, s) in line.sections.iter().enumerate() {
        let Some(op) = run.operations.get(s.op) else {
            continue;
        };
        let (a, b) = (x(s.start), x(s.start + s.seconds));
        shapes.push(DiagramShape::Path {
            points: vec![[a, 12.0], [b, 12.0], [b, 32.0], [a, 32.0]],
            closed: true,
            stroke: DiagramStroke::Outline,
            fill: false,
        });
        // Where the program goes on to another part, a line across.
        let wcs = run.placements.get(s.part).map(|p| p.wcs.as_str());
        if several && i > 0 && line.sections[i - 1].part != s.part {
            shapes.push(DiagramShape::Path {
                points: vec![[a, 8.0], [a, 36.0]],
                closed: false,
                stroke: DiagramStroke::Outline,
                fill: false,
            });
        }
        // A name where it fits, and the part's offset: else the offset
        // alone.
        let fits = |t: &str| b - a > 9.0 * t.chars().count() as f32;
        let named = match wcs.filter(|_| several) {
            Some(w) => format!("{} {w}", op.name),
            None => op.name.clone(),
        };
        let label = if fits(&named) {
            Some(named)
        } else {
            wcs.filter(|w| several && fits(w)).map(str::to_string)
        };
        if let Some(text) = label {
            shapes.push(DiagramShape::Text {
                at: [(a + b) / 2.0, 22.0],
                text,
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
        Caption {
            text: format!("{} / {}", clock(at), clock(line.total)),
            mono: true,
            ..Default::default()
        }
        .into(),
        Select {
            id: "t_speed".into(),
            options: SPEEDS.iter().map(|s| speed_label(*s)).collect(),
            selected: SPEEDS.iter().position(|s| *s == t.speed).unwrap_or(0),
            width: Some(72.0),
            ..Default::default()
        }
        .into(),
        toggle("t_stop_problems", "Stop at problems", t.stop_at_problems),
        toggle("t_stop_ops", "Stop after each operation", t.stop_at_ops),
    ];
    if let Some((s, moves, of)) = under_way(run, playing, exact) {
        let section = line.sections.get(s).copied().unwrap_or_default();
        let name = run
            .operations
            .get(section.op)
            .map_or("", |o| o.name.as_str());
        let value = match run.placements.get(section.part).filter(|_| several) {
            Some(p) => format!("{name} at {}, move {moves} of {of}", p.wcs),
            None => format!("{name}, move {moves} of {of}"),
        };
        out.push(pb::caption(&value, Tone::Neutral, false));
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

/// The section under way at the playhead (an operation at a part), how
/// many of its moves are made, and of how many: `exact`, as a playback
/// session last answered, or else worked out between the run's moments
/// either side of the playhead (at its end while it is not played back).
pub fn under_way(
    run: &Run,
    playing: Option<&Playing>,
    exact: Option<(usize, usize, usize)>,
) -> Option<(usize, usize, usize)> {
    exact.or_else(|| {
        run.playback.as_ref()?;
        let line = Timeline::of(run);
        let at = playing.map_or(line.total, |p| p.at);
        let s = line.section_at(at);
        let known = moments_of(run, s, &line);
        let of = known.last().map_or(0, |(n, _)| *n);
        let before = known.iter().rfind(|(_, t)| *t <= at + 1e-9)?;
        let made = match known.iter().find(|(_, t)| *t > at + 1e-9) {
            Some(after) if after.1 > before.1 => {
                let f = (at - before.1) / (after.1 - before.1);
                before.0 + (f * (after.0 - before.0) as f64) as usize
            }
            _ => before.0,
        };
        Some((s, made, of))
    })
}

/// Section `section`'s moments as moves made and when: its start, with
/// none made, then each of the run's.
fn moments_of(run: &Run, section: usize, line: &Timeline) -> Vec<(usize, f64)> {
    std::iter::once((0, line.start(section)))
        .chain(
            run.playback
                .iter()
                .flat_map(|p| p.moments.iter())
                .filter(|m| m.section == section)
                .map(|m| (m.moves, m.seconds)),
        )
        .collect()
}

/// When section `section` of `run` (an operation at a part) has made
/// `made` of its moves, in machining seconds from the setup's start: from
/// when each move ends (`times`, from a playback session) where they are
/// known, else between the run's moments either side. Where the ends are
/// known, a hair past it, so a seek there stands with that many made
/// whatever the rounding, but never as far as the next move's end.
pub fn time_of(run: &Run, times: Option<&[Vec<f32>]>, section: usize, made: usize) -> f64 {
    let line = Timeline::of(run);
    let start = line.start(section);
    if let Some(ends) = times.and_then(|t| t.get(section)) {
        let end = |i: usize| ends.get(i).map(|t| f64::from(*t));
        let at = made.checked_sub(1).and_then(end).unwrap_or(start);
        // At the last, no further: past it is the next operation's start.
        return match end(made) {
            Some(next) => at + (at.abs() * 1e-6 + 1e-6).min((next - at) / 2.0).max(0.0),
            None => at,
        };
    }
    let known = moments_of(run, section, &line);
    let before = known.iter().rfind(|(n, _)| *n <= made).copied();
    let after = known.iter().find(|(n, _)| *n >= made).copied();
    match (before, after) {
        (Some((a, s)), Some((b, t))) if b > a => s + (made - a) as f64 / (b - a) as f64 * (t - s),
        (Some((_, s)), _) => s,
        _ => start,
    }
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

/// The material at `m`, as a mesh over the grid, moved by `offset` to the
/// part it is of.
pub fn material_at(play: &Playback, m: &Moment, offset: [f32; 3]) -> Mesh {
    let positions = play
        .base
        .iter()
        .zip(&m.heights)
        .enumerate()
        .map(|(i, (b, h))| {
            let z = play.low + play.step * f32::from(*h);
            // On a rotary each point stands out from the axis its own way.
            let up = play.axes.get(i).unwrap_or(&play.axis);
            std::array::from_fn(|k| b[k] + up[k] * z + offset[k])
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

/// The way the tool's shank goes up from its tip at `tip`: the tool's axis,
/// or on a rotary straight out from the rotary's axis.
fn shank(play: &Playback, tip: [f32; 3]) -> [f32; 3] {
    let Some([at, along]) = play.about else {
        return play.axis;
    };
    let off: [f32; 3] = std::array::from_fn(|i| tip[i] - at[i]);
    let len2: f32 = along.iter().map(|v| v * v).sum();
    let t = off.iter().zip(&along).map(|(o, a)| o * a).sum::<f32>() / len2.max(1e-12);
    let out: [f32; 3] = std::array::from_fn(|i| off[i] - along[i] * t);
    let len = out.iter().map(|v| v * v).sum::<f32>().sqrt();
    if len < 1e-6 {
        play.axis
    } else {
        out.map(|v| v / len)
    }
}

/// The tool at `m`: its tip, and its shank up the tool's axis.
pub fn tool_at(frame: &mut Frame, play: &Playback, m: &Moment, color: Rgb) {
    tool_tip(frame, play, m.tool, color);
}

/// The tool with its tip at `tip`, and its shank up the tool's axis.
pub fn tool_tip(frame: &mut Frame, play: &Playback, tip: [f32; 3], color: Rgb) {
    let up = shank(play, tip);
    let top = [
        tip[0] + up[0] * 25.0,
        tip[1] + up[1] * 25.0,
        tip[2] + up[2] * 25.0,
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
                        seconds: 15.0,
                        fail: true,
                    }],
                ),
            ],
            playback: Some(Playback {
                base: vec![[0.0; 3]],
                axis: [0.0, 0.0, 1.0],
                axes: Vec::new(),
                about: None,
                indices: Vec::new(),
                low: -10.0,
                step: 0.5,
                moments: (0..=8)
                    .map(|i| Moment {
                        section: usize::from(i > 2),
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
    fn a_move_has_a_time_to_seek_to() {
        // Exactly, where the session knows when each move ends: a hair past
        // the end of the last made, and at the end of the last.
        let run = run();
        let times = vec![vec![0.0, 4.0, 10.0], vec![15.0, 40.0]];
        let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
        let t = time_of(&run, Some(&times), 0, 2);
        assert!(t > 4.0 && near(t, 4.0), "{t}");
        assert_eq!(time_of(&run, Some(&times), 0, 3), 10.0);
        let t = time_of(&run, Some(&times), 1, 0);
        assert!(t > 10.0 && near(t, 10.0), "{t}");
        // Else between the moments either side, from the operation's start.
        let mut run = run;
        let moments = &mut run.playback.as_mut().unwrap().moments;
        moments.retain(|m| m.op == 0);
        for (moves, seconds) in [(4, 20.0), (8, 40.0)] {
            moments.push(Moment {
                section: 1,
                op: 1,
                moves,
                of: 8,
                seconds,
                ..Moment::default()
            });
        }
        assert_eq!(time_of(&run, None, 1, 2), 15.0);
        assert_eq!(time_of(&run, None, 1, 6), 30.0);
        assert_eq!(time_of(&run, None, 1, 8), 40.0);
        assert_eq!(time_of(&run, None, 1, 0), 10.0);
    }

    #[test]
    fn under_way_is_the_sessions_else_between_the_moments() {
        let mut run = run();
        let playing = |at| Playing {
            setup: "s".into(),
            at,
            running: None,
        };
        assert_eq!(
            under_way(&run, Some(&playing(12.0)), Some((1, 5, 9))),
            Some((1, 5, 9))
        );
        // The second operation, from 10 s, makes 4 moves by 20 s and 8 by
        // 40 s.
        let moments = &mut run.playback.as_mut().unwrap().moments;
        moments.retain(|m| m.op == 0);
        for (moves, seconds) in [(4, 20.0), (8, 40.0)] {
            moments.push(Moment {
                section: 1,
                op: 1,
                moves,
                of: 8,
                seconds,
                ..Moment::default()
            });
        }
        assert_eq!(under_way(&run, Some(&playing(15.0)), None), Some((1, 2, 8)));
        assert_eq!(under_way(&run, Some(&playing(30.0)), None), Some((1, 6, 8)));
        assert_eq!(under_way(&run, None, None), Some((1, 8, 8)), "at the end");
        let first = under_way(&run, Some(&playing(5.0)), None).unwrap();
        assert_eq!(first.0, 0);
    }

    #[test]
    fn the_material_stands_where_the_heights_say() {
        let run = run();
        let play = run.playback.as_ref().unwrap();
        let mesh = material_at(play, &play.moments[0], [0.0; 3]);
        assert_eq!(mesh.positions, vec![[0.0, 0.0, 0.0]]);
        let moved = material_at(play, &play.moments[0], [100.0, 0.0, 0.0]);
        assert_eq!(moved.positions, vec![[100.0, 0.0, 0.0]], "at its part");
    }

    /// The run of [`run`] at two parts by tool: the face at G54 (0 to 10
    /// s), the tool across to G55 (2 s), the face there (12 to 22 s), back
    /// (2 s), the profile at G54 (24 to 54 s), across, and at G55 (56 to 86
    /// s). A moment at each section's end.
    fn two_parts() -> Run {
        use cam_protocol::{PlacedPart, Section};
        let mut run = run();
        run.placements = ["G54", "G55"]
            .map(|w| PlacedPart {
                wcs: w.into(),
                ..PlacedPart::default()
            })
            .to_vec();
        run.sections = [(0, 0, 0.0), (0, 1, 12.0), (1, 0, 24.0), (1, 1, 56.0)]
            .map(|(op, part, start)| Section {
                op,
                part,
                start,
                seconds: run.operations[op].seconds,
            })
            .to_vec();
        let play = run.playback.as_mut().unwrap();
        play.moments = std::iter::once((0, 0, 0, 0.0))
            .chain([
                (0, 0, 4, 10.0),
                (1, 0, 4, 22.0),
                (2, 1, 8, 54.0),
                (3, 1, 8, 86.0),
            ])
            .map(|(section, op, moves, seconds)| Moment {
                section,
                op,
                part: section % 2,
                moves,
                of: moves,
                seconds,
                heights: vec![u16::try_from(section).unwrap()],
                ..Moment::default()
            })
            .collect();
        run
    }

    /// Several parts play as the program cuts them: section by section,
    /// the operation buttons going from one to the next, each operation's
    /// problems at each part, and each part's material as far as it is cut.
    #[test]
    fn several_parts_play_section_by_section() {
        let run = two_parts();
        let line = Timeline::of(&run);
        assert_eq!(line.total, 86.0);
        assert_eq!(
            line.marks,
            vec![(39.0, true), (71.0, true)],
            "the profile's failure at each part"
        );
        let (mut p, mut t) = (None, Transport::default());
        let mut go = |c| {
            let steps = steps(&run, None);
            apply(&mut p, &mut t, "s", &run, c, &steps);
            p.as_ref().unwrap().at
        };
        assert_eq!(go(Command::Seek(11.0)), 11.0, "on the way to G55");
        assert_eq!(go(Command::NextOp), 12.0, "the face at G55");
        assert_eq!(go(Command::NextOp), 24.0);
        assert_eq!(go(Command::PreviousOp), 12.0);
        let playing = |at| Playing {
            setup: "s".into(),
            at,
            running: None,
        };
        assert_eq!(under_way(&run, Some(&playing(17.0)), None), Some((1, 2, 4)));
        assert_eq!(under_way(&run, Some(&playing(70.0)), None).unwrap().0, 3);
        // Seeking a move finds its own part's time.
        assert_eq!(time_of(&run, None, 3, 8), 86.0);
        assert_eq!(time_of(&run, None, 1, 0), 12.0);
        // Each part's material: the second untouched until its face, then
        // faced while the first is profiled.
        let play = run.playback.as_ref().unwrap();
        let heights = |at| -> Vec<u16> {
            playing(at)
                .materials(play, 2)
                .iter()
                .map(|m| m.heights[0])
                .collect()
        };
        assert_eq!(heights(5.0), [0, 0]);
        assert_eq!(heights(30.0), [0, 1]);
        assert_eq!(heights(60.0), [2, 1]);
        assert_eq!(heights(86.0), [2, 3]);
    }
}
