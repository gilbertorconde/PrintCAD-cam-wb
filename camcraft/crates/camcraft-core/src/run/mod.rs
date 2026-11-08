//! Running a setup: its operations in order, against one block of material.
//!
//! Every operation reads the stock as it currently stands and writes back what it
//! removed, so the next one never re-cuts cleared air. That ordering is the whole of
//! rest machining.
//!
//! Each operation is simulated as it runs, not at the end. A gouge in operation one
//! is worth knowing about before operation two is planned against the material it
//! left behind.
//!
//! A 3D operation whose boundary is its selection is kept to what its picked
//! faces and edges enclose ([`Picking::machining_boundary`]) as well as to the
//! stock's footprint. Picks nothing of the operation reads are kept, and never
//! found.

mod error;
mod operation;
mod pitch;
mod result;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::panic,
    clippy::indexing_slicing
)]
mod holder_tests;

use std::cell::OnceCell;
use std::sync::Arc;

use camcraft_sim::stock::{PackedStock, PartSurface, Stock, round_pitch};

use crate::cutter::allowance::{Allowance, EffectiveTool};
use crate::cutter::dropcutter::DropCutter;
use crate::job::{Job, JobError, Operation, Setup, StrategySpec, ToolSpec};
use crate::setup::features::{FeatureError, Picking};
use crate::setup::{Bodies, Prepared};

pub use crate::path::toolpath::PLUNGE_RATIO;
pub use error::RunError;
pub use result::{Clash, Clashes, Made, OperationResult, Plan, SetupResult, Stage, StrategyDetail};

use error::in_op;
use operation::{
    Target, Work, chamfer_curves, cut_kept, heights_of, hole_targets, not_made, planar_areas,
    run_operation,
};
use pitch::sim_pitch;

/// Material further above the part than this, in millimetres, counts as left over.
///
/// Leftover is not a failure; it is where the cutter could not reach. It is named
/// here so that anything drawing the material classifies it by the same rule the
/// verifier counts it by.
pub const LEFTOVER_LIMIT: f64 = 1.0;

/// Check, lay out and run setup `index` of `job`, with its models by name.
#[cfg(test)]
pub(crate) fn run_setup(
    job: &Job,
    index: usize,
    bodies: &Bodies<'_>,
) -> Result<SetupResult, RunError> {
    job.check_common()?;
    job.check_setup(index)?;
    let prepared = crate::setup::prepare(job, index, bodies)?;
    run_prepared_planned(job, index, &prepared, bodies, &[], &mut |_| {})
}

