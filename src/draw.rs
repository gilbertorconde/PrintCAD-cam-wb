//! What the view draws: each setup's stock and work zero, its operations'
//! toolpaths, and the material it leaves.

use std::rc::Rc;

use cam_protocol::{Cylinder, HeightPlane, Run};
use printcad_bench_sdk::api::*;
use printcad_bench_sdk::host;

use crate::playback::{material_at, tool_at, tool_tip};
use crate::runs::State;
use crate::{Cam, Kind, Lit, operations_of, setups};

const ACCENT: Rgb = [0.31, 0.64, 0.9];
const MUTED: Rgb = [0.55, 0.58, 0.62];
/// What a panel's row has the pointer on: a body, an operation's path.
pub(crate) const LIT: Rgb = [1.0, 0.82, 0.3];
pub(crate) const LIT_OPACITY: f32 = 0.45;
const STOCK: Rgb = [0.9, 0.62, 0.25];
const RAPID: Rgb = [0.9, 0.3, 0.25];
const ZERO: Rgb = [0.35, 0.8, 0.4];
/// A fixture, and one a tool meets.
const FIXTURE: Rgb = [0.62, 0.45, 0.85];
const FIXTURE_MET: Rgb = [0.95, 0.2, 0.2];
const MATERIAL: Rgb = [0.78, 0.66, 0.5];
/// camcraft's colours for a boundary, and for the hatching over what the
/// tool cuts outside the part: amber, the darkest for the hatch.
const BOUNDARY: Rgb = [0.95, 0.65, 0.2];
const HATCH: Rgb = [0.75, 0.45, 0.1];
const CLEARANCE: Rgb = [0.35, 0.8, 0.4];
const RETRACT: Rgb = [0.3, 0.75, 0.75];
const TOP: Rgb = [0.31, 0.64, 0.9];
const BOTTOM: Rgb = [0.75, 0.45, 0.85];

/// The most toolpath points a frame draws, every setup's together. printCAD
/// takes the frame within a few milliseconds, and asks for it every frame
/// while a run plays back: writing out a point takes about a tenth of a
/// microsecond, so a frame of every operation of a few big setups in full,
/// a hundred thousand points and more, takes most of that time on its own.
const FRAME_POINTS: usize = 30_000;

