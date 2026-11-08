//! A setup on a rotary: planned on its bar unrolled, simulated round it.

use super::*;
use crate::job::{BarAxis, Curve, Segment, StockSpec, TraceDirection, Wrap};
use crate::path::toolpath::{MoveKind, MoveRole};
use camcraft_geom::mesh::TriMesh;

/// A closed cylinder of radius `r` along X from 0 to `length`, `n` sides
/// round.
pub(super) fn rod(r: f64, length: f64, n: u32) -> TriMesh {
    let mut v = Vec::new();
    for x in [0.0, length] {
        for i in 0..n {
            let a = std::f64::consts::TAU * f64::from(i) / f64::from(n);
            v.push(DVec3::new(x, r * a.sin(), r * a.cos()));
        }
    }
    v.push(DVec3::new(0.0, 0.0, 0.0));
    v.push(DVec3::new(length, 0.0, 0.0));
    let (c0, c1) = (2 * n, 2 * n + 1);
    let mut t = Vec::new();
    for i in 0..n {
        let j = (i + 1) % n;
        t.push([i, n + i, j]);
        t.push([j, n + i, n + j]);
        t.push([c0, j, i]);
        t.push([c1, n + i, n + j]);
    }
    TriMesh::new(v, t).unwrap()
}

/// A rod `rod_radius` round and 60 long, in a bar round it on a rotary, with one
/// operation of `strategy` with `tool`, following `curves`.
pub(super) fn on_a_rotary(
    rod_radius: f64,
    strategy: StrategySpec,
    tool: crate::job::ToolSpec,
    curves: Vec<Curve>,
    edit: impl FnOnce(&mut Job),
) -> (Job, Result<SetupResult, RunError>) {
    let body = Body::derived(rod(rod_radius, 60.0, 96));
    let mut job: Job = toml::from_str(crate::fixture::JOB).unwrap();
    job.setups[0].operations.retain(|op| op.name == "rough");
    job.stocks[0].spec = StockSpec::CylinderAroundModels {
        models: vec![],
        axis: BarAxis::X,
        margin: 0.0,
        ends: 0.0,
    };
    job.setups[0].wrap = Some(Wrap::default());
    job.tools.insert("drawn".into(), tool);
    let op = &mut job.setups[0].operations[0];
    op.strategy = strategy;
    op.tool = Some("drawn".into());
    op.passes.stepdown = strategy.default_stepdown();
    op.passes.stock_to_leave = Default::default();
    op.selection.curves = curves;
    edit(&mut job);
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies);
    (job, result)
}

/// A line all the way round the bar at X 20, drawn flat over its top (Y
/// is round it), and a circle at X 40.
fn drawn() -> Vec<Curve> {
    let half = std::f64::consts::PI * 20.0;
    let round = Curve {
        start: [20.0, -half + 1.0, 20.0],
        segments: vec![Segment::Line {
            to: [20.0, half - 1.0, 20.0],
        }],
    };
    vec![round, super::drawn_circle(40.0, 30.0, 5.0)]
}

/// Engraved along a line round the bar and a circle flat on it: the paths
/// are straight moves, half a millimetre in from the bar's surface; the bar
/// is cut round where the line goes, far past where a flat drawing over
/// the top would reach; and the run verifies against the part.
#[test]
fn an_engraving_is_wrapped_round_the_bar() {
    let engrave = StrategySpec::Engrave {
        depth: 0.5,
        direction: TraceDirection::Climb,
        reverse: false,
    };
    let (_, result) = on_a_rotary(20.0, engrave, super::flat(1.0), drawn(), |_| {});
    let result = result.unwrap();
    let w = result.frame.wrap.unwrap();
    assert!((w.radius - 20.0).abs() < 1e-6, "{w:?}");
    let op = &result.operations[0];
    assert!(
        op.path
            .moves()
            .iter()
            .all(|m| matches!(m.kind, MoveKind::Linear))
    );
    let deepest = op
        .path
        .moves()
        .iter()
        .filter(|m| m.role == MoveRole::Cut)
        .map(|m| m.to.z)
        .fold(f64::MAX, f64::min);
    assert!((deepest - 19.5).abs() < 1e-6, "{deepest}");
    assert!(op.sim.is_clean(), "{:?}", op.sim);
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);

    // Round under the line, the bar is cut to 19.5 the whole way round, the
    // bottom too; beside it, it is whole.
    let after = result.keyframes[1].unpack().unwrap();
    assert!(after.round().is_some());
    for s in [-50.0, -20.0, 0.0, 35.0, 55.0] {
        let h = after.height_at(20.0, s).unwrap();
        assert!((h - 19.5).abs() < 0.02, "at {s}: {h}");
        let beside = after.height_at(26.0, s).unwrap();
        assert!((beside - 20.0).abs() < 0.02, "beside, at {s}: {beside}");
    }
    // Inside the circle, untouched; on it, cut.
    assert!((after.height_at(40.0, 30.0).unwrap() - 20.0).abs() < 0.02);
    assert!((after.height_at(45.0, 30.0).unwrap() - 19.5).abs() < 0.02);
}

/// An operation planned flat on the part, a pocket clearing say, or picks
/// on the part, are refused on a rotary, as is a stock that is not a bar.
#[test]
fn what_is_not_wrapped_is_refused() {
    let engrave = StrategySpec::Engrave {
        depth: 0.5,
        direction: TraceDirection::Climb,
        reverse: false,
    };
    let (_, r) = on_a_rotary(
        20.0,
        StrategySpec::Face {
            angle: 0.0,
            direction: Default::default(),
            pass_extension: 0.0,
            stock_offset: 0.0,
        },
        super::flat(6.0),
        vec![],
        |_| {},
    );
    let e = r.unwrap_err().to_string();
    assert!(e.contains("not planned on a rotary"), "{e}");
    let (_, r) = on_a_rotary(20.0, engrave, super::flat(1.0), drawn(), |job| {
        job.stocks[0].spec = StockSpec::AroundModels {
            models: vec![],
            margin: [0.0; 3],
            below: 0.0,
        };
    });
    let e = r.unwrap_err().to_string();
    assert!(e.contains("one cylinder"), "{e}");
}

