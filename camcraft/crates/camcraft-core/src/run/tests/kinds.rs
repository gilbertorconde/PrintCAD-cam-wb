//! The kinds of tool past the end mills, run end to end: a face mill
//! facing, a tapered ball finishing, a slot cutter grooving under an
//! overhang with its neck watched, a tap and a spot drill drilling.

use super::*;
use crate::job::{FaceDirection, Piece, StockSpec, ToolKind, ToolSpec};
use crate::path::toolpath::MoveRole;
use std::f64::consts::FRAC_1_SQRT_2;

fn cuts(op: &OperationResult) -> Vec<DVec3> {
    op.path
        .moves()
        .iter()
        .filter(|m| m.role == MoveRole::Cut)
        .map(|m| m.to)
        .collect()
}

/// A 45° face mill 20 mm across at its face faces the stock above the plate
/// down to its top, its passes 95% of its flat apart and off the stock only
/// as far as its edge reaches as deep as a level cuts, past its corner and up
/// its lead that far, not by its widest; and nothing of the part. A flat end
/// mill as wide at its face runs off by its radius, the lead less.
#[test]
fn a_face_mill_faces_the_stock_to_the_part() {
    let plate = Body::derived(synthetic::plate_with_hole(20.0, 5.0, 4.0, 48));
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.setups[0].operations.retain(|op| op.name == "rough");
    job.stocks[0].spec = StockSpec::AroundModels {
        models: vec![],
        margin: [10.0, 10.0, 3.0],
        below: 0.0,
    };
    let mill = ToolSpec {
        kind: ToolKind::FaceMill,
        diameter: 20.0,
        corner: 0.8,
        taper_angle: Some(45.0),
        flute_length: Some(4.0),
        ..ToolSpec::default()
    };
    let widest = mill.cutting_diameter() / 2.0;
    assert!(widest > 13.0, "{widest}");
    job.tools.insert("rough".into(), mill.clone());
    let op = &mut job.setups[0].operations[0];
    op.strategy = StrategySpec::Face {
        angle: 0.0,
        direction: FaceDirection::BothWays,
        pass_extension: 1.0,
        stock_offset: 0.0,
    };
    op.passes.stepdown = Some(2.0);
    op.passes.stock_to_leave = Default::default();
    op.passes.stepover = op.default_stepover(&mill, job.tolerance).unwrap();
    assert!((op.passes.stepover - 0.95 * 18.4).abs() < 1e-9);
    assert_eq!(op.cannot_use(&mill), None);
    job.validate().unwrap();
    let bodies: Bodies<'_> = [("dome", &plate)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    let op = &result.operations[0];
    let StrategyDetail::Face(report) = &op.detail else {
        panic!("reported {:?}", op.detail);
    };
    assert_eq!(report.levels, 2);
    let top = result.heights.model_top;
    let cuts = cuts(op);
    let deepest = cuts.iter().map(|p| p.z).fold(f64::MAX, f64::min);
    assert!((deepest - top).abs() < 1e-9, "down to {deepest}, not {top}");
    // 3 mm in two levels of 1.5: the corner's centre is 9.2 out and 0.8 up,
    // the flank leaves it 45° out, and is 1.5 up 1.5 less its foot's height
    // further.
    let foot = 0.8 * (1.0 - FRAC_1_SQRT_2);
    let edge = 9.2 + 0.8 * FRAC_1_SQRT_2 + (1.5 - foot);
    assert!((edge - 11.031).abs() < 1e-3, "{edge}");
    let off = |cuts: &[DVec3]| cuts.iter().map(|p| p.x.abs() - 30.0).collect::<Vec<_>>();
    assert!(
        off(&cuts).iter().all(|d| (d - (edge + 1.0)).abs() < 1e-9),
        "off the stock by {:?}, not its edge {edge} 1.5 up and the extension",
        off(&cuts)
    );
    assert!(edge < widest - 2.0);
    // Rows across the 60 mm, 95% of its flat apart.
    let mut rows: Vec<i64> = cuts.iter().map(|p| (p.y * 1000.0).round() as i64).collect();
    rows.sort_unstable();
    rows.dedup();
    let gaps: Vec<i64> = rows.windows(2).map(|w| w[1] - w[0]).collect();
    assert!(
        gaps.iter().all(|g| *g <= 17_480 + 1),
        "rows {rows:?}, gaps {gaps:?}"
    );
    let taken = op.sim.removed_mm3;
    assert!((taken - 10_800.0).abs() < 400.0, "took {taken}");
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);

    let end_mill = ToolSpec {
        kind: ToolKind::Flat,
        diameter: 20.0,
        ..ToolSpec::default()
    };
    job.tools.insert("rough".into(), end_mill);
    let result = run_setup(&job, 0, &bodies).unwrap();
    let op = &result.operations[0];
    let flat = super::kinds::cuts(op);
    assert!(
        off(&flat).iter().all(|d| (d - 11.0).abs() < 1e-9),
        "off the stock by {:?}, not its radius and the extension",
        off(&flat)
    );
    assert!(
        (op.sim.removed_mm3 - taken).abs() < 1.0,
        "{}",
        op.sim.removed_mm3
    );
}