pub fn draw(cam: &Cam, frame: &mut Frame) {
    let editing = cam.editing.as_ref();
    // What is drawn, before drawing it, to share the points out.
    let mut drawn = Vec::new();
    for (node, _) in setups() {
        let Some(computed) = cam.runs.get(&node.id) else {
            continue;
        };
        let ops = operations_of(&node.id);
        let edited_setup = editing.is_some_and(|e| e.kind == Kind::Setup && e.id == node.id);
        let edited_op = editing
            .filter(|e| e.kind == Kind::Operation)
            .and_then(|e| ops.iter().position(|(n, _)| n.id == e.id));
        if !node.visible && !(edited_setup || edited_op.is_some()) {
            continue;
        }
        drawn.push((node, ops, computed, edited_setup, edited_op));
    }
    // The edited operation first, then the rest of its setup, or the edited
    // setup, then the others: each in full while it fits, else sharing what
    // is left.
    let mut wanted = [Vec::new(), Vec::new(), Vec::new()];
    for (s, (_, ops, computed, edited_setup, edited_op)) in drawn.iter().enumerate() {
        let Some(run) = computed.shown() else {
            continue;
        };
        for (i, op) in run.operations.iter().enumerate() {
            let this = *edited_op == Some(i);
            if !(ops.get(i).is_none_or(|(n, _)| n.visible) || this) {
                continue;
            }
            let points = op.strokes.iter().map(|k| k.points.len()).sum();
            let group = if this {
                0
            } else if *edited_setup || edited_op.is_some() {
                1
            } else {
                2
            };
            wanted[group].push(((s, i), points));
        }
    }
    let share: std::collections::HashMap<(usize, usize), usize> = wanted
        .iter()
        .zip(shares(
            &wanted
                .iter()
                .map(|g| g.iter().map(|(_, p)| *p).collect())
                .collect::<Vec<_>>(),
            FRAME_POINTS,
        ))
        .flat_map(|(g, s)| g.iter().map(|(k, _)| *k).zip(s))
        .collect();
    for (s, (node, _, computed, edited_setup, edited_op)) in drawn.iter().enumerate() {
        let (edited_setup, edited_op) = (*edited_setup, *edited_op);
        let mine = edited_setup || edited_op.is_some();
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
        // Each other part: its stock and its work zero, where it is.
        for p in run.placements.iter().skip(1) {
            let moved = |c: [f64; 3]| std::array::from_fn(|i| c[i] + p.offset[i]);
            box_lines(
                frame,
                &run.stock.map(moved),
                if mine { STOCK } else { MUTED },
                true,
            );
            frame.marks.push(Mark {
                at: moved(run.origin).map(|v| v as f32),
                color: ZERO,
                kind: MarkKind::Cross { size: 8.0 },
            });
        }
        for f in &run.fixtures {
            let color = if f.met { FIXTURE_MET } else { FIXTURE };
            box_lines(frame, &f.corners, if mine { color } else { MUTED }, false);
        }
        frame.marks.push(Mark {
            at: run.origin.map(|v| v as f32),
            color: ZERO,
            kind: MarkKind::Cross { size: 8.0 },
        });
        // The moment being played back, while the setup is open.
        let playing = cam.playing.as_ref().filter(|p| mine && p.setup == node.id);
        let moment = playing.and_then(|p| {
            let play = run.playback.as_ref()?;
            Some((play, p.moment(play)?))
        });
        // At full resolution once the playback session answers.
        let exact = moment.and(
            cam.session
                .as_ref()
                .filter(|s| s.setup == node.id)
                .and_then(|s| Some((s.meshes()?, s.at.as_ref()?))),
        );
        // How far the tool has gone: which operation, how many of its drawn
        // points, and at which part.
        let gone = match (exact, moment) {
            (Some((_, at)), _) => Some((at.op, at.drawn, at.part)),
            (None, Some((_, m))) => Some((m.op, m.drawn, m.part)),
            _ => None,
        };
        // Toolpaths are drawn at the part being cut.
        let shift: [f32; 3] = gone
            .and_then(|(_, _, part)| run.placements.get(part))
            .map_or([0.0; 3], |p| p.offset.map(|v| v as f32));
        if let Some((meshes, _)) = exact {
            for mesh in meshes {
                frame.kept.push(KeptMesh {
                    handle: *mesh,
                    color: crate::session::TINT,
                    opacity: 0.92,
                    ..Default::default()
                });
            }
        } else if let (Some(p), Some((play, _))) = (playing, moment) {
            // Each part's material as far as it is cut.
            for (part, m) in p
                .materials(play, run.placements.len())
                .into_iter()
                .enumerate()
            {
                let offset = run
                    .placements
                    .get(part)
                    .map_or([0.0; 3], |p| p.offset.map(|v| v as f32));
                let make = || material_at(play, m, offset);
                let mesh = computed
                    .shown_rc()
                    .and_then(|run| cam.material.borrow_mut().mesh(&run, part, Some(m), make));
                match mesh {
                    Some(handle) => frame.kept.push(material(handle)),
                    None => frame.meshes.push(make()),
                }
            }
        } else if edited_setup
            && cam.show_material
            && let Some(m) = &run.material
        {
            let left = || Mesh {
                positions: m.positions.clone(),
                indices: m.indices.clone(),
                color: MATERIAL,
                opacity: 0.85,
                ..Default::default()
            };
            let mesh = computed
                .shown_rc()
                .and_then(|run| cam.material.borrow_mut().mesh(&run, 0, None, left));
            match mesh {
                Some(handle) => frame.kept.push(material(handle)),
                None => frame.meshes.push(left()),
            }
        }
        for (i, op) in run.operations.iter().enumerate() {
            let this = edited_op == Some(i);
            let Some(&share) = share.get(&(s, i)) else {
                continue;
            };
            let points: usize = op.strokes.iter().map(|s| s.points.len()).sum();
            // No room left for it, unless it is the one edited.
            if !this && share < 2 && points > 0 {
                continue;
            }
            // Every so many of its points, past what its share allows.
            let every = points.div_ceil(share.max(1)).max(1);
            if this {
                for plane in &op.heights {
                    height_plane(frame, plane);
                }
                if let Some(o) = &op.outlines {
                    outlines(frame, o);
                }
            }
            let lit =
                matches!(&cam.lit, Some(Lit::Operation(setup, k)) if *setup == node.id && *k == i);
            let color = if lit {
                LIT
            } else if this || edited_setup {
                ACCENT
            } else {
                MUTED
            };
            // Played back, as far as the tool has gone.
            let mut left = match gone {
                Some((op, _, _)) if i > op => continue,
                Some((op, drawn, _)) if i == op => drawn,
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
                let mut points = thinned(points, every);
                if shift != [0.0; 3] {
                    for p in &mut points {
                        *p = std::array::from_fn(|k| p[k] + shift[k]);
                    }
                }
                frame.lines.push(Polyline {
                    points,
                    color: if stroke.rapid { RAPID } else { color },
                    width: if lit {
                        2.0
                    } else if this {
                        1.5
                    } else {
                        1.0
                    },
                    dashed: stroke.rapid,
                    ..Default::default()
                });
            }
            if this && op.points > points.min(share) {
                frame.hud.footer.push(format!(
                    "Toolpath drawn thinned: {} of {} points",
                    points.min(share),
                    op.points
                ));
            }
        }
        match (exact, moment) {
            (Some((_, at)), Some((play, _))) => tool_tip(frame, play, at.tool, STOCK),
            (None, Some((play, m))) => tool_at(frame, play, m, STOCK),
            _ => {}
        }
    }
    match &cam.lit {
        Some(Lit::Body(_, Ok(kept))) => frame.kept.push(KeptMesh {
            handle: kept.0,
            color: LIT,
            opacity: LIT_OPACITY,
            on_top: true,
            ..Default::default()
        }),
        Some(Lit::Body(_, Err(mesh))) => frame.meshes.push(mesh.clone()),
        _ => {}
    }
}

