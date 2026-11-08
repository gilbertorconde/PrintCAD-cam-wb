//! A probe grid touched over the stock, and a setup's cuts corrected to a
//! probed surface: run end to end against the material.

use super::*;
use crate::job::{Curve, Segment, ToolKind, ToolSpec, TraceDirection, ZCorrection};
use crate::path::toolpath::{MoveKind, MoveRole};
use crate::probe::ProbeMap;

/// A board warped and tilted: up 0.01 a millimetre along X, and bowed
/// 0.3 mm up in the middle of Y, measured from the work zero.
fn warp(x: f64, y: f64) -> f64 {
    0.01 * x + 0.3 * (1.0 - (y / 20.0).powi(2))
}

/// The warp probed every 10 mm over the plate, 5 x 5 points, in the order a
/// probe goes, back and forth.
fn probed() -> Vec<[f64; 3]> {
    let mut out = Vec::new();
    for (row, y) in [-20.0, -10.0, 0.0, 10.0, 20.0].into_iter().enumerate() {
        let mut xs = vec![-20.0, -10.0, 0.0, 10.0, 20.0];
        if row % 2 == 1 {
            xs.reverse();
        }
        out.extend(xs.into_iter().map(|x| [x, y, warp(x, y)]));
    }
    out
}

/// The plate engraved 0.4 mm deep along a circle and a line across it, the
/// setup corrected to `points`.
fn engraved_on(points: Vec<[f64; 3]>) -> (Job, SetupResult) {
    let line = Curve {
        start: [-15.0, -15.0, 5.0],
        segments: vec![Segment::Line {
            to: [15.0, 12.0, 5.0],
        }],
    };
    let engrave = StrategySpec::Engrave {
        depth: 0.4,
        direction: TraceDirection::Climb,
        reverse: false,
    };
    let curves = vec![drawn_circle(0.0, 0.0, 8.0), line];
    let (mut job, _) = drawn_on_a_plate(engrave, flat(1.0), curves);
    job.setups[0].z_correction = Some(ZCorrection {
        points,
        file: None,
        units: crate::job::LengthUnit::Mm,
        segment: 1.0,
    });
    let body = Body::derived(synthetic::cuboid(
        DVec3::new(-20.0, -20.0, 0.0),
        DVec3::new(20.0, 20.0, 5.0),
    ));
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    (job, result)
}

/// Engraved on a warped, tilted board, every cut stays 0.4 mm below the
/// probed surface, each piece no longer than a millimetre, the arcs cut as
/// straight pieces along them; the stock's top follows the surface, and the
/// setup verifies against it.
#[test]
fn an_engraving_on_a_warped_board_stays_at_its_depth_below_it() {
    let (_, result) = engraved_on(probed());
    let map = result.probe_map.clone().unwrap();
    let op = &result.operations[0];
    assert!(op.path.len() > 80, "{}", op.path.len());
    let mut at = op.path.moves()[0].to;
    let mut cuts = 0;
    for m in op.path.moves() {
        assert_eq!(
            m.kind,
            MoveKind::Linear,
            "an arc left over a warped surface"
        );
        let below = map.height(m.to.truncate()) + 5.0 - m.to.z;
        if m.role == MoveRole::Cut {
            cuts += 1;
            assert!((below - 0.4).abs() < 1e-9, "{below} below at {}", m.to);
            assert!(at.truncate().distance(m.to.truncate()) <= 1.0 + 1e-9);
        }
        // Nothing goes deeper than the engraving anywhere.
        assert!(below <= 0.4 + 1e-9, "{below} below at {}", m.to);
        at = m.to;
    }
    assert!(cuts > 60, "{cuts}");
    // All of it is inside the probed area: nothing is said.
    assert!(op.warnings.is_empty(), "{:?}", op.warnings);
    // The stock's top is the surface, and what was cut is under the path.
    let before = result.keyframes[0].unpack().unwrap();
    for (x, y) in [(-18.0, 3.0), (5.0, -17.0), (0.0, 0.0)] {
        let top = before.height_at(x, y).unwrap();
        assert!((top - (5.0 + warp(x, y))).abs() < 0.02, "{top} at {x} {y}");
    }
    assert!(op.sim.removed_mm3 > 0.0);
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);
}

/// The same uncorrected cuts at a level 0.4 mm below the top: the warp is
/// what moves them.
#[test]
fn without_a_map_the_engraving_is_level() {
    let (job, _) = engraved_on(probed());
    let mut job = job;
    job.setups[0].z_correction = None;
    let body = Body::derived(synthetic::cuboid(
        DVec3::new(-20.0, -20.0, 0.0),
        DVec3::new(20.0, 20.0, 5.0),
    ));
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    assert!(result.probe_map.is_none());
    let cut = result.operations[0]
        .path
        .moves()
        .iter()
        .filter(|m| m.role == MoveRole::Cut)
        .all(|m| (m.to.z - 4.6).abs() < 1e-9);
    assert!(cut);
}

/// One point probed, setting the work zero's height: everything moves by
/// it, and the circle's arcs stay arcs.
#[test]
fn a_single_point_moves_everything_and_keeps_the_arcs() {
    let (_, level) = engraved_on(vec![[0.0, 0.0, -0.25]]);
    let op = &level.operations[0];
    let arcs = op
        .path
        .moves()
        .iter()
        .filter(|m| matches!(m.kind, MoveKind::Arc(_)))
        .count();
    assert!(arcs >= 4, "{arcs}");
    for m in op.path.moves().iter().filter(|m| m.role == MoveRole::Cut) {
        assert!((m.to.z - (4.6 - 0.25)).abs() < 1e-9, "{}", m.to);
    }
    assert!(level.is_clean(), "{:?}", level.warnings());
}

