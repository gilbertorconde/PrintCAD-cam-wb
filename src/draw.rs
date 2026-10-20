//! What the view draws: each setup's stock and work zero, its operations'
//! toolpaths, and the material it leaves.

use cam_protocol::{HeightPlane, Run};
use printcad_bench_sdk::api::*;

use crate::playback::{material_at, tool_at};
use crate::runs::State;
use crate::{Cam, Kind, operations_of, setups};

const ACCENT: Rgb = [0.31, 0.64, 0.9];
const MUTED: Rgb = [0.55, 0.58, 0.62];
const STOCK: Rgb = [0.9, 0.62, 0.25];
const RAPID: Rgb = [0.9, 0.3, 0.25];
const ZERO: Rgb = [0.35, 0.8, 0.4];
const MATERIAL: Rgb = [0.78, 0.66, 0.5];
const CLEARANCE: Rgb = [0.35, 0.8, 0.4];
const RETRACT: Rgb = [0.3, 0.75, 0.75];
const TOP: Rgb = [0.31, 0.64, 0.9];
const BOTTOM: Rgb = [0.75, 0.45, 0.85];

/// The most points drawn of the setups not being edited, together.
const BACKGROUND_POINTS: usize = 60_000;

pub fn draw(cam: &Cam, frame: &mut Frame) {
    let editing = cam.editing.as_ref();
    let mut background = 0;
    for (node, _) in setups() {
        let Some(computed) = cam.runs.get(&node.id) else {
            continue;
        };
        let ops = operations_of(&node.id);
        let edited_setup = editing.is_some_and(|e| e.kind == Kind::Setup && e.id == node.id);
        let edited_op = editing
            .filter(|e| e.kind == Kind::Operation)
            .and_then(|e| ops.iter().position(|(n, _)| n.id == e.id));
        let mine = edited_setup || edited_op.is_some();
        if !node.visible && !mine {
            continue;
        }
        if mine {
            match &computed.state {
                State::Queued => frame.hud.footer.push("Waiting to run".into()),
                State::Running(_) => frame.hud.footer.push("Working out the toolpaths…".into()),
                _ => {}
            }
        }
        let Some(run) = computed.shown() else {
            continue;
        };
        stock_box(frame, run, mine);
        frame.marks.push(Mark {
            at: run.origin.map(|v| v as f32),
            color: ZERO,
            kind: MarkKind::Cross { size: 8.0 },
        });
        // The moment being played back, while the setup is open.
        let moment = cam
            .playing
            .as_ref()
            .filter(|p| mine && p.setup == node.id)
            .and_then(|p| {
                let play = run.playback.as_ref()?;
                Some((play, p.moment(play)?))
            });
        if let Some((play, m)) = moment {
            frame.meshes.push(material_at(play, m));
        } else if edited_setup
            && cam.show_material
            && let Some(m) = &run.material
        {
            frame.meshes.push(Mesh {
                positions: m.positions.clone(),
                indices: m.indices.clone(),
                color: MATERIAL,
                opacity: 0.85,
                ..Default::default()
            });
        }
        for (i, op) in run.operations.iter().enumerate() {
            let this = edited_op == Some(i);
            let shown = ops.get(i).is_none_or(|(n, _)| n.visible) || this;
            if !shown {
                continue;
            }
            let points: usize = op.strokes.iter().map(|s| s.points.len()).sum();
            if !mine {
                if background + points > BACKGROUND_POINTS {
                    continue;
                }
                background += points;
            }
            if this {
                for plane in &op.heights {
                    height_plane(frame, plane);
                }
            }
            let color = if this || edited_setup { ACCENT } else { MUTED };
            // Played back, as far as the tool has gone.
            let mut left = match moment {
                Some((_, m)) if i > m.op => continue,
                Some((_, m)) if i == m.op => m.drawn,
                _ => usize::MAX,
            };
            for stroke in &op.strokes {
                if left < 2 {
                    break;
                }
                let points = &stroke.points[..stroke.points.len().min(left)];
                left -= points.len();
                if stroke.rapid && !(this || edited_setup) {
                    continue;
                }
                frame.lines.push(Polyline {
                    points: points.to_vec(),
                    color: if stroke.rapid { RAPID } else { color },
                    width: if this { 1.5 } else { 1.0 },
                    dashed: stroke.rapid,
                    ..Default::default()
                });
            }
            if this && op.points > points {
                frame.hud.footer.push(format!(
                    "Toolpath drawn thinned: {} of {} points",
                    points, op.points
                ));
            }
        }
        if let Some((play, m)) = moment {
            tool_at(frame, play, m, STOCK);
        }
    }
}

/// One of the edited operation's heights: a see-through plane across the
/// stock, its edge, and its name and height at a corner.
fn height_plane(frame: &mut Frame, plane: &HeightPlane) {
    let color = match plane.name.as_str() {
        "Clearance" => CLEARANCE,
        "Retract" => RETRACT,
        "Top" => TOP,
        _ => BOTTOM,
    };
    frame.meshes.push(Mesh {
        positions: plane.corners.to_vec(),
        indices: vec![0, 1, 2, 0, 2, 3],
        color,
        opacity: 0.12,
        ..Default::default()
    });
    frame.lines.push(Polyline {
        points: plane.corners.to_vec(),
        color,
        closed: true,
        ..Default::default()
    });
    frame.labels.push(Label {
        at: plane.corners[0],
        text: format!("{}  Z {:.2}", plane.name, plane.z),
        color,
        size: 12.0,
        pill: true,
        mono: false,
    });
}

/// The stock's box, as twelve edges.
fn stock_box(frame: &mut Frame, run: &Run, mine: bool) {
    let [lo, hi] = run.stock;
    if lo == hi {
        return;
    }
    let p = |x: bool, y: bool, z: bool| {
        [
            if x { hi[0] } else { lo[0] } as f32,
            if y { hi[1] } else { lo[1] } as f32,
            if z { hi[2] } else { lo[2] } as f32,
        ]
    };
    let ring = |z: bool| {
        vec![
            p(false, false, z),
            p(true, false, z),
            p(true, true, z),
            p(false, true, z),
        ]
    };
    let color = if mine { STOCK } else { MUTED };
    for z in [false, true] {
        frame.lines.push(Polyline {
            points: ring(z),
            color,
            dashed: true,
            closed: true,
            ..Default::default()
        });
    }
    for (x, y) in [(false, false), (true, false), (true, true), (false, true)] {
        frame.lines.push(Polyline {
            points: vec![p(x, y, false), p(x, y, true)],
            color,
            dashed: true,
            ..Default::default()
        });
    }
}