/// The dome finished with a tapered ball, its ball the plain ball's: where
/// its sides steepen past the taper the flank holds it off, and nothing of
/// the part is gouged anywhere.
#[test]
fn a_tapered_ball_finishes_the_dome_without_a_gouge() {
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.tools.insert(
        "finish".into(),
        ToolSpec {
            kind: ToolKind::Tapered,
            diameter: 4.0,
            corner: 2.0,
            taper_angle: Some(20.0),
            flute_length: Some(12.0),
            number: 2,
            ..ToolSpec::default()
        },
    );
    let tapered = run_job(&job, &dome()).unwrap();
    // Nothing gouged. Beside the sphere, under its equator, the flank hangs
    // the tip far below it, where the stock the roughing could not reach
    // stands: that the tool above its flutes meets it is said, rightly, and
    // is all that is.
    let finish = &tapered.operations[1];
    assert!(
        finish
            .warnings
            .iter()
            .all(|w| w.contains("the cutter above its flutes meets the stock")),
        "{:?}",
        finish.warnings
    );
    assert!(
        finish.clashes.found.iter().all(|c| !c.part),
        "{:?}",
        finish.clashes
    );
    assert_eq!(tapered.verify.gouged, 0, "{:?}", tapered.verify);
    // Never below where the plain ball, its tip's twin, would rest; and
    // where the dome's sides steepen past the taper, above it, held off by
    // its flank.
    let body = Body::derived(dome());
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let prepared = crate::setup::prepare(&job, 0, &bodies).unwrap();
    let dc = DropCutter::new(&prepared.part);
    let plain = crate::cutter::tool::Ball::new(2.0).unwrap();
    let (mut under, mut above) = (0, 0);
    for p in cuts(finish) {
        if let Some(z) = dc.drop_at(&plain, p.x, p.y) {
            under += 1;
            assert!(p.z >= z - 1e-6, "{p} below the ball's {z}");
            above += usize::from(p.z > z + 0.01);
        }
    }
    assert!(
        under > 100 && above > 0,
        "{under} over the part, {above} held off"
    );
}

/// A plate 30 mm thick with a hole 12 mm across, its stock the plate's box,
/// the hole cleared with the 6 mm flat, then a slot cutter 10 mm across on
/// a neck `neck` across, 3 mm tall, cutting a groove 2 mm into the walls
/// 10 mm up, round the hole and the outline in one 2D profile: its head
/// under the walls, its neck beside them.
fn grooved(neck: f64) -> SetupResult {
    let body = Body::derived(synthetic::plate_with_hole(20.0, 30.0, 6.0, 64));
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.setups[0].operations.retain(|op| op.name == "rough");
    job.stocks[0].spec = StockSpec::AroundModels {
        models: vec![],
        margin: [0.0, 0.0, 0.0],
        below: 0.0,
    };
    let clear = &mut job.setups[0].operations[0];
    clear.passes.stock_to_leave = Default::default();
    let mut groove = clear.clone();
    groove.name = "groove".into();
    groove.tool = Some("groove".into());
    job.setups[0].operations.push(groove);
    let cutter = ToolSpec {
        kind: ToolKind::SlotCutter,
        diameter: 10.0,
        neck_diameter: Some(neck),
        flute_length: Some(3.0),
        number: 5,
        ..ToolSpec::default()
    };
    job.tools.insert("groove".into(), cutter.clone());
    let op = &mut job.setups[0].operations[1];
    op.strategy = StrategySpec::Profile {
        side: crate::job::ProfileSide::Outside,
        conventional: false,
    };
    op.passes.stepdown = None;
    op.passes.stock_to_leave = crate::job::StockToLeave {
        radial: -2.0,
        axial: 0.0,
    };
    op.heights.bottom = Some(Height {
        reference: HeightRef::ModelTop,
        offset: -20.0,
    });
    assert_eq!(op.cannot_use(&cutter), None);
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    run_setup(&job, 0, &bodies).unwrap()
}

/// On a neck narrow enough, the groove is cut at its one height and nothing
/// above the head meets the plate; the wall above it is left standing in
/// the material, which the height field cannot undercut.
#[test]
fn a_slot_cutter_grooves_under_the_wall() {
    let result = grooved(4.0);
    let op = &result.operations[1];
    let cuts = cuts(op);
    assert!(!cuts.is_empty());
    assert!(cuts.iter().all(|p| (p.z - 10.0).abs() < 1e-9), "one height");
    // Round the hole, 2 mm past where the head would only touch it.
    let inner = cuts
        .iter()
        .map(|p| p.truncate().length())
        .fold(f64::MAX, f64::min);
    assert!((inner - 3.0).abs() < 0.05, "{inner}");
    assert!(op.clashes.checked);
    assert!(op.clashes.found.is_empty(), "{:?}", op.clashes);
    assert!(op.sim.removed_mm3 < 1.0, "took {}", op.sim.removed_mm3);
    assert_eq!(result.verify.gouged, 0);
}