/// A rod 30 across in a bar 40 across, roughed and finished on a rotary by
/// a flat end mill in parallel passes along it (angle 0) or one helix round
/// it (90): a level down from the bar, then on the rod. Every column the
/// passes went over is the rod's to within the tolerance, none below it;
/// the helix goes round many turns.
#[test]
fn a_rotary_finish_takes_a_bar_down_to_the_rod() {
    use crate::job::FixedCylinder;
    for angle in [0.0, 90.0] {
        let (job, result) = on_a_rotary(
            15.0,
            StrategySpec::Parallel { angle },
            super::flat(6.0),
            vec![],
            |job| {
                job.stocks[0].spec = StockSpec::FixedCylinder(FixedCylinder {
                    models: vec![],
                    axis: BarAxis::X,
                    diameter: 40.0,
                    length: 60.0,
                    place: crate::job::Place::Center {},
                });
                let op = &mut job.setups[0].operations[0];
                op.passes.stepover = 2.5;
                op.passes.stepdown = Some(3.0);
                op.passes.feed = 1000.0;
                op.heights.clearance = Height {
                    reference: HeightRef::StockTop,
                    offset: 5.0,
                };
            },
        );
        let result = result.unwrap();
        let op = &result.operations[0];
        assert!(op.sim.is_clean(), "{angle}: {:?}", op.sim);
        assert!(result.is_clean(), "{angle}: {:?}", result.warnings());
        assert_eq!(result.verify.gouged, 0, "{angle}");
        let lowest = op
            .path
            .moves()
            .iter()
            .filter(|m| m.role == MoveRole::Cut)
            .map(|m| m.to.z)
            .fold(f64::MAX, f64::min);
        assert!((lowest - 15.0).abs() < job.tolerance, "{angle}: {lowest}");
        // Round and round, past a whole turn, for the helix.
        let w = result.frame.wrap.unwrap();
        let (lo, hi) = op
            .path
            .moves()
            .iter()
            .fold((f64::MAX, f64::MIN), |(a, b), m| {
                (a.min(m.to.y), b.max(m.to.y))
            });
        if angle > 45.0 {
            assert!(hi - lo > 10.0 * w.period(), "{angle}: {lo} to {hi}");
        } else {
            assert!(hi - lo < w.period(), "{angle}: {lo} to {hi}");
        }
        // Along the rod, clear of its ends, the bar is the rod all round.
        let after = result.keyframes[1].unpack().unwrap();
        let f = after.field();
        let mut worst: f64 = 0.0;
        for iy in 0..f.ny() {
            for ix in 0..f.nx() {
                let p = f.position(ix, iy).unwrap();
                if !(5.0..55.0).contains(&p.x) {
                    continue;
                }
                let h = f.get(ix, iy).unwrap();
                assert!(h > 15.0 - 0.05, "{angle}: {h} at {p}");
                worst = worst.max(h - 15.0);
            }
        }
        assert!(worst < 0.05, "{angle}: {worst} left");
    }
}

/// A tapered ball finishes the rod on a rotary as a ball does, its profile
/// met radially, down to the rod and nowhere below it; a slot cutter is
/// refused, as its head would reach in under the bar round it.
#[test]
fn a_tapered_ball_finishes_on_a_rotary_and_a_slot_cutter_does_not() {
    use crate::job::{FixedCylinder, ToolKind, ToolSpec};
    let tapered = ToolSpec {
        kind: ToolKind::Tapered,
        diameter: 4.0,
        corner: 2.0,
        taper_angle: Some(5.0),
        flute_length: Some(10.0),
        ..ToolSpec::default()
    };
    let (job, result) = on_a_rotary(
        15.0,
        StrategySpec::Parallel { angle: 0.0 },
        tapered,
        vec![],
        |job| {
            job.stocks[0].spec = StockSpec::FixedCylinder(FixedCylinder {
                models: vec![],
                axis: BarAxis::X,
                diameter: 34.0,
                length: 60.0,
                place: crate::job::Place::Center {},
            });
            let op = &mut job.setups[0].operations[0];
            op.passes.stepover = 1.5;
            op.passes.feed = 1000.0;
            op.heights.clearance = Height {
                reference: HeightRef::StockTop,
                offset: 5.0,
            };
        },
    );
    let result = result.unwrap();
    assert_eq!(result.verify.gouged, 0);
    let lowest = result.operations[0]
        .path
        .moves()
        .iter()
        .filter(|m| m.role == MoveRole::Cut)
        .map(|m| m.to.z)
        .fold(f64::MAX, f64::min);
    assert!((lowest - 15.0).abs() < job.tolerance, "{lowest}");
    let slot = ToolSpec {
        kind: ToolKind::SlotCutter,
        diameter: 10.0,
        neck_diameter: Some(4.0),
        flute_length: Some(3.0),
        ..ToolSpec::default()
    };
    let profile = StrategySpec::Profile {
        side: crate::job::ProfileSide::On,
        conventional: false,
    };
    let (_, r) = on_a_rotary(20.0, profile, slot, drawn(), |job| {
        job.setups[0].operations[0].passes.stepdown = None;
    });
    let e = r.unwrap_err().to_string();
    assert!(e.contains("lollipop or slot cutter"), "{e}");
}