/// A map that covers only part of what is cut is used to its edge, and
/// says how far outside the path goes.
#[test]
fn cuts_outside_the_probed_area_are_said() {
    let small: Vec<[f64; 3]> = [(-5.0, -5.0), (5.0, -5.0), (-5.0, 5.0), (5.0, 5.0)]
        .into_iter()
        .map(|(x, y)| [x, y, 0.1])
        .collect();
    let (_, result) = engraved_on(small);
    let op = &result.operations[0];
    assert!(
        op.warnings
            .iter()
            .any(|w| w.contains("outside the probed area")),
        "{:?}",
        op.warnings
    );
}

/// Probe results that are no grid stop the run before it starts, saying so.
#[test]
fn probe_results_that_are_no_grid_are_refused() {
    let (mut job, _) = engraved_on(probed());
    if let Some(z) = &mut job.setups[0].z_correction {
        z.points.pop();
    }
    let e = job.check_setup(0).unwrap_err();
    assert!(e.to_string().contains("not a grid"), "{e}");
    if let Some(z) = &mut job.setups[0].z_correction {
        z.points.clear();
        z.file = Some("probe.txt".into());
    }
    let e = job.check_setup(0).unwrap_err();
    assert!(e.to_string().contains("never read from probe.txt"), "{e}");
}

fn probe_tool() -> ToolSpec {
    ToolSpec {
        kind: ToolKind::Probe,
        diameter: 2.0,
        number: 9,
        ..ToolSpec::default()
    }
}

/// A probe over the plate: a grid 2 mm in from its edges, no more than
/// 10 mm apart, each point touched from the retract height at the top at
/// the probe's feed. Nothing is cut, no spindle turns, and a probe cuts
/// nothing else.
#[test]
fn a_probe_touches_a_grid_over_the_stock_and_cuts_nothing() {
    let probe = StrategySpec::named("probe").unwrap();
    let (job, result) = drawn_on_a_plate(probe, probe_tool(), Vec::new());
    let op = &result.operations[0];
    let StrategyDetail::Probe(r) = op.detail else {
        panic!("reported {:?}", op.detail);
    };
    assert_eq!((r.columns, r.rows), (5, 5));
    assert_eq!(r.max_depth, 3.0);
    assert_eq!(op.rpm, 0);
    let touches: Vec<_> = op
        .path
        .moves()
        .iter()
        .filter(|m| m.role == MoveRole::Plunge)
        .collect();
    assert_eq!(touches.len(), 25);
    for m in &touches {
        assert_eq!(m.to.z, 5.0);
        assert_eq!(m.feed, Some(50.0));
        assert!(m.to.x.abs() <= 18.0 + 1e-9 && m.to.y.abs() <= 18.0 + 1e-9);
    }
    assert!(touches.iter().any(|m| (m.to.x + 18.0).abs() < 1e-9));
    assert!(touches.iter().any(|m| (m.to.y - 18.0).abs() < 1e-9));
    assert!(Arc::ptr_eq(&result.keyframes[0], &result.keyframes[1]));
    assert!(result.is_clean(), "{:?}", result.warnings());

    // Counted, and a probe that does not fit.
    let mut counted = probe;
    if let StrategySpec::Probe {
        x_count, y_count, ..
    } = &mut counted
    {
        (*x_count, *y_count) = (Some(3), Some(1));
    }
    let (_, result) = drawn_on_a_plate(counted, probe_tool(), Vec::new());
    let StrategyDetail::Probe(r) = result.operations[0].detail else {
        panic!();
    };
    assert_eq!((r.columns, r.rows), (3, 1));
    let tool = &job.tools["drawn"];
    let mut engrave = job.setups[0].operations[0].clone();
    engrave.strategy = StrategySpec::named("engrave").unwrap();
    assert!(engrave.cannot_use(tool).is_some());
    let mut probing = engrave.clone();
    probing.strategy = probe;
    assert!(probing.cannot_use(&flat(1.0)).is_some());
    assert!(probing.cannot_use(tool).is_none());
}

/// Probed and engraved in one corrected setup: the probe is not corrected
/// to what it probes, the engraving is.
#[test]
fn a_probe_in_a_corrected_setup_is_left_as_it_is() {
    let (mut job, _) = engraved_on(probed());
    let mut probe = job.setups[0].operations[0].clone();
    probe.name = "probe".into();
    probe.strategy = StrategySpec::named("probe").unwrap();
    probe.tool = Some("probe".into());
    job.tools.insert("probe".into(), probe_tool());
    job.setups[0].operations.insert(0, probe);
    let body = Body::derived(synthetic::cuboid(
        DVec3::new(-20.0, -20.0, 0.0),
        DVec3::new(20.0, 20.0, 5.0),
    ));
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    let map: &ProbeMap = result.probe_map.as_ref().unwrap();
    let touches = result.operations[0]
        .path
        .moves()
        .iter()
        .filter(|m| m.role == MoveRole::Plunge);
    for m in touches {
        assert_eq!(m.to.z, 5.0);
    }
    let engraving = &result.operations[1];
    for m in engraving
        .path
        .moves()
        .iter()
        .filter(|m| m.role == MoveRole::Cut)
    {
        assert!((map.height(m.to.truncate()) + 4.6 - m.to.z).abs() < 1e-9);
    }
    assert!(result.is_clean(), "{:?}", result.warnings());
}