/// Run every operation of setup `index` against its block of material, already
/// laid out in its frame, telling `progress` each [`Stage`] as it starts. The job
/// must have been checked. `bodies` are the models it was laid out from, where
/// picked faces and edges are found.
///
/// Each operation is done as `plans` says, by its place: an operation with no
/// plan is worked out, and one turned off is left out whatever its plan. Kept
/// operations are cut into the material again, so what comes after them is
/// planned against what they leave.
pub fn run_prepared_planned(
    job: &Job,
    index: usize,
    prepared: &Prepared,
    bodies: &Bodies<'_>,
    plans: &[Plan],
    progress: &mut dyn FnMut(Stage),
) -> Result<SetupResult, RunError> {
    let setup = job.setups.get(index).ok_or(JobError::NoSetups)?;
    let mesh = &prepared.part;
    let all = prepared.stock_bounds();
    let lo = all.min.truncate();
    let hi = all.max.truncate();
    let ctx = prepared.heights();

    let narrowest = setup
        .operations
        .iter()
        .filter_map(|op| job.tool_for(op))
        .map(|t| t.diameter)
        .fold(None, |n: Option<f64>, d| Some(n.map_or(d, |n| n.min(d))));
    let sim_pitch = sim_pitch(job.tolerance, hi - lo, narrowest);
    // On a rotary, a whole number of cells make a turn.
    let round = prepared.frame.wrap.map(|w| w.round());
    let sim_pitch = round.map_or(sim_pitch, |r| round_pitch(&r, sim_pitch));
    let blocks = &prepared.stocks;
    // What an earlier setup left, carried into this frame, when it carries on
    // from one; its solid, when it starts from one; else its blocks, fresh,
    // on the rotary when it is on one.
    let fresh = || match round {
        Some(round) if !prepared.starts_shaped() => {
            Stock::from_blocks(blocks, ctx.stock_bottom - 1.0, sim_pitch)?.rounded(round)
        }
        _ => prepared.fresh_stock(ctx.stock_bottom - 1.0, sim_pitch),
    };
    let mut stock = fresh()?;
    // The part's top on the simulator's grid: what the tool above its flutes
    // is checked against as each operation cuts, and what the result is
    // verified against at the end.
    let surface = PartSurface::from_mesh(mesh, lo, hi, sim_pitch)?;
    let cutter = DropCutter::new(mesh);
    // Found once for the setup, when an operation first needs them.
    let flats = OnceCell::new();
    // The probed surface the setup's depths are corrected to, if they are.
    let probed = match setup.z_correction.as_ref().and_then(|z| z.map()) {
        Some(Ok(map)) => Some(map),
        Some(Err(e)) => {
            return Err(JobError::BadProbeMap {
                setup: setup.name.clone(),
                why: format!("are no map: {e}"),
            }
            .into());
        }
        None => None,
    };

    let mut operations = Vec::with_capacity(setup.operations.len());
    let mut keyframes = Vec::with_capacity(setup.operations.len() + 1);
    keyframes.push(Arc::new(PackedStock::pack(&stock)));
    for (i, op) in setup.operations.iter().enumerate() {
        let plan = plans.get(i).unwrap_or(&Plan::Generate);
        if !op.enabled || matches!(plan, Plan::Skip) {
            let made = if op.enabled {
                Made::Skipped
            } else {
                Made::Disabled
            };
            operations.push(not_made(op, job.tool_for(op), made));
            // Nothing cut: the material is as it was, and so is its keyframe.
            let same = keyframes
                .last()
                .map_or_else(|| Arc::new(PackedStock::pack(&stock)), Arc::clone);
            keyframes.push(same);
            continue;
        }
        progress(Stage::Operation(i));
        // Lines for the program, which cut nothing.
        if matches!(op.strategy, crate::job::StrategySpec::CustomGcode {}) {
            operations.push(not_made(op, None, Made::Text));
            let same = keyframes
                .last()
                .map_or_else(|| Arc::new(PackedStock::pack(&stock)), Arc::clone);
            keyframes.push(same);
            continue;
        }
        let (spec, tool) = tool_of(job, op)?;
        // Touched, not cut: nothing of the material changes.
        if matches!(op.strategy, StrategySpec::Probe { .. }) {
            operations.push(probe(job, op, spec, prepared, bodies, &ctx, (lo, hi))?);
            let same = keyframes
                .last()
                .map_or_else(|| Arc::new(PackedStock::pack(&stock)), Arc::clone);
            keyframes.push(same);
            continue;
        }
        // What was kept was corrected to the surface, and is worked out again
        // rather than cut uncorrected into the material the rest is planned
        // against.
        if let Plan::Keep(before) = plan
            && probed.is_none()
        {
            let mut kept = (**before).clone();
            (kept.sim, kept.clashes) = cut_kept(
                &mut stock,
                &kept.path,
                spec,
                &tool,
                job.tolerance,
                surface.field(),
            );
            kept.made = Made::Kept;
            operations.push(kept);
            keyframes.push(Arc::new(PackedStock::pack(&stock)));
            continue;
        }
        // Its picks, found once, however many of its settings read them.
        let picking = Picking::new(&op.selection, prepared, bodies, job.tolerance);
        let ctx = heights_of(op, &picking, &ctx)?;
        // A drilling operation's picked edges are its holes, a 2D one's
        // outline is what it cuts round or clears, not a limit, an
        // engraving's picks are what it follows, and a V-carving's what it
        // carves.
        let uses = op.strategy.uses();
        let target = if uses.boundary {
            Target::Limit(
                picking
                    .machining_boundary(&op.boundary, tool.cutter_radius())
                    .map_err(in_op(op))?,
            )
        } else if uses.outline {
            Target::Areas(planar_areas(job, op, &picking, bodies, &ctx, &tool)?)
        } else if uses.holes {
            Target::Holes(hole_targets(job, op, prepared, bodies, &ctx)?)
        } else if uses.traces {
            Target::Traces(picking.traces().map_err(in_op(op))?)
        } else if uses.carves {
            if op.selection.is_empty() {
                return Err(in_op(op)(FeatureError::NothingToCarve));
            }
            let area = picking
                .boundary_shape(crate::job::Boundary::Selection)
                .map_err(in_op(op))?
                .unwrap_or_default();
            Target::Carve(area)
        } else {
            // Facing works from the stock alone.
            Target::Limit(None)
        };
        let work = Work {
            job,
            op,
            spec,
            tool: &tool,
            heights: &ctx,
            prepared,
            cutter: &cutter,
            bounds: (lo, hi),
            blocks,
            flats: &flats,
            part: &surface,
        };
        operations.push(run_operation(&work, &target, &mut stock)?);
        keyframes.push(Arc::new(PackedStock::pack(&stock)));
    }

    // Planned against the stock as modelled, each operation is then moved
    // to the probed surface and cut again, in order, into a stock whose top
    // follows it, so what is simulated and checked is what will be cut.
    // The part follows the surface too: what the holder is checked against
    // and what the result is verified against.
    let surface = match &probed {
        Some(map) => {
            let segment = setup
                .z_correction
                .as_ref()
                .map_or(crate::job::DEFAULT_SEGMENT, |z| z.segment);
            let warped = surface.warped(|p| map.height(p));
            let (s, k) = corrected(
                job,
                setup,
                (map, segment),
                &mut operations,
                fresh()?.warped(|p| map.height(p)),
                warped.field(),
            )?;
            stock = s;
            keyframes = k;
            warped
        }
        None => surface,
    };

    // The simulator resolves the surface on a grid, so it cannot be held finer than
    // one cell plus the chordal tolerance the paths were built to; and that holds
    // across as well as down, or every vertical wall is a gouge beside it.
    progress(Stage::Verifying);
    let budget = job.tolerance + sim_pitch;
    let part = cut_past(job, setup, surface, (&operations, &keyframes))?;
    let part = Arc::new(part.reaching(budget));
    let verify = stock.verify(&part, budget, LEFTOVER_LIMIT);

    Ok(SetupResult {
        setup: setup.name.clone(),
        frame: prepared.frame,
        operations,
        part,
        verify,
        verify_budget: budget,
        stocks: prepared.stocks.clone(),
        stock_min: lo,
        stock_max: hi,
        heights: ctx,
        keyframes,
        probe_map: probed,
    })
}

