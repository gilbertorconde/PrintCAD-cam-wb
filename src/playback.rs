//! Playing a setup's run back while its panel is open: the material as it
//! stood, the tool where it was, and the toolpath as far as it had gone, at
//! moments the helper sent along the run.
//!
//! A bench is drawn again only after something reached it, so playing keeps a
//! short job going, [`TICK`], whose end moves it on and starts the next.

use std::time::Instant;

use cam_protocol::{Moment, Playback, Run};
use printcad_bench_sdk::api::*;

use crate::panel::{button, group};

/// How many moments a second it plays.
const MOMENTS_PER_SECOND: f64 = 8.0;

/// The job that waits a moment while it plays: its input is how long, in
/// milliseconds.
pub const TICK: &str = "tick";

/// How long a [`TICK`] waits.
pub const TICK_MS: u64 = 125;

/// What a [`TICK`] job does: wait.
pub fn wait(input: &str) -> String {
    let ms = input.trim().parse().unwrap_or(TICK_MS).min(1000);
    std::thread::sleep(std::time::Duration::from_millis(ms));
    String::new()
}

/// A setup being played back.
#[derive(Debug, Clone, PartialEq)]
pub struct Playing {
    /// The setup, by id.
    pub setup: String,
    /// Which moment, by its place.
    pub at: usize,
    /// While it plays: when it was last moved on, and how far between moments
    /// it had got.
    pub running: Option<(Instant, f64)>,
}

impl Playing {
    /// Move on with the time passed. Stops at the last moment.
    pub fn tick(&mut self, play: &Playback) {
        let Some((then, part)) = self.running else {
            return;
        };
        let now = Instant::now();
        let part = part + now.duration_since(then).as_secs_f64() * MOMENTS_PER_SECOND;
        let whole = part.floor();
        let last = play.moments.len().saturating_sub(1);
        self.at = (self.at + whole as usize).min(last);
        self.running = (self.at < last).then_some((now, part - whole));
    }

    pub fn moment<'a>(&self, play: &'a Playback) -> Option<&'a Moment> {
        play.moments.get(self.at)
    }
}

/// The Playback group of the setup panel, for a run that has one.
pub fn playback_group(run: &Run, playing: Option<&Playing>) -> Option<Widget> {
    let play = run.playback.as_ref()?;
    let last = play.moments.len().checked_sub(1)?;
    let at = playing.map_or(last, |p| p.at.min(last));
    let running = playing.is_some_and(|p| p.running.is_some());
    let mut rows = vec![Widget::Slider {
        id: "playback".into(),
        label: "Moment".into(),
        value: at as f64,
        min: 0.0,
        max: last as f64,
        step: Some(1.0),
        decimals: 0,
        show_value: false,
    }];
    if let Some(m) = play.moments.get(at) {
        let name = run.operations.get(m.op).map_or("", |o| o.name.as_str());
        rows.push(Widget::Value {
            label: "At".into(),
            value: format!("{name}, move {} of {}", m.moves, m.of),
            mono: false,
        });
    }
    rows.push(Widget::Row {
        children: vec![
            button(
                "play",
                if running { "Pause" } else { "Play" },
                ButtonStyle::Secondary,
                true,
            ),
            button(
                "play_done",
                "Show it finished",
                ButtonStyle::Small,
                playing.is_some(),
            ),
        ],
    });
    Some(group("Playback", rows))
}

/// What a playback widget of `setup`'s panel does to `playing`. Whether it was
/// one.
pub fn playback_event(
    playing: &mut Option<Playing>,
    setup: &str,
    run: Option<&Run>,
    event: &PanelEvent,
) -> bool {
    let Some(play) = run.and_then(|r| r.playback.as_ref()) else {
        return false;
    };
    let last = play.moments.len().saturating_sub(1);
    let mine = playing.take().filter(|p| p.setup == setup);
    *playing = match event {
        PanelEvent::Number { id, value } if id == "playback" => Some(Playing {
            setup: setup.into(),
            at: (value.round().max(0.0) as usize).min(last),
            running: None,
        }),
        PanelEvent::Button { id } if id == "play" => {
            let mut p = mine.unwrap_or(Playing {
                setup: setup.into(),
                at: 0,
                running: None,
            });
            p.running = match p.running {
                Some(_) => None,
                None => {
                    // From the start again once it has played through.
                    if p.at >= last {
                        p.at = 0;
                    }
                    Some((Instant::now(), 0.0))
                }
            };
            Some(p)
        }
        PanelEvent::Button { id } if id == "play_done" => None,
        _ => {
            *playing = mine;
            return false;
        }
    };
    true
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
    let tip = m.tool;
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
    use super::*;

    fn run(moments: usize) -> Run {
        Run {
            playback: Some(Playback {
                base: vec![[0.0; 3]],
                axis: [0.0, 0.0, 1.0],
                indices: Vec::new(),
                low: -10.0,
                step: 0.5,
                moments: (0..moments)
                    .map(|i| Moment {
                        moves: i,
                        of: moments,
                        heights: vec![20],
                        ..Moment::default()
                    })
                    .collect(),
            }),
            ..Run::default()
        }
    }

    #[test]
    fn the_slider_and_play_move_it() {
        let run = run(5);
        let mut playing = None;
        let slide = PanelEvent::Number {
            id: "playback".into(),
            value: 9.0,
        };
        assert!(playback_event(&mut playing, "s", Some(&run), &slide));
        assert_eq!(playing.as_ref().unwrap().at, 4, "kept to the last");
        // Played through, Play starts again.
        let play = PanelEvent::Button { id: "play".into() };
        assert!(playback_event(&mut playing, "s", Some(&run), &play));
        let p = playing.as_ref().unwrap();
        assert_eq!(p.at, 0);
        assert!(p.running.is_some());
        assert!(playback_event(&mut playing, "s", Some(&run), &play));
        assert!(playing.as_ref().unwrap().running.is_none(), "paused");
        let done = PanelEvent::Button {
            id: "play_done".into(),
        };
        assert!(playback_event(&mut playing, "s", Some(&run), &done));
        assert_eq!(playing, None);
        // Not the playback's.
        let other = PanelEvent::Button { id: "stop".into() };
        assert!(!playback_event(&mut playing, "s", Some(&run), &other));
    }

    #[test]
    fn the_material_stands_where_the_heights_say() {
        let run = run(2);
        let play = run.playback.as_ref().unwrap();
        let mesh = material_at(play, &play.moments[0]);
        assert_eq!(mesh.positions, vec![[0.0, 0.0, 0.0]]);
    }
}