/// The material a run leaves, or leaves at a moment of its playback, kept
/// by the host: written out with every frame it is a fifth of a megabyte
/// and more, and a playback asks for a frame every frame. A mesh a part.
#[derive(Default)]
pub struct KeptMaterial {
    /// The run it is of, kept so another run is never taken for it.
    run: Option<Rc<Run>>,
    /// Each part's: which moment of the run's playback, by where it is
    /// (`None` for the material the run leaves), and its mesh.
    parts: Vec<(Option<usize>, u64)>,
}

impl KeptMaterial {
    /// The kept mesh of `run`'s material at part `part` at `moment` (`None`
    /// at its end), `make` making it when it is not the one kept; `None`
    /// when the host keeps none.
    fn mesh(
        &mut self,
        run: &Rc<Run>,
        part: usize,
        moment: Option<&cam_protocol::Moment>,
        make: impl FnOnce() -> Mesh,
    ) -> Option<u64> {
        let at = moment.map(|m| std::ptr::from_ref(m) as usize);
        if !self.run.as_ref().is_some_and(|r| Rc::ptr_eq(r, run)) {
            self.forget();
            self.run = Some(run.clone());
        }
        if let Some(&(was, handle)) = self.parts.get(part) {
            if was == at {
                return Some(handle);
            }
            let mesh = make();
            // Another moment of the same playback: the same grid, risen.
            if at.is_some()
                && was.is_some()
                && host::mesh_update(handle, 0, &mesh.positions, &[]).is_ok()
            {
                self.parts[part].0 = at;
                return Some(handle);
            }
            let made = host::mesh_create(&mesh.positions, &mesh.indices, &[]).ok()?;
            host::mesh_remove(handle);
            self.parts[part] = (at, made);
            return Some(made);
        }
        // Parts are asked for in turn, the first first.
        if part != self.parts.len() {
            return None;
        }
        let mesh = make();
        let handle = host::mesh_create(&mesh.positions, &mesh.indices, &[]).ok()?;
        self.parts.push((at, handle));
        Some(handle)
    }

    fn forget(&mut self) {
        for (_, handle) in self.parts.drain(..) {
            host::mesh_remove(handle);
        }
        self.run = None;
    }
}

fn material(handle: u64) -> KeptMesh {
    KeptMesh {
        handle,
        color: MATERIAL,
        opacity: 0.85,
        ..Default::default()
    }
}

/// How many points each of `groups`' toolpaths (how many points each has)
/// is drawn with, within `budget`: a group in full while it fits in what
/// the groups before it left, else its toolpaths sharing that in
/// proportion to their points.
fn shares(groups: &[Vec<usize>], budget: usize) -> Vec<Vec<usize>> {
    let mut left = budget;
    groups
        .iter()
        .map(|group| {
            let total: usize = group.iter().sum();
            if total <= left {
                left -= total;
                return group.clone();
            }
            let out: Vec<usize> = group.iter().map(|p| p * left / total).collect();
            left -= out.iter().sum::<usize>();
            out
        })
        .collect()
}