/// A probe operation's run: the points of its grid, over what it is kept
/// to, the stock's footprint unless its boundary says otherwise, each
/// touched from its retract height at its top.
fn probe(
    job: &Job,
    op: &Operation,
    spec: &ToolSpec,
    prepared: &Prepared,
    bodies: &Bodies<'_>,
    ctx: &crate::job::HeightContext,
    (lo, hi): (glam::DVec2, glam::DVec2),
) -> Result<OperationResult, RunError> {
    let StrategySpec::Probe {
        x_count,
        y_count,
        x_spacing,
        y_spacing,
        inset,
        probe_feed,
        max_depth,
    } = op.strategy
    else {
        return Ok(not_made(op, Some(spec), Made::Skipped));
    };
    let picking = Picking::new(&op.selection, prepared, bodies, job.tolerance);
    let ctx = heights_of(op, &picking, ctx)?;
    // The probe's ball centre is where it is kept to: the area's own edge.
    let area = picking
        .machining_boundary(&op.boundary, 0.0)
        .map_err(in_op(op))?
        .and_then(|r| r.bounds())
        .map_or((lo, hi), |(a, b)| (a.max(lo), b.min(hi)));
    let (a, b) = (area.0 + inset, area.1 - inset);
    if a.x > b.x || a.y > b.y {
        return Err(in_op(op)(crate::strategy::StrategyError::EmptyRegion {
            min: a,
            max: b,
        }));
    }
    let xs = crate::probe::grid_lines(a.x, b.x, x_count, x_spacing);
    let ys = crate::probe::grid_lines(a.y, b.y, y_count, y_spacing);
    let clearance = op.heights.clearance.resolve(&ctx);
    let retract = op.heights.retract.resolve(&ctx);
    let (top, _) = op.levels_3d(&ctx);
    if retract <= top || clearance < retract {
        return Err(in_op(op)(crate::strategy::StrategyError::ProbeHeights {
            clearance,
            retract,
            top,
        }));
    }
    let path = crate::probe::probe_path(&xs, &ys, (clearance, retract, top), probe_feed)?;
    let points = xs.len() * ys.len();
    Ok(OperationResult {
        path,
        passes: points as u64,
        detail: StrategyDetail::Probe(crate::probe::ProbeReport {
            columns: xs.len(),
            rows: ys.len(),
            max_depth,
        }),
        clearance_z: clearance,
        ..not_made(op, Some(spec), Made::Generated)
    })
}

