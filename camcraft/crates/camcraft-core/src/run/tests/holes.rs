//! Holes bored, milled in circles and threaded, and found by their
//! diameter, run end to end against the material.

use super::*;
use crate::job::{FeatureRef, HoleOrder, ToolSpec};
use crate::path::toolpath::{MoveKind, MoveRole};
use crate::setup::features::find_face;
use camcraft_geom::mesh::TriMesh;
use glam::dvec3;

/// A closed rod `r` round and `h` tall standing on the origin, wound out.
fn rod(r: f64, h: f64, n: u32) -> TriMesh {
    let mut v = vec![dvec3(0.0, 0.0, 0.0), dvec3(0.0, 0.0, h)];
    for z in [0.0, h] {
        for i in 0..n {
            let a = std::f64::consts::TAU * f64::from(i) / f64::from(n);
            v.push(dvec3(r * a.cos(), r * a.sin(), z));
        }
    }
    let (b, t) = (|i: u32| 2 + i % n, |i: u32| 2 + n + i % n);
    let mut tris = Vec::new();
    for i in 0..n {
        tris.extend([
            [0, b(i + 1), b(i)],
            [1, t(i), t(i + 1)],
            [b(i), b(i + 1), t(i + 1)],
            [b(i), t(i + 1), t(i)],
        ]);
    }
    TriMesh::new(v, tris).unwrap()
}

/// The job's one operation, `strategy` with tool `tool`, picking the face
/// through `at` of `body`.
fn job_on(body: &Body, strategy: &str, tool: &str, at: [f64; 3]) -> Job {
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.tools
        .insert("cutter".into(), toml::from_str::<ToolSpec>(tool).unwrap());
    job.setups[0].operations.retain(|op| op.name == "finish");
    let op = &mut job.setups[0].operations[0];
    op.name = "holes".into();
    op.tool = Some("cutter".into());
    op.strategy = toml::from_str(strategy).unwrap();
    op.passes.stepover = 2.0;
    op.passes.stepdown = Some(2.0);
    op.passes.feed = 800.0;
    let pick = FeatureRef {
        model: "dome".into(),
        index: 0,
        at,
    };
    let face = find_face(body, &pick, 0.01).unwrap().index;
    op.selection.faces.push(FeatureRef {
        index: face,
        ..pick
    });
    job
}

fn run_on(job: &Job, body: &Body) -> Result<SetupResult, RunError> {
    let bodies: Bodies<'_> = [("dome", body)].into_iter().collect();
    run_setup(job, 0, &bodies)
}

const FLAT: &str = "kind = \"flat\"\ndiameter = 6.0\nnumber = 3\n";

/// The plate's 8 mm hole, picked by its wall, bored with a 6 mm end mill:
/// from its top to its bottom, the whole cylinder of material in it taken,
/// none of the part, and every move round it an arc.
#[test]
fn a_hole_picked_by_its_wall_is_bored() {
    let body = Body::derived(synthetic::plate_with_hole(20.0, 5.0, 4.0, 64));
    let job = job_on(&body, "strategy = \"bore\"", FLAT, [4.0, 0.0, 2.5]);
    let result = run_on(&job, &body).unwrap();
    let op = &result.operations[0];
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);
    let StrategyDetail::Holes(report) = &op.detail else {
        panic!("reported {:?}", op.detail);
    };
    assert_eq!(report.rounds.len(), 1);
    let hole = report.rounds[0].hole;
    assert!(!report.rounds[0].boss);
    assert!(
        (hole.top - 5.0).abs() < 1e-6 && hole.bottom.abs() < 1e-6,
        "{hole:?}"
    );
    assert!((hole.radius - 4.0).abs() < 1e-3);
    // The material in the hole, from the stock's top down to its bottom.
    let cylinder = std::f64::consts::PI * 16.0 * result.heights.stock_top;
    let removed = op.sim.removed_mm3;
    assert!(
        (removed - cylinder).abs() < 0.05 * cylinder,
        "{removed} of {cylinder}"
    );
    assert!(
        op.path
            .moves()
            .iter()
            .filter(|m| m.role == MoveRole::Cut)
            .all(|m| matches!(m.kind, MoveKind::Arc(_)))
    );
}

