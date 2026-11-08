//! A hole operation's holes, found from its picks, and its paths: drilled
//! (`clearing::drill_paths`), bored, milled in circles or threaded.
//!
//! A hole is picked by a rim, as drilling always took them, or by its wall:
//! a round face standing along the tool, whose rims are its top and its
//! bottom. With the same diameter asked, every such wall of the setup's
//! models as wide as one picked, a hole's or a boss's as it was, is taken
//! too: Fusion's Select Same Diameter. With a diameter asked, every hole
//! that wide is, picked or not.

use camcraft_geom::topology::{Body, EdgeKind, FaceKind};

use super::*;
use crate::setup::features::{chain_of, find_face, tolerance};
use crate::setup::frame::Frame;
use crate::strategy::drilling::{HoleError, Rim, Round, hole_order, rounds};
use crate::strategy::holes::{
    BoreParams, CircularParams, HoleReport, ThreadParams, bore, circular, thread,
};

/// A round wall standing along the tool: its rims, their radius, and
/// whether it is a boss's.
struct Wall {
    rims: Vec<u32>,
    radius: f64,
    boss: bool,
}

/// Face `face` of `body`, as a round wall standing along the setup's tool
/// in `frame`, if it is one: curved, every point of it `tol` from one
/// radius about one axis, and bounded by round edges about that axis at two
/// heights or more, its top and its bottom.
fn wall(body: &Body, face: u32, frame: &Frame, tol: f64) -> Option<Wall> {
    let t = body.topology();
    let f = t.face(face)?;
    if !matches!(f.kind, FaceKind::Curved) {
        return None;
    }
    let mut circles = Vec::new();
    for &e in &f.edges {
        if let EdgeKind::Circle(c) = t.edge(e)?.kind {
            let up = frame.turn.apply(c.normal);
            if up.truncate().length() > 1e-6 {
                return None;
            }
            circles.push((e, frame.to_setup(c.center), c.radius));
        }
    }
    let &(_, first, radius) = circles.first()?;
    let axis = first.truncate();
    let same = circles
        .iter()
        .all(|(_, c, r)| (r - radius).abs() <= tol && c.truncate().distance(axis) <= tol);
    let (lo, hi) = circles
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), (_, c, _)| {
            (lo.min(c.z), hi.max(c.z))
        });
    if !same || hi - lo <= tol {
        return None;
    }
    // Every corner of it on the cylinder, and which way it faces: out from
    // the axis is a boss's, in toward it a hole's.
    let mut outward = 0.0;
    for &tri in &f.triangles {
        let [a, b, c] = body
            .mesh()
            .triangle(tri as usize)?
            .map(|p| frame.to_setup(p));
        for p in [a, b, c] {
            if ((p.truncate() - axis).length() - radius).abs() > tol {
                return None;
            }
        }
        let normal = (b - a).cross(c - a);
        let mid = (a + b + c) / 3.0;
        outward += normal.truncate().dot(mid.truncate() - axis);
    }
    Some(Wall {
        rims: circles.into_iter().map(|(e, _, _)| e).collect(),
        radius,
        boss: outward > 0.0,
    })
}

/// Whether the round edge `edge` of `body` is a boss's rim: the round wall
/// it bounds faces out. A rim with no such wall is a hole's, as drilling
/// takes it.
fn boss_rim(body: &Body, edge: u32, frame: &Frame, tol: f64) -> bool {
    body.topology().edge(edge).is_some_and(|e| {
        e.faces
            .iter()
            .find_map(|&f| wall(body, f, frame, tol))
            .is_some_and(|w| w.boss)
    })
}