/// Every operation of `setup` with paths, but a probe's, moved to the probed
/// surface `map`, its moves split no longer than `segment`, and cut again in
/// order into `stock`, whose top is moved to the surface already, the holder
/// watched against the `part` moved likewise: that stock as they leave it,
/// and the material before the first and after each.
fn corrected(
    job: &Job,
    setup: &Setup,
    (map, segment): (&crate::probe::ProbeMap, f64),
    operations: &mut [OperationResult],
    mut stock: Stock,
    part: &camcraft_geom::heightmap::HeightMap,
) -> Result<(Stock, Vec<Arc<PackedStock>>), RunError> {
    let mut keyframes = vec![Arc::new(PackedStock::pack(&stock))];
    for (op, result) in setup.operations.iter().zip(operations.iter_mut()) {
        let probing = matches!(op.strategy, StrategySpec::Probe { .. });
        if result.made.has_paths() && !probing {
            let c = crate::probe::correct(&result.path, map, segment, job.tolerance)?;
            if c.outside > 0 {
                result.warnings.push(format!(
                    "{} point(s) of its path are outside the probed area, up to {:.1} mm: \
                     corrected as at its nearest edge",
                    c.outside, c.furthest
                ));
            }
            result.path = c.path;
            if let StrategyDetail::Drill(r) = &mut result.detail {
                crate::probe::correct_holes(&mut r.holes, map);
            }
            let (spec, tool) = tool_of(job, op)?;
            (result.sim, result.clashes) =
                cut_kept(&mut stock, &result.path, spec, &tool, job.tolerance, part);
            // What the simulator said of the path as planned is said again
            // of it as corrected.
            result
                .warnings
                .retain(|w| !w.contains("pass through material"));
            if result.sim.rapid_contacts > 0 {
                result.warnings.push(format!(
                    "{} rapid move(s) pass through material, deepest {:.2} mm",
                    result.sim.rapid_contacts, result.sim.deepest_rapid_contact_mm
                ));
            }
            let start = result
                .path
                .moves()
                .first()
                .map_or(glam::DVec3::ZERO, |m| m.to);
            let (steep, deepest) = result.path.steep_cut_descents(start, PLUNGE_RATIO);
            result.steep_cut_descents = steep;
            result.deepest_cut_descent_mm = deepest;
        }
        keyframes.push(Arc::new(PackedStock::pack(&stock)));
    }
    Ok((stock, keyframes))
}