/// Every `every`th of `points`, and the last, so a stroke still ends where
/// it does.
fn thinned(points: &[[f32; 3]], every: usize) -> Vec<[f32; 3]> {
    if every <= 1 {
        return points.to_vec();
    }
    let mut out: Vec<[f32; 3]> = points.iter().step_by(every).copied().collect();
    if let Some(&last) = points.last()
        && !(points.len() - 1).is_multiple_of(every)
    {
        out.push(last);
    }
    out
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

/// What the edited operation machines, as camcraft's app draws it: the
/// boundary as chosen, wide; where the tool's centre may go against it,
/// thin; what the tool cuts outside the part, hatched.
fn outlines(frame: &mut Frame, o: &cam_protocol::Outlines) {
    for ring in &o.boundary {
        frame.lines.push(Polyline {
            points: ring.clone(),
            color: BOUNDARY,
            width: 2.5,
            ..Default::default()
        });
    }
    for ring in &o.limit {
        frame.lines.push(Polyline {
            points: ring.clone(),
            color: BOUNDARY,
            width: 1.0,
            dashed: true,
            ..Default::default()
        });
    }
    for piece in &o.cut {
        frame.regions.push(Region {
            outline: piece.outline.clone(),
            holes: piece.holes.clone(),
            color: HATCH,
            opacity: 0.12,
            hatch: Some(2.0),
            hatch_angle: 45.0,
            on_top: false,
        });
    }
}

/// The stock's box, as twelve edges: turned with the part, for a tilted
/// setup.
fn stock_box(frame: &mut Frame, run: &Run, mine: bool) {
    let c = run.stock;
    if c[0] == c[7] {
        return;
    }
    let color = if mine { STOCK } else { MUTED };
    if let Some(c) = &run.cylinder {
        frame.lines.extend(cylinder(c, color));
        return;
    }
    box_lines(frame, &c, color, true);
}

/// A box by its corners as its twelve edges. Corner `i` is high in X, Y and
/// Z where bits 1, 2 and 4 of it are.
fn box_lines(frame: &mut Frame, c: &[[f64; 3]; 8], color: Rgb, dashed: bool) {
    let p = |i: usize| c[i].map(|v| v as f32);
    for z in [0, 4] {
        frame.lines.push(Polyline {
            points: [0, 1, 3, 2].iter().map(|i| p(i + z)).collect(),
            color,
            dashed,
            closed: true,
            ..Default::default()
        });
    }
    for i in [0, 1, 3, 2] {
        frame.lines.push(Polyline {
            points: vec![p(i), p(i + 4)],
            color,
            dashed,
            ..Default::default()
        });
    }
}

/// How many sides a drawn circle of stock has.
const ROUND: usize = 72;

/// A round bar of stock as an outline: a circle round each end, and four
/// lines along it between them.
fn cylinder(c: &Cylinder, color: Rgb) -> Vec<Polyline> {
    let [a, b] = c.ends;
    let along = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    // Two directions square to the axis and to each other: across it from
    // whichever document axis it lies least along, as a tilted setup's need
    // not lie along any.
    let cross = |p: [f64; 3], q: [f64; 3]| {
        [
            p[1] * q[2] - p[2] * q[1],
            p[2] * q[0] - p[0] * q[2],
            p[0] * q[1] - p[1] * q[0],
        ]
    };
    let unit = |p: [f64; 3]| {
        let l = p.iter().map(|c| c * c).sum::<f64>().sqrt().max(1e-12);
        p.map(|c| c / l)
    };
    let least = (0..3)
        .min_by(|&i, &j| along[i].abs().total_cmp(&along[j].abs()))
        .unwrap_or(0);
    let mut other = [0.0; 3];
    other[least] = 1.0;
    let u = unit(cross(along, other));
    let v = unit(cross(along, u));
    let at = |end: [f64; 3], angle: f64| -> [f32; 3] {
        let (s, c_) = angle.sin_cos();
        std::array::from_fn(|i| (end[i] + c.radius * (c_ * u[i] + s * v[i])) as f32)
    };
    let mut out: Vec<Polyline> = [a, b]
        .into_iter()
        .map(|end| Polyline {
            points: (0..ROUND)
                .map(|k| at(end, std::f64::consts::TAU * k as f64 / ROUND as f64))
                .collect(),
            color,
            dashed: true,
            closed: true,
            ..Default::default()
        })
        .collect();
    for k in 0..4 {
        let angle = std::f64::consts::FRAC_PI_2 * f64::from(k);
        out.push(Polyline {
            points: vec![at(a, angle), at(b, angle)],
            color,
            dashed: true,
            ..Default::default()
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The edited operation is drawn in full; the rest share what is left
    /// by their size, and what does not fit is not drawn.
    #[test]
    fn a_frame_s_points_go_to_the_edited_operation_first() {
        let got = shares(&[vec![20_000], vec![4_000, 12_000], vec![50_000]], 30_000);
        assert_eq!(got, vec![vec![20_000], vec![2_500, 7_500], vec![0]]);
        let got = shares(&[vec![], vec![1_000, 2_000], vec![5_000, 5_000]], 30_000);
        assert_eq!(got, vec![vec![], vec![1_000, 2_000], vec![5_000, 5_000]]);
    }

    /// Thinned, a stroke keeps its ends.
    #[test]
    fn a_thinned_stroke_keeps_its_ends() {
        let p = |i: usize| [i as f32, 0.0, 0.0];
        let stroke: Vec<[f32; 3]> = (0..10).map(p).collect();
        assert_eq!(thinned(&stroke, 1), stroke);
        assert_eq!(thinned(&stroke, 3), vec![p(0), p(3), p(6), p(9)]);
        assert_eq!(thinned(&stroke, 4), vec![p(0), p(4), p(8), p(9)]);
        assert_eq!(thinned(&stroke[..1], 4), vec![p(0)]);
    }
}
