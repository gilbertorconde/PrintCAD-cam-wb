//! What the view draws: each setup's stock and work zero, its operations'
//! toolpaths, and the material it leaves.

use cam_protocol::Run;
use printcad_bench_sdk::api::*;

use crate::runs::State;
use crate::{Cam, Kind, operations_of, setups};

const ACCENT: Rgb = [0.31, 0.64, 0.9];
const MUTED: Rgb = [0.55, 0.58, 0.62];
const STOCK: Rgb = [0.9, 0.62, 0.25];
const RAPID: Rgb = [0.9, 0.3, 0.25];
const ZERO: Rgb = [0.35, 0.8, 0.4];
const MATERIAL: Rgb = [0.78, 0.66, 0.5];

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
        if edited_setup
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
            let color = if this || edited_setup { ACCENT } else { MUTED };
            for stroke in &op.strokes {
                if stroke.rapid && !(this || edited_setup) {
                    continue;
                }
                frame.lines.push(Polyline {
                    points: stroke.points.clone(),
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
    }
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
