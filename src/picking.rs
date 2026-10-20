//! Picking in the view, as camcraft's app picks: a slot is armed by its row
//! in the panel, and while it is, hovering shows what a click would take and
//! a click takes it. An operation takes faces, and edges in loops (a click on
//! an edge takes the loop it goes round at its height, shown before the
//! click); a setup takes its work zero, a corner or the centre of a round
//! edge.

use printcad_bench_sdk::api::*;
use printcad_bench_sdk::{host, json, serde_json};

use camcraft_core::job::Snap;

/// What an armed slot takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Faces,
    Edges,
    Zero,
}

impl Slot {
    pub fn named(id: &str) -> Option<Self> {
        Some(match id {
            "faces" => Self::Faces,
            "edges" => Self::Edges,
            "zero_pick" => Self::Zero,
            _ => return None,
        })
    }
}

/// A slot armed for a feature, and what a click would take now.
#[derive(Debug, Clone, PartialEq)]
pub struct Armed {
    /// The setup or operation picking, by id.
    pub feature: String,
    pub slot: Slot,
    pub hover: Option<Hover>,
}

/// What a click would take, and how it is drawn.
#[derive(Debug, Clone, PartialEq)]
pub enum Hover {
    /// A face, by a point on it, drawn by its loops.
    Face {
        body: String,
        at: [f32; 3],
        loops: Vec<Vec<[f32; 3]>>,
    },
    /// An edge, by a point on it, drawn by the loop a click takes.
    Edge {
        body: String,
        at: [f32; 3],
        round: Vec<[f32; 3]>,
    },
    /// A point for the work zero.
    Point {
        body: String,
        snap: Snap,
        at: [f32; 3],
        /// The round edge whose centre it is.
        circle: Vec<[f32; 3]>,
    },
}

/// The loops of the face of `body` at `point`, each a polyline.
fn face_loops(body: &str, at: [f32; 3]) -> Vec<Vec<[f32; 3]>> {
    let answer = host::call(calls::GEOM_FACE_LOOPS, json!({ "body": body, "point": at }));
    let Some(loops) = answer
        .ok()
        .and_then(|v| serde_json::from_value::<FaceLoops>(v).ok())
    else {
        return Vec::new();
    };
    loops
        .loops
        .iter()
        .map(|round| {
            round
                .iter()
                .flat_map(|e| e.points.iter())
                .map(|p| p.map(|v| v as f32))
                .collect()
        })
        .collect()
}

/// The loop a click on `edge` takes, camcraft's way: of the loops of the two
/// faces it runs between that hold it, the one that keeps flattest along
/// `up`, the tool's axis: the edges joined to it end to end at its height.
fn edge_loop(edge: &EdgeRef, up: [f32; 3]) -> Vec<[f32; 3]> {
    let Some(names) = edge.faces else {
        return Vec::new();
    };
    let along = |p: &[f32; 3]| p[0] * up[0] + p[1] * up[1] + p[2] * up[2];
    // An edge that is not level is taken alone.
    let level = along(&edge.direction).abs() < 0.1;
    let mut best: Option<(f32, Vec<[f32; 3]>)> = None;
    for name in names {
        let answer = host::call(
            calls::GEOM_FACE_LOOPS,
            json!({ "body": edge.body, "name": name, "point": edge.point }),
        );
        let Some(loops) = answer
            .ok()
            .and_then(|v| serde_json::from_value::<FaceLoops>(v).ok())
        else {
            continue;
        };
        for round in &loops.loops {
            let Some(own) = round.iter().find(|e| e.faces == edge.faces) else {
                continue;
            };
            if !level {
                return own.points.iter().map(|p| p.map(|v| v as f32)).collect();
            }
            let points: Vec<[f32; 3]> = round
                .iter()
                .flat_map(|e| e.points.iter())
                .map(|p| p.map(|v| v as f32))
                .collect();
            let (lo, hi) = points
                .iter()
                .map(along)
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), h| {
                    (lo.min(h), hi.max(h))
                });
            let spread = hi - lo;
            if best.as_ref().is_none_or(|(s, _)| spread < *s) {
                best = Some((spread, points));
            }
        }
    }
    best.map(|(_, p)| p).unwrap_or_default()
}

/// What a click would take for `slot`, with the pointer at `pointer`, `up`
/// being the setup's tool axis.
pub fn hover(slot: Slot, pointer: &Pointer, up: [f32; 3]) -> Option<Hover> {
    match slot {
        Slot::Faces => {
            let face = pointer.hover_face.as_ref()?;
            let body = pointer.body.clone()?;
            Some(Hover::Face {
                loops: face_loops(&body, face.point),
                body,
                at: face.point,
            })
        }
        Slot::Edges => {
            let edge = pointer.hover_edge.as_ref()?;
            Some(Hover::Edge {
                body: edge.body.clone(),
                at: edge.point,
                round: edge_loop(edge, up),
            })
        }
        Slot::Zero => {
            if let Some(v) = &pointer.vertex {
                return Some(Hover::Point {
                    body: v.body.clone(),
                    snap: Snap::Vertex,
                    at: v.point,
                    circle: Vec::new(),
                });
            }
            let edge = pointer.hover_edge.as_ref()?;
            let c = edge.circle.as_ref()?;
            Some(Hover::Point {
                body: edge.body.clone(),
                snap: Snap::CircleCenter,
                at: c.center,
                circle: circle(c),
            })
        }
    }
}

/// A round edge as a polyline.
fn circle(c: &EdgeCircle) -> Vec<[f32; 3]> {
    let n = c.normal;
    // Two directions square to the axis.
    let a = if n[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let cross = |u: [f32; 3], v: [f32; 3]| {
        [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ]
    };
    let norm = |v: [f32; 3]| {
        let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
        v.map(|x| x / l)
    };
    let u = norm(cross(n, a));
    let w = norm(cross(n, u));
    (0..=48)
        .map(|i| {
            let t = i as f32 / 48.0 * std::f32::consts::TAU;
            let (s, k) = t.sin_cos();
            std::array::from_fn(|j| c.center[j] + c.radius * (k * u[j] + s * w[j]))
        })
        .collect()
}

/// What an armed slot draws: what a click would take, over the part.
pub fn draw(frame: &mut Frame, armed: &Armed, color: Rgb) {
    let line = |points: Vec<[f32; 3]>| Polyline {
        points,
        color,
        width: 3.0,
        ..Default::default()
    };
    match &armed.hover {
        Some(Hover::Face { loops, .. }) => {
            frame.lines.extend(loops.iter().cloned().map(line));
        }
        Some(Hover::Edge { round, .. }) => frame.lines.push(line(round.clone())),
        Some(Hover::Point { at, circle, .. }) => {
            if !circle.is_empty() {
                frame.lines.push(line(circle.clone()));
            }
            frame.marks.push(Mark {
                at: *at,
                color,
                kind: MarkKind::Cross { size: 12.0 },
            });
        }
        None => {}
    }
    frame.hud.footer.push(match armed.slot {
        Slot::Faces => "Click faces to pick them; click a picked one to take it out".into(),
        Slot::Edges => "Click an edge to pick the loop it goes round".into(),
        Slot::Zero => "Click a corner or a round edge for the work zero".into(),
    });
}