/// The part as the setup's operations mean to leave it: cut past where one with
/// negative stock to leave cut, which is then no gouge there, and nowhere else;
/// cut into where an engraving or a V-carving cut, as deep as it cut, since
/// what they cut is drawn on the part, not modelled in it; and cut into where
/// a corner's relief cut past the corner, as it is meant to. Where each cut is
/// the material it took down, between the stock before it and after
/// (`keyframes`); where its reliefs are, its result's (`operations`).
fn cut_past(
    job: &Job,
    setup: &Setup,
    mut part: PartSurface,
    (operations, keyframes): (&[OperationResult], &[Arc<PackedStock>]),
) -> Result<PartSurface, RunError> {
    for (i, op) in setup.operations.iter().enumerate() {
        let leave = op.stock_to_leave();
        let (radial, axial) = (-leave.radial.min(0.0), -leave.axial.min(0.0));
        let uses = op.strategy.uses();
        // A thread's groove, past the wall it is cut in, is meant too: a
        // milled one's, or a tap's, wider than the hole it is tapped in;
        // and a spot's or a countersink's seat, wider than its hole.
        let into = uses.traces
            || uses.carves
            || matches!(
                op.strategy,
                StrategySpec::Thread { .. }
                    | StrategySpec::Drill { pitch: Some(_), .. }
                    | StrategySpec::Drill { spot: Some(_), .. }
            );
        let reliefs = operations.get(i).map_or(&[][..], |r| r.reliefs.as_slice());
        if !op.enabled || (radial == 0.0 && axial == 0.0 && !into && reliefs.is_empty()) {
            continue;
        }
        let (Some(before), Some(after)) = (keyframes.get(i), keyframes.get(i + 1)) else {
            continue;
        };
        let (before, after) = (before.unpack()?, after.unpack()?);
        let (before, after) = (before.field(), after.field());
        let lowered = |ix, iy| match (before.get(ix, iy), after.get(ix, iy)) {
            (Some(b), Some(a)) if a < b - 1e-6 => Some(a),
            _ => None,
        };
        part = if into {
            part.cut_into(lowered)
        } else if radial > 0.0 || axial > 0.0 {
            part.cut_past(radial, axial, |ix, iy| lowered(ix, iy).is_some())
        } else {
            part
        };
        if reliefs.is_empty() {
            continue;
        }
        // What the tool cut beside each relief, past where it reaches from
        // the corner itself: a cell's width either way, as the grid has it.
        let reach = job.tool_for(op).map_or(0.0, |t| t.diameter / 2.0) + leave.radial;
        let pitch = before.pitch();
        let relieved = |p: glam::DVec2| {
            reliefs.iter().any(|&[at, end]| {
                let along = end - at;
                let t = (p - at).dot(along) / along.length_squared().max(1e-12);
                let nearest = at + along * t.clamp(0.0, 1.0);
                p.distance(nearest) <= reach + pitch && p.distance(at) > reach - pitch
            })
        };
        part = part.cut_into(|ix, iy| {
            lowered(ix, iy).filter(|_| before.position(ix, iy).is_some_and(relieved))
        });
    }
    Ok(part)
}

/// Where a chamfer's tool's centre goes, at each edge's height, and where it
/// puts the cone, as a run of it finds them: for drawing what it cuts.
/// `None` for an operation that is no chamfer, or whose curves or tool do
/// not work out.
pub fn chamfer_centres(
    job: &Job,
    op: &Operation,
    prepared: &Prepared,
    bodies: &Bodies<'_>,
) -> Option<(
    Vec<crate::strategy::engrave::Trace>,
    crate::strategy::chamfer::Place,
)> {
    if !matches!(op.strategy, crate::job::StrategySpec::Chamfer { .. }) {
        return None;
    }
    let spec = job.tool_for(op)?;
    let picking = Picking::new(&op.selection, prepared, bodies, job.tolerance);
    let traces = picking.traces().ok()?;
    let cutter = DropCutter::new(&prepared.part);
    let clockwise = spec.spindle == crate::job::SpindleDirection::Clockwise;
    let (curves, place, _) =
        chamfer_curves(op, spec, &cutter, &traces, (clockwise, job.tolerance)).ok()?;
    Some((curves, place))
}

/// The tool an operation cuts with: the job's cutter, grown by the stock to
/// leave it cuts with to find where the tip goes, and swept as it is.
///
/// The one place this is built, so a replay of the material sweeps exactly the
/// tool the run did.
pub fn cutting_tool(job: &Job, op: &Operation) -> Result<EffectiveTool, RunError> {
    tool_of(job, op).map(|(_, tool)| tool)
}

/// The tool `op` names, both as the job describes it and as it cuts
/// ([`cutting_tool`]), looked up and built once.
fn tool_of<'j>(job: &'j Job, op: &Operation) -> Result<(&'j ToolSpec, EffectiveTool), RunError> {
    let spec = job.tool_for(op).ok_or_else(|| unknown_tool(op))?;
    let base = spec.build()?;
    let leave = op.stock_to_leave();
    let leave = Allowance::new(leave.radial, leave.axial)?;
    Ok((spec, EffectiveTool::of(&base, leave)?))
}

fn unknown_tool(op: &Operation) -> JobError {
    match &op.tool {
        Some(tool) => JobError::UnknownTool {
            op: op.name.clone(),
            tool: tool.clone(),
        },
        None => JobError::NoTool {
            op: op.name.clone(),
        },
    }
}