/// On a neck too wide for the groove, the neck meets the plate above the
/// head, and says so.
#[test]
fn a_slot_cutters_neck_meets_the_wall_above_its_head() {
    let result = grooved(8.0);
    let op = &result.operations[1];
    let neck = op
        .clashes
        .found
        .iter()
        .find(|k| k.piece == Piece::Body)
        .unwrap_or_else(|| panic!("{:?}", op.clashes));
    assert!(neck.part, "{neck:?}");
    assert!(!op.is_clean());
}

/// The plate's hole, drilled with `tool` as `strategy` asks, its rim picked.
fn drilled_with(tool: ToolSpec, strategy: StrategySpec) -> SetupResult {
    use crate::job::FeatureRef;
    use crate::setup::features::find_edge;
    let plate = Body::derived(synthetic::plate_with_hole(20.0, 5.0, 4.0, 48));
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.tools.insert("drill".into(), tool);
    job.setups[0].operations.retain(|op| op.name == "finish");
    let op = &mut job.setups[0].operations[0];
    op.tool = Some("drill".into());
    op.strategy = strategy;
    let pick = FeatureRef {
        model: "dome".into(),
        index: 0,
        at: [4.0, 0.0, 5.0],
    };
    let edge = find_edge(&plate, &pick, 0.01).unwrap().index;
    op.selection.edges.push(FeatureRef {
        index: edge,
        ..pick
    });
    let bodies: Bodies<'_> = [("dome", &plate)].into_iter().collect();
    run_setup(&job, 0, &bodies).unwrap()
}

fn drill(pitch: Option<f64>, spot: Option<f64>) -> StrategySpec {
    StrategySpec::Drill {
        peck: None,
        dwell: None,
        chip_break: None,
        pitch,
        hand: crate::job::ThreadHand::Right,
        spot,
    }
}

/// A spot drill spots the hole to 3 mm across: 1.5 mm below its top with a
/// 90° point, whatever the hole's bottom; and says nothing of its width.
#[test]
fn a_spot_drill_spots_to_a_diameter() {
    let spot = ToolSpec {
        kind: ToolKind::SpotDrill,
        diameter: 6.0,
        ..ToolSpec::default()
    };
    let result = drilled_with(spot, drill(None, Some(3.0)));
    let op = &result.operations[0];
    let deepest = op
        .path
        .moves()
        .iter()
        .map(|m| m.to.z)
        .fold(f64::MAX, f64::min);
    assert!((deepest - (5.0 - 1.5)).abs() < 1e-9, "down to {deepest}");
    assert!(op.warnings.is_empty(), "{:?}", op.warnings);
}

/// Tapping with a tap is quiet; with a flat end mill, as a document from
/// before taps did, it still taps, and says to make the tool a tap.
#[test]
fn a_flat_end_mill_still_taps_and_says_so() {
    let tap = ToolSpec {
        kind: ToolKind::Tap,
        diameter: 8.0,
        pitch: Some(1.25),
        ..ToolSpec::default()
    };
    let with_tap = drilled_with(tap, drill(Some(1.25), None));
    let op = &with_tap.operations[0];
    assert!(
        !op.warnings.iter().any(|w| w.contains("tap")),
        "{:?}",
        op.warnings
    );
    let flat = ToolSpec {
        diameter: 8.0,
        ..ToolSpec::default()
    };
    let with_flat = drilled_with(flat, drill(Some(1.25), None));
    let op = &with_flat.operations[0];
    assert!(
        op.warnings
            .iter()
            .any(|w| w.contains("make its tool a tap")),
        "{:?}",
        op.warnings
    );
    assert_eq!(op.path.moves(), with_tap.operations[0].path.moves());
}

/// A tap is wider than the hole it taps, its thread cut into the wall: that
/// is meant, not a gouge. So is a countersink's seat, wider than its hole.
#[test]
fn a_tap_and_a_countersink_cut_past_their_hole_as_meant() {
    let tap = ToolSpec {
        kind: ToolKind::Tap,
        diameter: 10.0,
        pitch: Some(1.5),
        chamfer_length: Some(3.0),
        ..ToolSpec::default()
    };
    let tapped = drilled_with(tap, drill(Some(1.5), None));
    assert!(tapped.operations[0].sim.removed_mm3 > 50.0);
    assert_eq!(tapped.verify.gouged, 0, "{:?}", tapped.verify);
    let sink = ToolSpec {
        kind: ToolKind::Countersink,
        diameter: 12.0,
        ..ToolSpec::default()
    };
    let sunk = drilled_with(sink, drill(None, Some(11.0)));
    assert!(sunk.operations[0].sim.removed_mm3 > 5.0);
    assert_eq!(sunk.verify.gouged, 0, "{:?}", sunk.verify);
}
