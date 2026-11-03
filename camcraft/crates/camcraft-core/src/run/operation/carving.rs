//! The paths of the strategies that follow what was drawn: engraving along
//! curves, and V-carving down the middle of areas.

use super::*;

/// The engraving of `traces`, `depth` below the top, each run the way
/// `direction` and `reverse` say.
pub(super) fn engrave_paths(
    work: &Work<'_>,
    (depth, direction, reverse): (f64, crate::job::TraceDirection, bool),
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