/// Milled in circles, the same hole is taken as boring takes it; and
/// threaded with a thread mill, cut past its wall to the thread's major
/// diameter, which is meant, not a gouge.
#[test]
fn a_hole_is_milled_in_circles_and_threaded() {
    let body = Body::derived(synthetic::plate_with_hole(20.0, 5.0, 4.0, 64));
    let job = job_on(&body, "strategy = \"circular\"", FLAT, [4.0, 0.0, 2.5]);
    let result = run_on(&job, &body).unwrap();
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);
    let cylinder = std::f64::consts::PI * 16.0 * result.heights.stock_top;
    let removed = result.operations[0].sim.removed_mm3;
    assert!((removed - cylinder).abs() < 0.05 * cylinder, "{removed}");

    let mill = "kind = \"thread-mill\"\ndiameter = 6.0\nnumber = 4\ntip_length = 0.5\n\
                pitch_range = [1.0, 1.5]\n";
    let job = job_on(
        &body,
        "strategy = \"thread\"\npitch = 1.25",
        mill,
        [4.0, 0.0, 2.5],
    );
    let result = run_on(&job, &body).unwrap();
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0, "the thread is meant");
    let op = &result.operations[0];
    assert!(op.sim.removed_mm3 > 0.0, "past the wall");
    let lowest = op
        .path
        .moves()
        .iter()
        .map(|m| m.to.z)
        .fold(f64::MAX, f64::min);
    assert!(lowest >= -1e-9, "no lower than the hole's bottom: {lowest}");

    // A pitch the mill cannot cut, and an end mill threading, are refused.
    let job = job_on(
        &body,
        "strategy = \"thread\"\npitch = 2.0",
        mill,
        [4.0, 0.0, 2.5],
    );
    let op = &job.setups[0].operations[0];
    assert!(
        op.cannot_use(&job.tools["cutter"])
            .unwrap()
            .contains("pitch")
    );
    let job = job_on(&body, "strategy = \"thread\"", FLAT, [4.0, 0.0, 2.5]);
    let op = &job.setups[0].operations[0];
    assert!(
        op.cannot_use(&job.tools["cutter"])
            .unwrap()
            .contains("thread mill")
    );
}

/// A rod's side, picked, is a boss: milled round from outside, the way that
/// climbs, and refused for drilling.
#[test]
fn a_boss_is_milled_round_and_not_drilled() {
    let body = Body::derived(rod(6.0, 5.0, 64));
    let job = job_on(
        &body,
        "strategy = \"circular\"\nboss_width = 2.0",
        FLAT,
        [6.0, 0.0, 2.5],
    );
    let result = run_on(&job, &body).unwrap();
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);
    let op = &result.operations[0];
    let StrategyDetail::Holes(report) = &op.detail else {
        panic!("reported {:?}", op.detail);
    };
    assert!(report.rounds[0].boss);
    // Clockwise round it, seen from above: climbing outside.
    for m in op.path.moves() {
        if let (MoveRole::Cut, MoveKind::Arc(a)) = (m.role, m.kind) {
            assert!(a.sweep < 0.0);
            assert!(m.to.truncate().length() >= 9.0 - 1e-9);
        }
    }
    let drill = job_on(
        &body,
        "strategy = \"drill\"",
        "kind = \"drill\"\ndiameter = 6.0\nnumber = 3\n",
        [6.0, 0.0, 2.5],
    );
    let err = run_on(&drill, &body).unwrap_err();
    assert!(err.to_string().contains("boss"), "{err}");
}

/// Three plates, two with 8 mm holes and one with a 6 mm hole: one wall
/// picked with the same diameter takes both 8 mm holes, and not the 6 mm
/// one, and they are taken in the order asked.
#[test]
fn the_same_diameter_finds_every_hole_as_wide() {
    let plate = |r: f64, at: DVec3| {
        synthetic::plate_with_hole(10.0, 5.0, r, 64)
            .mapped(|p| p + at)
            .unwrap()
    };
    let (a, b, c) = (
        plate(4.0, dvec3(30.0, 0.0, 0.0)),
        plate(4.0, dvec3(0.0, 0.0, 0.0)),
        plate(3.0, dvec3(0.0, 30.0, 0.0)),
    );
    let body = Body::derived(TriMesh::merged(&[&a, &b, &c]).unwrap());
    let drill = "kind = \"drill\"\ndiameter = 8.0\nnumber = 3\n";
    let mut job = job_on(&body, "strategy = \"drill\"", drill, [34.0, 0.0, 2.5]);
    let found = |job: &Job| {
        let result = run_on(job, &body).unwrap();
        assert!(result.is_clean(), "{:?}", result.warnings());
        let StrategyDetail::Drill(r) = &result.operations[0].detail else {
            panic!();
        };
        r.holes.iter().map(|h| h.at.x.round()).collect::<Vec<_>>()
    };
    assert_eq!(found(&job), [30.0]);
    job.setups[0].operations[0].holes.same_diameter = true;
    assert_eq!(
        found(&job),
        [30.0, 0.0],
        "the picked first, then the nearest"
    );
    job.setups[0].operations[0].holes.order = HoleOrder::XThenY;
    assert_eq!(found(&job), [0.0, 30.0]);
}