/// A hole operation's holes, in the order it takes them: its picked rims,
/// and the rims of its picked walls, each axis one hole (or boss) from its
/// highest rim to its lowest, or to the operation's bottom where it has
/// rims at one height only; then, with the same diameter asked, every wall
/// of the setup's models as wide as one of them.
pub(in crate::run) fn hole_targets(
    job: &Job,
    op: &Operation,
    prepared: &Prepared,
    bodies: &Bodies<'_>,
    ctx: &HeightContext,
) -> Result<Vec<Round>, RunError> {
    let frame = &prepared.frame;
    let tol = job.tolerance.max(0.01);
    let mut chains =
        picked_chains(&op.selection, prepared, bodies, job.tolerance).map_err(in_op(op))?;
    for pick in &op.selection.faces {
        let body = *bodies
            .get(pick.model.as_str())
            .ok_or_else(|| in_op(op)(FeatureError::NotLoaded(pick.model.clone())))?;
        let face = find_face(body, pick, tolerance(job.tolerance, body))
            .map_err(in_op(op))?
            .index;
        let w = wall(body, face, frame, tol + body.topology().tolerance()).ok_or_else(|| {
            in_op(op)(HoleError::NotRoundFace {
                model: pick.model.clone(),
                face,
            })
        })?;
        chains.extend(
            w.rims
                .iter()
                .filter_map(|&e| chain_of(body, &pick.model, e, frame)),
        );
    }
    let rim = |c: &Chain| {
        let body = bodies.get(c.model.as_str())?;
        let EdgeKind::Circle(circle) = body.topology().edge(c.edge)?.kind else {
            return None;
        };
        Some(Rim {
            centre: frame.to_setup(circle.center),
            radius: circle.radius,
            boss: boss_rim(body, c.edge, frame, tol + body.topology().tolerance()),
        })
    };
    let (_, bottom) = op.levels_3d(ctx);
    // Holes of a diameter asked for need no pick.
    let mut found = if chains.is_empty() && op.holes.diameter.is_some() {
        Vec::new()
    } else {
        rounds(&chains, rim, bottom, tol).map_err(in_op(op))?
    };
    if op.holes.same_diameter || op.holes.diameter.is_some() {
        let mut wanted: Vec<(f64, bool)> = if op.holes.same_diameter {
            found.iter().map(|r| (r.hole.radius, r.boss)).collect()
        } else {
            Vec::new()
        };
        wanted.extend(op.holes.diameter.map(|d| (d / 2.0, false)));
        let mut more = Vec::new();
        for piece in &prepared.pieces {
            let Some(&body) = bodies.get(piece.model.as_str()) else {
                continue;
            };
            let within = tol + body.topology().tolerance();
            for face in 0..body.topology().faces().len() {
                let Ok(face) = u32::try_from(face) else {
                    continue;
                };
                let Some(w) = wall(body, face, frame, within) else {
                    continue;
                };
                if wanted
                    .iter()
                    .any(|&(r, boss)| boss == w.boss && (r - w.radius).abs() <= within)
                {
                    more.extend(
                        w.rims
                            .iter()
                            .filter_map(|&e| chain_of(body, &piece.model, e, frame)),
                    );
                }
            }
        }
        if !more.is_empty() {
            for r in rounds(&more, rim, bottom, tol).map_err(in_op(op))? {
                let known = found
                    .iter()
                    .any(|f| f.boss == r.boss && f.hole.at.distance(r.hole.at) <= tol);
                if !known {
                    found.push(r);
                }
            }
        }
    }
    if let (true, Some(diameter)) = (found.is_empty(), op.holes.diameter) {
        return Err(in_op(op)(HoleError::NoneAsWide { diameter }));
    }
    // Drilled and bored from inside: a boss has none.
    if matches!(
        op.strategy,
        StrategySpec::Drill { .. } | StrategySpec::Bore { .. }
    ) && let Some(b) = found.iter().find(|r| r.boss)
    {
        return Err(in_op(op)(HoleError::Boss {
            x: b.hole.at.x,
            y: b.hole.at.y,
        }));
    }
    let points: Vec<DVec2> = found.iter().map(|r| r.hole.at).collect();
    Ok(hole_order(&points, op.holes.order, tol)
        .into_iter()
        .filter_map(|i| found.get(i).copied())
        .collect())
}

/// Each of `rounds` bored, milled in circles or threaded, as `work`'s
/// operation says; a hole bored wider than twice the tool, which leaves a
/// core in its middle, said.
pub(super) fn milled_paths(
    work: &Work<'_>,
    rounds: &[Round],
    warnings: &mut Vec<String>,
) -> Result<Paths, RunError> {
    let op = work.op;
    let tool_radius = work.spec.diameter / 2.0;
    let leave = op.stock_to_leave();
    let leave = (leave.radial, leave.axial);
    let ccw = work.kept_on_right();
    let path = match op.strategy {
        StrategySpec::Bore { pitch, .. } => {
            let finishing = op
                .finishing(work.spec.diameter)
                .map(|f| (f.passes, f.stepover, f.feed, f.repeat));
            let params = BoreParams {
                tool_radius,
                leave,
                pitch,
                ccw,
                feed: op.passes.feed,
                retract_feed: op.linking.retract_feed,
                finishing,
            };
            let (path, cores) = bore(rounds, &params).map_err(in_op(op))?;
            for h in cores.iter().filter_map(|&i| rounds.get(i)) {
                warnings.push(format!(
                    "the hole at ({:.3}, {:.3}) is more than twice the tool across: the \
                     helix leaves a core standing in its middle; clear it first, or mill \
                     it in circles",
                    h.hole.at.x, h.hole.at.y
                ));
            }
            path
        }
        StrategySpec::Circular { boss_width, .. } => {
            let params = CircularParams {
                tool_radius,
                leave,
                stepover: op.passes.stepover,
                stepdown: op.stepdown(),
                ramp_angle: op.linking.ramp_angle.to_radians(),
                boss_width,
                ccw,
                feed: op.passes.feed,
                ramp_feed: op.ramp_feed(),
                retract_feed: op.linking.retract_feed,
            };
            circular(rounds, &params).map_err(in_op(op))?
        }
        StrategySpec::Thread {
            pitch,
            hand,
            diameter,
            radial_passes,
            spring_pass,
            ..
        } => {
            let params = ThreadParams {
                pitch,
                hand,
                diameter,
                passes: radial_passes,
                spring_pass,
                tool_radius,
                tip_length: work.spec.tip_length,
                teeth: work.spec.threads_per_turn(),
                ccw,
                feed: op.passes.feed,
                plunge_feed: op.linking.plunge_feed,
                retract_feed: op.linking.retract_feed,
            };
            thread(rounds, &params).map_err(in_op(op))?.0
        }
        _ => Toolpath::new(),
    };
    let report = HoleReport {
        rounds: rounds.to_vec(),
    };
    Ok((path, rounds.len() as u64, StrategyDetail::Holes(report)))
}
