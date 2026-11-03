//! The paths of the strategies that follow what was drawn: engraving along
//! curves, and V-carving down the middle of areas.

use super::*;

/// The engraving of `traces`, `depth` below the top, each run the way
/// `direction` and `reverse` say, and down to the depth as `levels` says: a
/// slot is an engraving deeper, with a tool its width.
pub(super) fn engrave_paths(
    work: &Work<'_>,
    (depth, direction, reverse): (f64, crate::job::TraceDirection, bool),
    levels: Levels,
    traces: &[Trace],
    warnings: &mut Vec<String>,
) -> Result<Paths, RunError> {
    let op = work.op;
    let (top, _) = op.levels_3d(work.heights);
    let params = EngraveParams {
        top,
        depth,
        stepdown: op.stepdown(),
        ramp_angle: op.linking.ramp_angle.to_radians(),
        feed: op.passes.feed,
        ramp_feed: op.ramp_feed(),
        plunge_feed: op.linking.plunge_feed,
        direction,
        reverse,
        levels,
        climb_right: work.clockwise(),
        tolerance: work.job.tolerance,
    };
    let (path, report) = engrave(traces, &params).map_err(in_op(op))?;
    if report.plunged > 0 {
        warnings.push(format!(
            "{} curve(s) too short to ramp down along at {:.1}°, so it goes straight \
             down at their start",
            report.plunged, op.linking.ramp_angle
        ));
    }
    Ok((path, report.passes, StrategyDetail::Engrave(report)))
}

/// A slot along `traces`, as [`engrave_paths`]: as drawn, unless reversed,
/// since a slot cuts both its walls, and back and forth or one way as
/// `pattern` says.
pub(super) fn slot_paths(
    work: &Work<'_>,
    (depth, pattern, reverse): (f64, SlotPattern, bool),
    traces: &[Trace],
    warnings: &mut Vec<String>,
) -> Result<Paths, RunError> {
    let levels = match pattern {
        SlotPattern::Bidirectional => Levels::Ramped,
        SlotPattern::Directional => Levels::OneWay,
    };
    engrave_paths(
        work,
        (depth, crate::job::TraceDirection::AsDrawn, reverse),
        levels,
        traces,
        warnings,
    )
}

/// Where a chamfer's tool's centre goes along `traces`, and where it puts
/// the cone, for `op` cutting with `spec`: each curve moved away from the
/// material on the part `cutter` drops onto, at its own height, run so that
/// cutting climbs with a `clockwise` spindle. And what finding them found.
pub(crate) fn chamfer_curves(
    op: &Operation,
    spec: &ToolSpec,
    cutter: &DropCutter<'_>,
    traces: &[Trace],
    (clockwise, tolerance): (bool, f64),
) -> Result<(Vec<Trace>, Place, Centres), RunError> {
    let StrategySpec::Chamfer {
        width,
        tip_offset,
        side,
    } = op.strategy
    else {
        return Ok((Vec::new(), Place::default(), Centres::default()));
    };
    let half = (spec.angle.unwrap_or(f64::NAN) / 2.0).to_radians();
    let place = Place::new(
        width,
        tip_offset,
        half,
        spec.tip_diameter / 2.0,
        spec.diameter / 2.0,
    )
    .map_err(|e| in_op(op)(StrategyError::from(e)))?;
    // The part's height at a point: a probe a tolerance across, looked at
    // far enough either side of an edge to be clear of it, near enough to
    // be on what is beside it.
    let probe = Flat::new(tolerance.max(1e-3))?;
    let height = |p: DVec2| cutter.drop_at(&probe, p.x, p.y);
    let aside = (5.0 * tolerance).max(0.25);
    let side_of = |t: &Trace| material_side(t, &height, aside, tolerance);
    let (curves, found) = centres(
        traces,
        &side_of,
        place.offset,
        side == ChamferSide::Flipped,
        clockwise,
        tolerance,
    );
    Ok((curves, place, found))
}

/// The chamfer along `traces`: each edge's curve, moved beside it away from
/// the material, engraved from the edge's height down to the cone's depth,
/// nearest first. The tool is a cone that fits ([`Operation::cannot_use`]).
pub(super) fn chamfer_paths(
    work: &Work<'_>,
    traces: &[Trace],
    warnings: &mut Vec<String>,
) -> Result<Paths, RunError> {
    let op = work.op;
    let tolerance = work.job.tolerance;
    let (curves, place, found) = chamfer_curves(
        op,
        work.spec,
        work.cutter,
        traces,
        (work.clockwise(), tolerance),
    )?;
    if found.guessed > 0 {
        warnings.push(format!(
            "{} curve(s) have the part as high either side, so which side the material is \
             on is a guess: the tool goes outside a closed one and on an open one's right; \
             flip the side if that is wrong",
            found.guessed
        ));
    }
    if found.dropped > 0 {
        warnings.push(format!(
            "{} curve(s) turn too tightly for the tool's centre {:.2} mm beside them, and \
             are left out",
            found.dropped, place.offset
        ));
    }
    let mut path = Toolpath::new();
    let mut report = EngraveReport::default();
    for t in in_order(curves) {
        // Each from its edge's height.
        let params = EngraveParams {
            top: t.start.z,
            depth: place.depth,
            stepdown: op.stepdown(),
            ramp_angle: op.linking.ramp_angle.to_radians(),
            feed: op.passes.feed,
            ramp_feed: op.ramp_feed(),
            plunge_feed: op.linking.plunge_feed,
            direction: crate::job::TraceDirection::AsDrawn,
            reverse: false,
            levels: Levels::Ramped,
            climb_right: work.clockwise(),
            tolerance,
        };
        let (one, r) = engrave(std::slice::from_ref(&t), &params).map_err(in_op(op))?;
        for m in one.moves() {
            path.push(*m)
                .map_err(|e| in_op(op)(StrategyError::from(e)))?;
        }
        report.curves += r.curves;
        report.closed += r.closed;
        report.arcs += r.arcs;
        report.passes += r.passes;
        report.plunged += r.plunged;
    }
    Ok((path, report.passes, StrategyDetail::Engrave(report)))
}

/// The V-carving of `area`, no deeper than `max_depth` below the top when
/// given. The tool is a cone: [`Operation::cannot_use`] turns away any other.
pub(super) fn vcarve_paths(
    work: &Work<'_>,
    max_depth: Option<f64>,
    area: &Region,
    warnings: &mut Vec<String>,
) -> Result<Paths, RunError> {
    let (op, spec) = (work.op, work.spec);
    let (top, _) = op.levels_3d(work.heights);
    let params = VCarveParams {
        top,
        half_angle: (spec.angle.unwrap_or(f64::NAN) / 2.0).to_radians(),
        tip_radius: spec.tip_diameter / 2.0,
        radius: spec.diameter / 2.0,
        max_depth,
        stepdown: op.stepdown(),
        feed: op.passes.feed,
        tolerance: work.job.tolerance,
    };
    let (path, report) = vcarve(area, &params).map_err(in_op(op))?;
    // An operation cuts with one tool, and a V cannot cut a floor flat: what
    // is wider than it reaches is left for a flat end mill.
    if report.floor_mm2 > 0.01 {
        warnings.push(format!(
            "{:.1} mm² is wider than the V reaches at {:.2} mm deep, and its floor is \
             left: clear it with a flat end mill, a 2D pocket on the same picks {:.2} mm \
             below the top, leaving {:.2} mm on the walls",
            report.floor_mm2, report.deepest, report.deepest, report.widest
        ));
    }
    Ok((path, report.passes, StrategyDetail::VCarve(report)))
}
