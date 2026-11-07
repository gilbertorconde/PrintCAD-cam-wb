use super::pitch::{CELLS_PER_TOOL, FINEST_PITCH, MAX_SIM_CELLS};
use super::*;
use crate::fixture::JOB;
use crate::job::{Boundary, Height, HeightRef, Outline, StrategySpec};
use crate::setup::PrepareError;
use camcraft_geom::synthetic;
use camcraft_geom::topology::Body;
use glam::{DVec2, DVec3, dvec2};

mod holes;

/// A small stock is simulated finely, a large one fine enough for its
/// narrowest tool, and nothing beyond the memory cap.
#[test]
fn the_pitch_follows_the_stock_and_its_tools() {
    let inch = 25.4;
    let sheet = dvec2(5.0 * 12.0 * inch, 12.0 * 12.0 * inch);
    // The lego brick's block: about 0.053 mm, two million cells, whatever the tool.
    let lego = sim_pitch(0.05, dvec2(104.6, 53.8), Some(inch / 4.0));
    assert!((lego - 0.053).abs() < 0.001, "{lego}");
    // A tiny part stops at the finest pitch.
    assert_eq!(sim_pitch(0.05, dvec2(10.0, 10.0), None), FINEST_PITCH);
    // A full sheet with a quarter-inch cutter: eight tolerances.
    assert_eq!(sim_pitch(0.05, sheet, Some(inch / 4.0)), 0.4);
    // With a sixteenth, as fine as memory allows.
    let fine = sim_pitch(0.05, sheet, Some(inch / 16.0));
    assert!(fine < 0.4 && fine > inch / 16.0 / CELLS_PER_TOOL, "{fine}");
    assert!(((sheet.x * sheet.y) / (fine * fine) - MAX_SIM_CELLS).abs() < 1.0);
    // A thirty-second on a smaller stock gets its eight cells across.
    let small = sim_pitch(0.05, dvec2(300.0, 300.0), Some(inch / 32.0));
    assert!(
        (small - inch / 32.0 / CELLS_PER_TOOL).abs() < 1e-12,
        "{small}"
    );
    // Nothing sensible to go on: eight tolerances.
    assert_eq!(sim_pitch(0.05, dvec2(f64::NAN, 1.0), None), 0.4);
    for (size, tool) in [(sheet, Some(1.0)), (dvec2(900.0, 900.0), Some(0.5))] {
        let p = sim_pitch(0.05, size, tool);
        assert!(size.x * size.y / (p * p) <= MAX_SIM_CELLS * 1.0001);
    }
}

fn dome() -> camcraft_geom::mesh::TriMesh {
    synthetic::uv_sphere(8.0, 100, 140)
}

fn run_job(job: &Job, mesh: &camcraft_geom::mesh::TriMesh) -> Result<SetupResult, RunError> {
    let body = Body::derived(mesh.clone());
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    run_setup(job, 0, &bodies)
}

#[test]
fn runs_both_operations_in_order() {
    let job: Job = toml::from_str(JOB).unwrap();
    let result = run_job(&job, &dome()).unwrap();

    assert_eq!(result.operations.len(), 2);
    let names: Vec<&str> = result.operations.iter().map(|o| o.name.as_str()).collect();
    assert_eq!(names, vec!["rough", "finish"], "order must be preserved");
    for o in &result.operations {
        assert!(
            !o.path.has_unplanned_links(),
            "{} is not machinable",
            o.name
        );
        assert!(o.passes > 0, "{} produced no passes", o.name);
    }
}

/// Whatever looks at a result sees the run's own reports, not a recomputation.
/// A plunge at cutting feed is refused like a rapid through material, not
/// merely warned about. The strategy breaks passes before it gets here; this is
/// the backstop, so it is tested directly.
#[test]
fn a_descent_past_the_allowance_fails_the_operation() {
    let job: Job = toml::from_str(JOB).unwrap();
    let result = run_job(&job, &dome()).unwrap();
    let mut finish = result.operations[1].clone();
    assert!(finish.is_clean(), "{:?}", finish.warnings);
    finish.deepest_cut_descent_mm = finish.max_cut_step + 0.01;
    assert!(!finish.is_clean());
}

#[test]
fn results_carry_what_the_run_reported() {
    let job: Job = toml::from_str(JOB).unwrap();
    let result = run_job(&job, &dome()).unwrap();
    let [rough, finish] = result.operations.as_slice() else {
        panic!("expected two operations");
    };

    let StrategyDetail::PocketClearing(r) = rough.detail else {
        panic!("rough reported {:?}", rough.detail);
    };
    assert_eq!(r.rings, rough.passes);
    let StrategyDetail::Parallel(p) = finish.detail else {
        panic!("finish reported {:?}", finish.detail);
    };
    assert_eq!(p.passes, finish.passes);

    // Clearance resolved against this job's block: stock-top plus 6.
    assert!((result.heights.stock_top - (8.0 + 2.0)).abs() < 1e-6);
    for o in &result.operations {
        assert!((o.clearance_z - (result.heights.stock_top + 6.0)).abs() < 1e-9);
        assert!(o.links.links_resolved > 0, "{} resolved no links", o.name);
    }
    assert!(result.stock_min.x < -8.0 && result.stock_max.x > 8.0);
}

/// The point of threading: the second operation sees what the first removed.
#[test]
fn roughing_removes_material_before_finishing_runs() {
    let job: Job = toml::from_str(JOB).unwrap();
    let result = run_job(&job, &dome()).unwrap();
    let rough = result.operations.first().unwrap();
    assert!(
        rough.sim.removed_mm3 > 0.0,
        "roughing should remove material"
    );
}

/// Every operation is simulated as it runs, and nothing may traverse material.
#[test]
fn no_operation_rapids_through_stock() {
    let job: Job = toml::from_str(JOB).unwrap();
    let result = run_job(&job, &dome()).unwrap();
    for o in &result.operations {
        assert!(
            o.sim.is_clean(),
            "{} put {} rapid(s) through material, deepest {:.3} mm",
            o.name,
            o.sim.rapid_contacts,
            o.sim.deepest_rapid_contact_mm
        );
    }
}

/// The whole job must land on the part.
#[test]
fn the_finished_result_is_the_part() {
    let job: Job = toml::from_str(JOB).unwrap();
    let result = run_job(&job, &dome()).unwrap();
    assert!(result.verify.compared > 500);
    assert!(
        result.is_clean(),
        "job did not verify: {:?}",
        result.warnings()
    );
}

/// A V-bit finishes the dome after roughing: its path runs, sweeps through the
/// simulator as a cone, and the result verifies against the part.
#[test]
fn a_vbit_finishes_and_verifies() {
    let vbit = JOB.replace(
        "[tools.finish]\nkind = \"ball\"\ndiameter = 4.0\n",
        "[tools.finish]\nkind = \"cone\"\ndiameter = 6.0\nangle = 90.0\n",
    );
    assert_ne!(vbit, JOB, "the edit must land");
    let job: Job = toml::from_str(&vbit).unwrap();
    let result = run_job(&job, &dome()).unwrap();
    let finish = &result.operations[1];
    assert!(finish.passes > 0);
    assert!(finish.sim.is_clean(), "{:?}", finish.warnings);
    assert!(
        result.is_clean(),
        "job did not verify: {:?}",
        result.warnings()
    );
}

/// A validation failure must stop the job before anything is computed.
#[test]
fn an_invalid_job_never_starts() {
    let bad = JOB.replace("stepdown = 2.5", "stepdown = 0.0");
    let job: Job = toml::from_str(&bad).unwrap();
    assert!(matches!(run_job(&job, &dome()), Err(RunError::Job(_))));
}

#[test]
fn an_empty_model_is_refused() {
    let job: Job = toml::from_str(JOB).unwrap();
    let empty = camcraft_geom::mesh::TriMesh::default();
    assert!(matches!(
        run_job(&job, &empty),
        Err(RunError::Prepare(PrepareError::EmptyModel(_)))
    ));
}

/// Turned over, the setup machines the other side: a dome standing 11 mm up
/// and hanging 5 mm down shows its 5 mm side on top, and still verifies.
#[test]
fn a_flipped_setup_machines_the_underside() {
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.setups[0].orientation = crate::setup::frame::Orientation::default().flipped();
    let dome = dome().mapped(|p| p + glam::dvec3(0.0, 0.0, 3.0)).unwrap();
    let result = run_job(&job, &dome).unwrap();
    assert!((result.heights.model_top - 5.0).abs() < 1e-6);
    assert!((result.heights.model_bottom + 11.0).abs() < 1e-6);
    assert!(result.is_clean(), "{:?}", result.warnings());
}

/// Tilted 30° about X, the setup machines the dome as it leans, from a box
/// round it as tilted: the same dome, so the same heights, and it verifies
/// clean, in a frame that is a rotation and not a swap of axes.
#[test]
fn a_tilted_setup_machines_and_verifies() {
    use crate::setup::frame::{Axis, Direction, Orientation};
    let mut job: Job = toml::from_str(JOB).unwrap();
    let (s, c) = 30f64.to_radians().sin_cos();
    job.setups[0].orientation = Orientation {
        z: Direction::Vector([0.0, -s, c]),
        x: Direction::Axis(Axis::PosX),
    };
    let result = run_job(&job, &dome()).unwrap();
    assert!(result.frame.turn.square().is_none());
    assert!(
        (result.heights.model_top - 8.0).abs() < 0.05,
        "{:?}",
        result.heights
    );
    let tip = result
        .frame
        .to_model(glam::dvec3(0.0, 0.0, result.heights.model_top));
    let up = result.frame.turn.tool_axis();
    assert!((up - glam::dvec3(0.0, -s, c)).length() < 1e-12);
    assert!((tip.dot(up) - 8.0).abs() < 0.05, "{tip}");
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert!(result.operations.iter().all(|o| o.passes > 0));
}

/// Two setups start from their own stock: the second's first stage is a whole
/// block, not what the first left.
#[test]
fn each_setup_starts_from_its_own_stock() {
    let mut job: Job = toml::from_str(JOB).unwrap();
    let mut second = job.setups[0].clone();
    second.name = "again".into();
    job.setups.push(second);
    let body = Body::derived(dome());
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let first = run_setup(&job, 0, &bodies).unwrap();
    let again = run_setup(&job, 1, &bodies).unwrap();
    assert_eq!(first.keyframes[0], again.keyframes[0]);
    assert_eq!(again.setup, "again");
}

/// Two blocks side by side make one stock: the taller one sets stock-top, and
/// both are cut.
#[test]
fn two_blocks_are_one_stock() {
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.stocks = vec![
        crate::job::StockDef {
            name: "left".into(),
            spec: crate::job::StockSpec::FixedBox(crate::job::FixedBox::at(
                [-10.0, -10.0, -8.0],
                [10.0, 20.0, 17.0],
            )),
        },
        crate::job::StockDef {
            name: "right".into(),
            spec: crate::job::StockSpec::FixedBox(crate::job::FixedBox::at(
                [0.0, -10.0, -8.0],
                [10.0, 20.0, 18.0],
            )),
        },
    ];
    job.setups[0].stocks = vec!["left".into(), "right".into()];
    let result = run_job(&job, &dome()).unwrap();
    assert_eq!(result.stocks.len(), 2);
    assert_eq!(result.heights.stock_top, 10.0);
    assert_eq!(result.stock_min, dvec2(-10.0, -10.0));
    assert_eq!(result.stock_max, dvec2(10.0, 10.0));
    assert!(result.is_clean(), "{:?}", result.warnings());
}

/// The step as the "dome" of `JOB`, with one operation left: `finish` or
/// `rough`, kept to the plateau.
fn kept_to_plateau(keep: &str, containment: crate::job::Containment) -> (Job, Body) {
    use crate::job::FeatureRef;
    use crate::setup::features::find_face;
    let body = Body::derived(synthetic::step(5.0, 10.0));
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.setups[0].operations.retain(|op| op.name == keep);
    let pick = FeatureRef {
        model: "dome".into(),
        index: 0,
        at: [-5.0, 0.0, 5.0],
    };
    let face = find_face(&body, &pick, 0.01).unwrap().index;
    let op = &mut job.setups[0].operations[0];
    op.selection.faces.push(FeatureRef {
        index: face,
        ..pick
    });
    op.boundary.containment = containment;
    op.boundary.kind = Boundary::Selection;
    (job, body)
}

/// The plate's 8 mm hole, picked by its top rim and drilled through with an
/// 8 mm drill in pecks: one hole, drilled to the model's bottom, each peck
/// backing out, and the result verifies.
fn drilled(peck: Option<f64>) -> (SetupResult, Job) {
    drilled_as(StrategySpec::Drill {
        peck,
        dwell: None,
        chip_break: None,
        pitch: None,
        hand: Default::default(),
    })
}

/// The plate's hole, drilled or tapped as `strategy` says.
fn drilled_as(strategy: StrategySpec) -> (SetupResult, Job) {
    use crate::job::{FeatureRef, ToolSpec};
    use crate::setup::features::find_edge;
    let body = Body::derived(synthetic::plate_with_hole(20.0, 5.0, 4.0, 48));
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.tools.insert(
        "drill".into(),
        toml::from_str::<ToolSpec>("kind = \"drill\"\ndiameter = 8.0\nnumber = 3\n").unwrap(),
    );
    job.setups[0].operations.retain(|op| op.name == "finish");
    let op = &mut job.setups[0].operations[0];
    op.name = "holes".into();
    op.tool = Some("drill".into());
    op.strategy = strategy;
    let pick = FeatureRef {
        model: "dome".into(),
        index: 0,
        at: [4.0, 0.0, 5.0],
    };
    let edge = find_edge(&body, &pick, 0.01).unwrap().index;
    op.selection.edges.push(FeatureRef {
        index: edge,
        ..pick
    });
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    (run_setup(&job, 0, &bodies).unwrap(), job)
}

#[test]
fn a_hole_is_drilled_through_in_pecks() {
    let (result, _) = drilled(Some(2.0));
    let op = &result.operations[0];
    let StrategyDetail::Drill(report) = &op.detail else {
        panic!("reported {:?}", op.detail);
    };
    assert_eq!(report.holes.len(), 1);
    let hole = report.holes[0];
    assert!(hole.at.length() < 1e-6, "{hole:?}");
    assert!((hole.top - hole.bottom - 5.0).abs() < 1e-6, "{hole:?}");
    assert_eq!(op.strategy, "drill");
    assert!(op.warnings.is_empty(), "{:?}", op.warnings);
    // Down to the bottom in three pecks, backing out between.
    let deepest = op
        .path
        .moves()
        .iter()
        .map(|m| m.to.z)
        .fold(f64::MAX, f64::min);
    assert!((deepest - hole.bottom).abs() < 1e-9);
    let retracts = op
        .path
        .moves()
        .iter()
        .filter(|m| m.role == crate::path::toolpath::MoveRole::Retract)
        .count();
    assert!(retracts >= 3, "{retracts} retracts");
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert!(result.verify.gouged == 0);
}

/// Breaking the chip, or tapping, the material taken is the hole drilled to
/// its depth, as drilling it at once takes; and it verifies. A tap is fed at
/// its pitch a turn, both ways, its spindle turning the thread's way.
#[test]
fn chip_breaking_and_tapping_take_the_hole() {
    let strategy = |text: &str| toml::from_str::<StrategySpec>(text).unwrap();
    let (once, _) = drilled(None);
    let removed = once.operations[0].sim.removed_mm3;
    assert!(removed > 0.0);
    for text in [
        "strategy = \"drill\"\npeck = 1.5\nchip_break = 0.3",
        "strategy = \"drill\"\npitch = 1.25",
        "strategy = \"drill\"\npitch = 1.0\nhand = \"left\"",
    ] {
        let (result, job) = drilled_as(strategy(text));
        let op = &result.operations[0];
        let r = op.sim.removed_mm3;
        assert!(
            (r - removed).abs() <= 0.01 * removed,
            "{text}: {r} of {removed}"
        );
        assert!(result.is_clean(), "{text}: {:?}", result.warnings());
        assert!(result.verify.gouged == 0, "{text}");
        let StrategyDetail::Drill(report) = &op.detail else {
            panic!("reported {:?}", op.detail);
        };
        let rpm = f64::from(op.rpm);
        assert_eq!(rpm, f64::from(job.tools["drill"].rpm));
        if let Some(tap) = report.tap {
            assert_eq!(report.feed, tap.pitch * rpm, "{text}");
            assert_eq!(op.spindle, tap.hand.spindle(), "{text}");
            // Down from the R plane and back up to it at the tap's feed; the
            // linker's moves above it are its own.
            let hole = report.holes[0];
            let tapped: Vec<_> = op
                .path
                .moves()
                .iter()
                .filter(|m| m.feed == Some(report.feed))
                .map(|m| m.to.z)
                .collect();
            assert_eq!(tapped, [hole.bottom, hole.retract()], "{text}");
        } else {
            assert_eq!(report.chip_break, Some(0.3));
        }
    }
}

/// A drill too small for its hole is drilled, and said.
#[test]
fn a_drill_the_wrong_size_is_said() {
    let (_, mut job) = drilled(None);
    job.tools.get_mut("drill").unwrap().diameter = 6.0;
    let body = Body::derived(synthetic::plate_with_hole(20.0, 5.0, 4.0, 48));
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    let w = &result.operations[0].warnings;
    assert!(w.iter().any(|w| w.contains("8.000 mm across")), "{w:?}");
}

fn cuts(result: &SetupResult) -> Vec<DVec2> {
    result.operations[0]
        .path
        .moves()
        .iter()
        .filter(|m| m.role == crate::path::toolpath::MoveRole::Cut)
        .map(|m| m.to.truncate())
        .collect()
}

/// Finishing kept to the plateau: every cut has the tool's centre over it, and
/// with the whole tool inside, a radius in from its edges.
#[test]
fn parallel_keeps_to_a_picked_face() {
    use crate::job::Containment;
    for (containment, inset) in [(Containment::Center, 0.0), (Containment::Inside, 2.0)] {
        let (job, body) = kept_to_plateau("finish", containment);
        let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
        let result = run_setup(&job, 0, &bodies).unwrap();
        let cuts = cuts(&result);
        assert!(cuts.len() > 20, "{containment:?}: only {} cuts", cuts.len());
        let slack = 1e-6 + 0.01;
        for p in &cuts {
            assert!(
                p.x <= -inset + slack
                    && p.x >= -10.0 + inset - slack
                    && p.y.abs() <= 10.0 - inset + slack,
                "{containment:?}: a cut at {p} leaves the plateau"
            );
        }
        // And the passes reach across it, not a stepover short.
        let reach = cuts.iter().map(|p| p.x).fold(f64::MIN, f64::max);
        assert!(
            reach > -inset - 0.1,
            "{containment:?}: stops at x = {reach}"
        );
    }
    // Outside, the tool's side reaches the plateau from past its edge.
    let (job, body) = kept_to_plateau("finish", Containment::Outside);
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    let far = cuts(&result).iter().map(|p| p.x).fold(f64::MIN, f64::max);
    assert!(far > 1.5, "outside stops at x = {far}");
}

/// Roughing kept to the plateau clears only around it, and less than the whole
/// block.
#[test]
fn roughing_keeps_to_a_picked_face() {
    use crate::job::Containment;
    let (job, body) = kept_to_plateau("rough", Containment::Center);
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let kept = run_setup(&job, 0, &bodies).unwrap();
    for p in cuts(&kept) {
        assert!(
            p.x <= 0.01 && p.x >= -10.01 && p.y.abs() <= 10.01,
            "a roughing cut at {p} leaves the plateau"
        );
    }
    let mut whole = job.clone();
    whole.setups[0].operations[0].selection.faces.clear();
    whole.setups[0].operations[0].boundary.kind = Boundary::Stock;
    let all = run_setup(&whole, 0, &bodies).unwrap();
    assert!(
        kept.operations[0].sim.removed_mm3 < all.operations[0].sim.removed_mm3,
        "kept {} vs whole {}",
        kept.operations[0].sim.removed_mm3,
        all.operations[0].sim.removed_mm3
    );
}

/// Roughing kept to the part's silhouette leaves the block around it alone: with
/// the tool's centre on it, nothing is cut outside the part; reaching in from
/// outside with an offset, a channel that wide is cut round it and no further.
#[test]
fn roughing_keeps_to_the_silhouette() {
    use crate::job::{Containment, StockSpec};
    let body = Body::derived(synthetic::step(5.0, 10.0));
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.setups[0].operations.retain(|op| op.name == "rough");
    job.stocks[0].spec = StockSpec::AroundModels {
        models: vec![],
        margin: [20.0, 20.0, 2.0],
        below: 0.0,
    };
    let all = run_setup(&job, 0, &bodies).unwrap();

    job.setups[0].operations[0].boundary.kind = Boundary::Silhouette;
    let inside = run_setup(&job, 0, &bodies).unwrap();
    for p in cuts(&inside) {
        assert!(
            p.x.abs() <= 10.01 && p.y.abs() <= 10.01,
            "a cut at {p} leaves the part"
        );
    }

    let b = &mut job.setups[0].operations[0].boundary;
    b.containment = Containment::Outside;
    b.margin = 6.0;
    let channel = run_setup(&job, 0, &bodies).unwrap();
    let cuts = cuts(&channel);
    // The step is 20 square; the tool's centre may go its radius and the offset
    // past it.
    let reach = 10.0 + 3.0 + 6.0 + 0.05;
    for p in &cuts {
        assert!(
            p.x.abs() <= reach && p.y.abs() <= reach,
            "a cut at {p} is past the channel"
        );
    }
    assert!(
        cuts.iter().any(|p| p.x.abs() > 12.0 || p.y.abs() > 12.0),
        "no channel is cut round the part"
    );
    let removed = |r: &SetupResult| r.operations[0].sim.removed_mm3;
    assert!(removed(&inside) < removed(&channel));
    assert!(
        removed(&channel) < removed(&all),
        "channel {} vs the whole block {}",
        removed(&channel),
        removed(&all)
    );
}

/// A pick on nothing stops the run and names the operation.
#[test]
fn a_gone_pick_stops_the_run() {
    let (mut job, body) = kept_to_plateau("finish", crate::job::Containment::Center);
    job.setups[0].operations[0].selection.faces[0].at = [40.0, 40.0, 40.0];
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let err = run_setup(&job, 0, &bodies).unwrap_err();
    assert!(
        matches!(err, RunError::Feature { ref op, .. } if op == "finish"),
        "{err}"
    );
}

/// Kept to a boundary, the same job gives the same moves on any number of
/// threads.
#[test]
fn bounded_runs_are_deterministic() {
    let (job, body) = kept_to_plateau("rough", crate::job::Containment::Center);
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let on = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| run_setup(&job, 0, &bodies).unwrap())
    };
    let (one, eight) = (on(1), on(8));
    assert_eq!(one.operations[0].path, eight.operations[0].path);
}

/// The plate with an 8 mm hole, standing in a sheet 10 mm bigger all round and no
/// thicker, and one operation with the 6 mm flat and no stock to leave, made
/// `strategy`.
fn in_a_sheet(strategy: StrategySpec, body: &Body) -> (Job, SetupResult) {
    let job = sheet_job(strategy);
    let bodies: Bodies<'_> = [("dome", body)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    (job, result)
}

/// The job [`in_a_sheet`] runs.
fn sheet_job(strategy: StrategySpec) -> Job {
    use crate::job::StockSpec;
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.setups[0].operations.retain(|op| op.name == "rough");
    job.stocks[0].spec = StockSpec::AroundModels {
        models: vec![],
        margin: [10.0, 10.0, 0.0],
        below: 0.0,
    };
    let op = &mut job.setups[0].operations[0];
    op.strategy = strategy;
    op.passes.stepdown = strategy.default_stepdown();
    op.passes.stock_to_leave = Default::default();
    job
}

/// A part profiled out of a sheet with nothing picked: round its silhouette, the
/// hole before the outline, each all the way through in even levels, and the
/// result verifies.
#[test]
fn a_part_is_profiled_out_hole_first() {
    use crate::job::ProfileSide;
    let body = Body::derived(synthetic::plate_with_hole(20.0, 5.0, 4.0, 48));
    let profile = StrategySpec::Profile {
        side: ProfileSide::Outside,
        conventional: false,
    };
    let (_, result) = in_a_sheet(profile, &body);
    let op = &result.operations[0];
    let StrategyDetail::Profile(report) = &op.detail else {
        panic!("reported {:?}", op.detail);
    };
    assert_eq!((report.areas, report.rings, report.levels), (1, 2, 3));
    assert_eq!(op.strategy, "profile");
    let cuts: Vec<DVec3> = op
        .path
        .moves()
        .iter()
        .filter(|m| m.role == crate::path::toolpath::MoveRole::Cut)
        .map(|m| m.to)
        .collect();
    let deepest = cuts.iter().map(|p| p.z).fold(f64::MAX, f64::min);
    assert!(deepest.abs() < 1e-9, "down to {deepest}");
    // The hole's ring, a tool's radius inside it, then the outline's, one outside.
    let first = cuts.first().unwrap().truncate().length();
    assert!(first < 1.1, "starts {first} from the hole's middle");
    let last = cuts.last().unwrap().truncate();
    assert!(last.x.abs().max(last.y.abs()) > 22.9, "ends at {last}");
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);
}

/// With finishing passes asked for, the same pocket is cleared short of its
/// walls and then finished round them, at the finishing feed, on and off along
/// leads in what is cleared; it still reaches the walls and verifies.
#[test]
fn a_pocket_with_finishing_passes_verifies() {
    use crate::job::{FeatureRef, Finishing};
    use crate::path::toolpath::MoveRole;
    use crate::setup::features::find_face;
    let body = Body::derived(synthetic::step(5.0, 10.0));
    let pick = FeatureRef {
        model: "dome".into(),
        index: 0,
        at: [5.0, 0.0, 0.0],
    };
    let face = find_face(&body, &pick, 0.01).unwrap().index;
    let pocket = StrategySpec::Pocket {
        conventional: false,
    };
    let (mut job, _) = in_a_sheet(pocket, &body);
    job.stocks[0].spec = crate::job::StockSpec::AroundModels {
        models: vec![],
        margin: [0.0, 0.0, 0.0],
        below: 0.0,
    };
    let op = &mut job.setups[0].operations[0];
    op.selection.faces.push(FeatureRef {
        index: face,
        ..pick
    });
    op.outline = Outline::Selection;
    op.passes.finishing = Some(Finishing {
        passes: Some(2),
        feed: Some(333.0),
        ..Finishing::default()
    });
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    let op = &result.operations[0];
    let moves = op.path.moves();
    assert!(
        moves
            .iter()
            .any(|m| m.feed == Some(333.0) && m.role == MoveRole::Cut)
    );
    assert!(moves.iter().any(|m| m.role == MoveRole::Lead));
    // Still to the wall, and no further.
    let cuts: Vec<DVec3> = moves
        .iter()
        .filter(|m| m.role == MoveRole::Cut)
        .map(|m| m.to)
        .collect();
    assert!(cuts.iter().all(|p| p.x >= 3.0 - 1e-6 && p.x <= 7.0 + 1e-6));
    assert!(
        cuts.iter()
            .any(|p| (p.x - 3.0).abs() < 0.03 && p.z.abs() < 1e-9)
    );
    assert!(op.sim.removed_mm3 > 950.0, "{}", op.sim.removed_mm3);
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);
}

/// A pocket picked by its floor, the step's shelf: cleared to it within the
/// shelf's outline, the tool's side on the wall and nowhere past it, and the
/// result verifies. A tool too wide for it is refused.
#[test]
fn a_pocket_is_cleared_to_its_picked_floor() {
    use crate::job::FeatureRef;
    use crate::setup::features::find_face;
    let body = Body::derived(synthetic::step(5.0, 10.0));
    let pick = FeatureRef {
        model: "dome".into(),
        index: 0,
        at: [5.0, 0.0, 0.0],
    };
    let face = find_face(&body, &pick, 0.01).unwrap().index;
    let pocket = StrategySpec::Pocket {
        conventional: false,
    };
    let (mut job, _) = in_a_sheet(pocket, &body);
    job.stocks[0].spec = crate::job::StockSpec::AroundModels {
        models: vec![],
        margin: [0.0, 0.0, 0.0],
        below: 0.0,
    };
    job.setups[0].operations[0]
        .selection
        .faces
        .push(FeatureRef {
            index: face,
            ..pick
        });
    job.setups[0].operations[0].outline = Outline::Selection;
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    let op = &result.operations[0];
    let StrategyDetail::Pocket(report) = &op.detail else {
        panic!("reported {:?}", op.detail);
    };
    assert_eq!((report.areas, report.levels), (1, 3));
    let cuts: Vec<DVec3> = op
        .path
        .moves()
        .iter()
        .filter(|m| m.role == crate::path::toolpath::MoveRole::Cut)
        .map(|m| m.to)
        .collect();
    for p in &cuts {
        assert!(
            p.x >= 3.0 - 1e-6 && p.x <= 7.0 + 1e-6 && p.y.abs() <= 7.0 + 1e-6,
            "a cut at {p} leaves the shelf, less the tool's radius"
        );
    }
    assert!(
        cuts.iter()
            .any(|p| (p.x - 3.0).abs() < 0.03 && p.z.abs() < 1e-9)
    );
    // All of the shelf's block but its corners, which a round tool leaves.
    assert!(op.sim.removed_mm3 > 950.0, "{}", op.sim.removed_mm3);
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);

    // Measured from the picked floor: from 3 mm above it to half a millimetre
    // under it, in two even levels.
    let from_picks = |offset| {
        Some(Height {
            reference: HeightRef::Selection,
            offset,
        })
    };
    let h = &mut job.setups[0].operations[0].heights;
    h.top = from_picks(3.0);
    h.bottom = from_picks(-0.5);
    let result = run_setup(&job, 0, &bodies).unwrap();
    let op = &result.operations[0];
    let StrategyDetail::Pocket(report) = &op.detail else {
        panic!("reported {:?}", op.detail);
    };
    assert_eq!(report.levels, 2);
    let zs: Vec<f64> = op
        .path
        .moves()
        .iter()
        .filter(|m| m.role == crate::path::toolpath::MoveRole::Cut)
        .map(|m| m.to.z)
        .collect();
    let deepest = zs.iter().copied().fold(f64::MAX, f64::min);
    let highest = zs.iter().copied().fold(f64::MIN, f64::max);
    assert!((deepest + 0.5).abs() < 1e-9, "down to {deepest}");
    assert!((highest - 1.25).abs() < 1e-9, "first level at {highest}");
    let h = &mut job.setups[0].operations[0].heights;
    (h.top, h.bottom) = (None, None);

    job.tools.get_mut("rough").unwrap().diameter = 25.0;
    let err = run_setup(&job, 0, &bodies).unwrap_err().to_string();
    assert!(err.contains("does not fit"), "{err}");
}

/// The radial stock to leave moves a 2D cut, and an additional offset, a 3D
/// machining boundary's, does not: a profile out by it, a pocket in. The pocket
/// still goes down to the floor picked on the boundary itself.
#[test]
fn stock_to_leave_moves_a_2d_cut_and_an_offset_does_not() {
    use crate::job::{FeatureRef, ProfileSide};
    use crate::setup::features::find_face;
    let cuts = |result: &SetupResult| -> Vec<DVec3> {
        result.operations[0]
            .path
            .moves()
            .iter()
            .filter(|m| m.role == crate::path::toolpath::MoveRole::Cut)
            .map(|m| m.to)
            .collect()
    };

    let plate = Body::derived(synthetic::plate_with_hole(20.0, 5.0, 4.0, 48));
    let profile = StrategySpec::Profile {
        side: ProfileSide::Outside,
        conventional: false,
    };
    let (mut job, _) = in_a_sheet(profile, &plate);
    let op = &mut job.setups[0].operations[0];
    op.passes.stock_to_leave.radial = 2.0;
    op.boundary.margin = 5.0;
    let bodies: Bodies<'_> = [("dome", &plate)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    // The outline's ring a tool's radius and 2 mm outside it; the hole's, a
    // radius inside the hole less 2 mm, nearer its middle.
    let reach = cuts(&result)
        .iter()
        .map(|p| p.x.abs().max(p.y.abs()))
        .fold(0.0, f64::max);
    assert!((reach - 25.0).abs() < 0.05, "reaches {reach}");

    let step = Body::derived(synthetic::step(5.0, 10.0));
    let pick = FeatureRef {
        model: "dome".into(),
        index: 0,
        at: [5.0, 0.0, 0.0],
    };
    let face = find_face(&step, &pick, 0.01).unwrap().index;
    let pocket = StrategySpec::Pocket {
        conventional: false,
    };
    let (mut job, _) = in_a_sheet(pocket, &step);
    job.stocks[0].spec = crate::job::StockSpec::AroundModels {
        models: vec![],
        margin: [0.0, 0.0, 0.0],
        below: 0.0,
    };
    let op = &mut job.setups[0].operations[0];
    op.selection.faces.push(FeatureRef {
        index: face,
        ..pick
    });
    op.outline = Outline::Selection;
    job.setups[0].operations[0].passes.stock_to_leave.radial = 1.0;
    let bodies: Bodies<'_> = [("dome", &step)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    let cuts = cuts(&result);
    for p in &cuts {
        assert!(
            p.x >= 4.0 - 1e-6 && p.x <= 6.0 + 1e-6 && p.y.abs() <= 6.0 + 1e-6,
            "a cut at {p} is not a millimetre further in"
        );
    }
    assert!(
        cuts.iter().any(|p| p.z.abs() < 1e-9),
        "not down to the floor"
    );
}

/// Tabs hold a part profiled out of a sheet: the deepest cut rides over them at
/// their top, less is cut, nothing rapids into what they leave, and the result
/// verifies.
#[test]
fn a_profiled_part_is_held_by_tabs() {
    use crate::job::{ProfileSide, TabShape, Tabs};
    let plate = Body::derived(synthetic::plate_with_hole(20.0, 5.0, 4.0, 48));
    let profile = StrategySpec::Profile {
        side: ProfileSide::Outside,
        conventional: false,
    };
    let (mut job, without) = in_a_sheet(profile, &plate);
    let bodies: Bodies<'_> = [("dome", &plate)].into_iter().collect();
    for shape in [TabShape::Rectangular, TabShape::Triangular] {
        job.setups[0].operations[0].passes.tabs = Some(Tabs {
            shape,
            width: 4.0,
            height: 2.0,
            spacing: 60.0,
        });
        let with = run_setup(&job, 0, &bodies).unwrap();
        let op = &with.operations[0];
        assert!(with.is_clean(), "{shape:?}: {:?}", with.warnings());
        assert_eq!(with.verify.gouged, 0);
        assert_eq!(op.sim.rapid_contacts, 0);
        // The deepest level is 0; the tabs' top 2 mm up, and cut along.
        let on_top = op
            .path
            .moves()
            .iter()
            .filter(|m| {
                m.role == crate::path::toolpath::MoveRole::Cut && (m.to.z - 2.0).abs() < 1e-9
            })
            .count();
        assert!(on_top > 0, "{shape:?}: nothing rides over a tab");
        let (a, b) = (op.sim.removed_mm3, without.operations[0].sim.removed_mm3);
        assert!(
            a < b - 10.0,
            "{shape:?}: removed {a} against {b} without tabs"
        );
    }
}

/// A profile with leads enters each ring along an arc and leaves along another,
/// with tabs on it still, and the result verifies: the arcs stay in the waste.
#[test]
fn a_profile_leads_on_and_off_each_ring() {
    use crate::job::{Leads, ProfileSide, Tabs};
    use crate::path::toolpath::MoveRole;
    let plate = Body::derived(synthetic::plate_with_hole(20.0, 5.0, 4.0, 48));
    let profile = StrategySpec::Profile {
        side: ProfileSide::Outside,
        conventional: false,
    };
    let (mut job, _) = in_a_sheet(profile, &plate);
    let op = &mut job.setups[0].operations[0];
    op.linking.leads = Some(Leads::default());
    op.passes.tabs = Some(Tabs {
        width: 4.0,
        height: 2.0,
        spacing: 60.0,
        ..Default::default()
    });
    let bodies: Bodies<'_> = [("dome", &plate)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    let op = &result.operations[0];
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);
    assert_eq!(op.sim.rapid_contacts, 0);
    let roles: Vec<MoveRole> = op.path.moves().iter().map(|m| m.role).collect();
    // Every run of cuts is entered along a lead and left along one. A tab's
    // step down is part of the run.
    let mut runs = 0;
    for (i, w) in roles.windows(2).enumerate() {
        if w == [MoveRole::Lead, MoveRole::Cut] {
            runs += 1;
            let after = roles[i + 1..]
                .iter()
                .position(|r| !matches!(r, MoveRole::Cut | MoveRole::Plunge));
            assert_eq!(
                after.map(|k| roles[i + 1 + k]),
                Some(MoveRole::Lead),
                "a ring left without a lead"
            );
        }
    }
    // Two rings, three levels each.
    assert_eq!(runs, 6, "{roles:?}");
    // Tabs still ride over the deepest.
    assert!(
        op.path
            .moves()
            .iter()
            .any(|m| m.role == MoveRole::Cut && (m.to.z - 2.0).abs() < 1e-9)
    );
}

/// Stock to leave beside moves a profile out by it, and negative stock in by it,
/// as Fusion's does; what is cut past the part on purpose is no gouge.
#[test]
fn a_profile_leaves_stock_beside_or_cuts_past_the_part() {
    use crate::job::{ProfileSide, StockToLeave};
    let plate = Body::derived(synthetic::plate_with_hole(20.0, 5.0, 4.0, 48));
    let profile = StrategySpec::Profile {
        side: ProfileSide::Outside,
        conventional: false,
    };
    let (mut job, _) = in_a_sheet(profile, &plate);
    let bodies: Bodies<'_> = [("dome", &plate)].into_iter().collect();
    for radial in [0.5, -0.5] {
        job.setups[0].operations[0].passes.stock_to_leave = StockToLeave { radial, axial: 0.0 };
        let result = run_setup(&job, 0, &bodies).unwrap();
        let reach = result.operations[0]
            .path
            .moves()
            .iter()
            .filter(|m| m.role == crate::path::toolpath::MoveRole::Cut)
            .map(|m| m.to.x.abs().max(m.to.y.abs()))
            .fold(0.0, f64::max);
        assert!(
            (reach - (23.0 + radial)).abs() < 0.05,
            "{radial}: reaches {reach}"
        );
        assert_eq!(result.verify.gouged, 0, "{radial}");
    }
}

/// Stock left beside a pocket's walls stays in the material: the pocket's cut
/// keeps off them by it, and less is taken out.
#[test]
fn a_pocket_leaves_stock_on_its_walls() {
    use crate::job::{FeatureRef, StockToLeave};
    use crate::setup::features::find_face;
    let body = Body::derived(synthetic::step(5.0, 10.0));
    let pick = FeatureRef {
        model: "dome".into(),
        index: 0,
        at: [5.0, 0.0, 0.0],
    };
    let face = find_face(&body, &pick, 0.01).unwrap().index;
    let pocket = StrategySpec::Pocket {
        conventional: false,
    };
    let (mut job, _) = in_a_sheet(pocket, &body);
    job.stocks[0].spec = crate::job::StockSpec::AroundModels {
        models: vec![],
        margin: [0.0, 0.0, 0.0],
        below: 0.0,
    };
    job.setups[0].operations[0]
        .selection
        .faces
        .push(FeatureRef {
            index: face,
            ..pick
        });
    job.setups[0].operations[0].outline = Outline::Selection;
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let removed = |job: &Job| {
        let result = run_setup(job, 0, &bodies).unwrap();
        assert_eq!(result.verify.gouged, 0);
        let op = &result.operations[0];
        for m in op.path.moves() {
            if m.role == crate::path::toolpath::MoveRole::Cut {
                let leave = job.setups[0].operations[0].passes.stock_to_leave.radial;
                assert!(
                    m.to.x >= 3.0 + leave - 1e-6 && m.to.x <= 7.0 - leave + 1e-6,
                    "a cut at {} is not {leave} off the walls",
                    m.to
                );
            }
        }
        op.sim.removed_mm3
    };
    let bare = removed(&job);
    job.setups[0].operations[0].passes.stock_to_leave = StockToLeave {
        radial: 0.5,
        axial: 0.0,
    };
    let leaving = removed(&job);
    assert!(
        leaving < bare - 50.0,
        "removed {leaving} leaving stock, {bare} not"
    );
}

/// Roughing the same again, leaving the same stock, finds nothing to cut: the
/// stock left on the walls is the part's to keep, not material for a ring.
#[test]
fn roughing_twice_leaves_nothing_for_the_second() {
    let mut job: Job = toml::from_str(JOB).unwrap();
    let rough = job.setups[0].operations[0].clone();
    job.setups[0].operations = vec![
        rough.clone(),
        Operation {
            name: "again".into(),
            ..rough
        },
    ];
    let result = run_job(&job, &dome()).unwrap();
    let cut = |o: &OperationResult| -> f64 {
        let moves = o.path.moves();
        moves
            .windows(2)
            .filter(|w| w[1].role == crate::path::toolpath::MoveRole::Cut)
            .map(|w| w[0].to.distance(w[1].to))
            .sum()
    };
    let (first, again) = (&result.operations[0], &result.operations[1]);
    assert!(
        again.sim.removed_mm3 < 1.0,
        "removed {}",
        again.sim.removed_mm3
    );
    assert!(
        cut(again) < 0.02 * cut(first),
        "cut {} mm again against {} mm",
        cut(again),
        cut(first)
    );
}

/// Facing takes the stock above the part down to its top, all of it and past
/// its edges, off the stock at both ends of every pass, and nothing of the part:
/// the result verifies.
#[test]
fn facing_takes_the_stock_down_to_the_part() {
    use crate::job::{FaceDirection, StockSpec};
    let plate = Body::derived(synthetic::plate_with_hole(20.0, 5.0, 4.0, 48));
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.setups[0].operations.retain(|op| op.name == "rough");
    job.stocks[0].spec = StockSpec::AroundModels {
        models: vec![],
        margin: [10.0, 10.0, 3.0],
        below: 0.0,
    };
    let op = &mut job.setups[0].operations[0];
    op.strategy = StrategySpec::Face {
        angle: 0.0,
        direction: FaceDirection::BothWays,
        pass_extension: 1.0,
        stock_offset: 0.0,
    };
    op.passes.stepdown = Some(2.0);
    op.passes.stock_to_leave = Default::default();
    let diameter = job.tools[op.tool.as_ref().unwrap()].diameter;
    op.passes.stepover = 0.9 * diameter;
    job.validate().unwrap();
    let bodies: Bodies<'_> = [("dome", &plate)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    let op = &result.operations[0];
    let StrategyDetail::Face(report) = &op.detail else {
        panic!("reported {:?}", op.detail);
    };
    assert_eq!(report.levels, 2);
    assert_eq!(op.strategy, "face");
    let top = result.heights.model_top;
    let cuts: Vec<DVec3> = op
        .path
        .moves()
        .iter()
        .filter(|m| m.role == crate::path::toolpath::MoveRole::Cut)
        .map(|m| m.to)
        .collect();
    let deepest = cuts.iter().map(|p| p.z).fold(f64::MAX, f64::min);
    assert!((deepest - top).abs() < 1e-9, "down to {deepest}, not {top}");
    // Every pass starts and ends off the stock, a tool's radius and the
    // extension past it.
    let reach = diameter / 2.0 + 1.0;
    for p in &cuts {
        assert!(p.y.abs() <= 30.0 + 1e-9, "{p}");
    }
    assert!(
        cuts.iter()
            .all(|p| (p.x.abs() - (30.0 + reach)).abs() < 1e-9)
    );
    // 60 by 60 by 3, less the grid's edges.
    let taken = op.sim.removed_mm3;
    assert!((taken - 10_800.0).abs() < 400.0, "took {taken}");
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);
}

/// The shared job with its roughing made adaptive: a tenth of the 6 mm flat's
/// width, the same levels.
fn adaptive_job() -> Job {
    use crate::job::CutDirection;
    let mut job: Job = toml::from_str(JOB).unwrap();
    let rough = &mut job.setups[0].operations[0];
    rough.strategy = StrategySpec::Adaptive {
        direction: CutDirection::Climb,
        flat_areas: true,
        rest: None,
    };
    rough.passes.stepover = 0.6;
    job
}

/// Adaptive clearing roughs the dome, the ball finishes it, and the result is
/// the part: nothing gouged, no rapid through material, every link resolved.
#[test]
fn adaptive_roughing_is_finished_to_the_part() {
    let job = adaptive_job();
    let result = run_job(&job, &dome()).unwrap();
    let rough = &result.operations[0];
    let StrategyDetail::Adaptive(r) = rough.detail else {
        panic!("rough reported {:?}", rough.detail);
    };
    assert!(r.passes > 0 && r.levels > 0, "{r:?}");
    assert_eq!(r.no_room, 0, "{r:?}");
    assert!(r.left_mm2 < 1.0, "{r:?}");
    assert!(!rough.path.has_unplanned_links());
    assert!(rough.sim.is_clean(), "{:?}", rough.warnings);
    assert!(
        result.is_clean(),
        "job did not verify: {:?}",
        result.warnings()
    );
}

/// Held to a tenth of the tool, it still takes off what Z-level roughing does,
/// and more, as its flat levels take the terraces over flats.
#[test]
fn adaptive_clears_what_zlevel_roughing_does() {
    use crate::job::StockSpec;
    let body = Body::derived(synthetic::step(5.0, 10.0));
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let removed = |mut job: Job| {
        job.setups[0].operations.retain(|op| op.name == "rough");
        job.stocks[0].spec = StockSpec::AroundModels {
            models: vec![],
            margin: [8.0, 8.0, 2.0],
            below: 0.0,
        };
        let result = run_setup(&job, 0, &bodies).unwrap();
        assert!(result.is_clean(), "{:?}", result.warnings());
        result.operations[0].sim.removed_mm3
    };
    let zlevel = removed(toml::from_str(JOB).unwrap());
    let adaptive = removed(adaptive_job());
    assert!(
        adaptive > 0.99 * zlevel,
        "adaptive removed {adaptive} mm3 against {zlevel}"
    );
}

/// Kept inside the part's plateau, there is no way in from the side: it goes
/// down a helix, and the result still verifies.
#[test]
fn adaptive_goes_down_a_helix_where_it_cannot_come_from_the_side() {
    let (mut job, body) = kept_to_plateau("rough", crate::job::Containment::Center);
    let op = &mut job.setups[0].operations[0];
    op.strategy = StrategySpec::Adaptive {
        direction: crate::job::CutDirection::Climb,
        flat_areas: true,
        rest: None,
    };
    op.passes.stepover = 0.6;
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    let rough = &result.operations[0];
    assert!(rough.links.helixes > 0, "{:?}", rough.links);
    assert!(rough.sim.is_clean(), "{:?}", rough.warnings);
    assert!(result.is_clean(), "{:?}", result.warnings());
}

#[test]
fn adaptive_runs_are_deterministic() {
    let job = adaptive_job();
    let body = Body::derived(dome());
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let on = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| run_setup(&job, 0, &bodies).unwrap())
    };
    let (one, eight) = (on(1), on(8));
    assert_eq!(one.operations[0].path, eight.operations[0].path);
}

/// An operation turned off or skipped cuts nothing, so the material after it
/// is the material before it: the same keyframe, not another copy of it.
#[test]
fn an_operation_left_out_keeps_the_keyframe_before_it() {
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.setups[0].operations[0].enabled = false;
    let body = Body::derived(dome());
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let prepared = crate::setup::prepare(&job, 0, &bodies).unwrap();
    let plans = [Plan::Generate, Plan::Skip];
    let result = run_prepared_planned(&job, 0, &prepared, &bodies, &plans, &mut |_| {}).unwrap();
    let k = &result.keyframes;
    assert_eq!(k.len(), 3);
    assert!(Arc::ptr_eq(&k[0], &k[1]) && Arc::ptr_eq(&k[1], &k[2]));
    assert_eq!(result.operations[0].made, Made::Disabled);
    assert_eq!(result.operations[1].made, Made::Skipped);
}

/// The plate faced 3 mm and then cleared round, adaptively, out of `stock`: the
/// run, and each operation's cut length.
fn faced_and_cleared(stock: crate::job::StockSpec) -> (SetupResult, [f64; 2]) {
    use crate::job::{CutDirection, FaceDirection};
    use crate::path::toolpath::MoveRole;
    let plate = Body::derived(synthetic::plate_with_hole(20.0, 5.0, 4.0, 48));
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.stocks[0].spec = stock;
    let diameter = job.tools["rough"].diameter;
    let mut face = job.setups[0].operations[0].clone();
    face.name = "face".into();
    face.strategy = StrategySpec::Face {
        angle: 0.0,
        direction: FaceDirection::BothWays,
        pass_extension: 1.0,
        stock_offset: 0.0,
    };
    face.passes.stepdown = Some(2.0);
    face.passes.stepover = 0.9 * diameter;
    face.passes.stock_to_leave = Default::default();
    let mut clear = job.setups[0].operations[0].clone();
    clear.strategy = StrategySpec::Adaptive {
        direction: CutDirection::Climb,
        flat_areas: false,
        rest: None,
    };
    clear.passes.stepover = 0.6;
    clear.passes.stock_to_leave = Default::default();
    job.setups[0].operations = vec![face, clear];
    job.validate().unwrap();
    let bodies: Bodies<'_> = [("dome", &plate)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    let cut = |o: &OperationResult| {
        let mut from = None;
        let mut length = 0.0;
        for m in o.path.moves() {
            if let Some(f) = from
                && m.role == MoveRole::Cut
            {
                length += m.to.distance(f);
            }
            from = Some(m.to);
        }
        length
    };
    let lengths = [cut(&result.operations[0]), cut(&result.operations[1])];
    (result, lengths)
}

/// Round stock up the tool, round the plate: facing takes off its disc and
/// no more, adaptive clearing clears the ring round the plate, and neither
/// cuts the air its box's corners would be, gouges, or warns of anything the
/// box does not.
#[test]
fn round_stock_is_cut_where_it_is() {
    use crate::job::{BarAxis, StockSpec};
    use std::f64::consts::PI;
    let (round, [round_face, round_clear]) = faced_and_cleared(StockSpec::CylinderAroundModels {
        models: vec![],
        axis: BarAxis::Z,
        margin: 3.0,
        ends: 3.0,
    });
    let Some(camcraft_sim::stock::Block::Bar(bar)) = round.stocks.first().copied() else {
        panic!("{:?}", round.stocks);
    };
    // The plate's corners are 20√2 from its middle.
    assert!(
        (bar.radius - (800.0_f64.sqrt() + 3.0)).abs() < 1e-6,
        "{bar:?}"
    );
    let r = bar.radius;
    // The box round the same disc.
    let (boxed, [box_face, box_clear]) = faced_and_cleared(StockSpec::AroundModels {
        models: vec![],
        margin: [r - 20.0, r - 20.0, 3.0],
        below: 3.0,
    });
    for result in [&round, &boxed] {
        assert!(result.is_clean(), "{:?}", result.warnings());
        assert_eq!(result.verify.gouged, 0);
        for op in &result.operations {
            assert!(op.sim.is_clean(), "{}: {:?}", op.name, op.sim);
            assert!(!op.path.has_unplanned_links(), "{}", op.name);
        }
    }
    for w in round.warnings() {
        assert!(boxed.warnings().contains(&w), "only round: {w}");
    }
    // Faced 3 mm: the disc, not the box, less the grid's edge.
    let taken = round.operations[0].sim.removed_mm3;
    let disc = PI * r * r * 3.0;
    assert!((taken - disc).abs() < 0.03 * disc, "took {taken} of {disc}");
    // Adaptive takes the ring round the plate and its hole, 5 deep, and no
    // corners.
    let taken = round.operations[1].sim.removed_mm3;
    let ring = (PI * r * r - 40.0 * 40.0 + PI * 16.0) * 5.0;
    assert!(
        taken < 1.03 * ring && taken > 0.9 * ring,
        "took {taken} of {ring}"
    );
    // The box's corners are air in the round stock: its passes are shorter.
    assert!(
        round_clear < 0.9 * box_clear,
        "{round_clear} against {box_clear}"
    );
    // Facing goes over the box's footprint either way: the same passes.
    assert!((round_face - box_face).abs() < 1e-6 * box_face);
}

/// A bar lying along X, the plate inside it: facing down to the plate's top
/// takes off the bar's cap above it, the circle's segment along its length,
/// and the run verifies.
#[test]
fn a_lying_bar_is_faced_down_its_curve() {
    use crate::job::{BarAxis, FixedCylinder, Place, StockSpec};
    let (result, _) = faced_and_cleared(StockSpec::FixedCylinder(FixedCylinder {
        models: vec![],
        axis: BarAxis::X,
        diameter: 60.0,
        length: 50.0,
        place: Place::Center {},
    }));
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);
    for op in &result.operations {
        assert!(op.sim.is_clean(), "{}: {:?}", op.name, op.sim);
    }
    // The plate is 5 high, so the bar's axis is at 2.5 and the plate's top
    // 2.5 above it: the segment of the circle over that, 50 long.
    let (r, d): (f64, f64) = (30.0, 2.5);
    let segment = r * r * (d / r).acos() - d * (r * r - d * d).sqrt();
    let taken = result.operations[0].sim.removed_mm3;
    let cap = segment * 50.0;
    assert!((taken - cap).abs() < 0.03 * cap, "took {taken} of {cap}");
}

/// A plate 40 mm square and 5 thick, in a sheet its size, and one operation
/// of `strategy` with `tool`, following `curves` drawn on its top.
fn drawn_on_a_plate(
    strategy: StrategySpec,
    tool: crate::job::ToolSpec,
    curves: Vec<crate::job::Curve>,
) -> (Job, SetupResult) {
    drawn_on_a_plate_with(strategy, tool, curves, |_| {})
}

/// As [`drawn_on_a_plate`], the operation changed by `edit` first.
fn drawn_on_a_plate_with(
    strategy: StrategySpec,
    tool: crate::job::ToolSpec,
    curves: Vec<crate::job::Curve>,
    edit: impl FnOnce(&mut Operation),
) -> (Job, SetupResult) {
    use crate::job::StockSpec;
    let body = Body::derived(synthetic::cuboid(
        DVec3::new(-20.0, -20.0, 0.0),
        DVec3::new(20.0, 20.0, 5.0),
    ));
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.setups[0].operations.retain(|op| op.name == "rough");
    job.stocks[0].spec = StockSpec::AroundModels {
        models: vec![],
        margin: [0.0, 0.0, 0.0],
        below: 0.0,
    };
    job.tools.insert("drawn".into(), tool);
    let op = &mut job.setups[0].operations[0];
    op.strategy = strategy;
    op.tool = Some("drawn".into());
    op.passes.stepdown = strategy.default_stepdown();
    op.passes.stock_to_leave = Default::default();
    op.selection.curves = curves;
    edit(op);
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    (job, result)
}

/// A circle of radius `r` about `(x, y)` on the plate's top, as two arcs.
fn drawn_circle(x: f64, y: f64, r: f64) -> crate::job::Curve {
    use crate::job::{Curve, Segment};
    Curve {
        start: [x + r, y, 5.0],
        segments: vec![
            Segment::Arc {
                via: [x, y + r, 5.0],
                to: [x - r, y, 5.0],
            },
            Segment::Arc {
                via: [x, y - r, 5.0],
                to: [x + r, y, 5.0],
            },
        ],
    }
}

fn flat(diameter: f64) -> crate::job::ToolSpec {
    crate::job::ToolSpec {
        kind: crate::job::ToolKind::Flat,
        diameter,
        number: 3,
        ..crate::job::ToolSpec::default()
    }
}

/// Engraved half a millimetre into the plate along a circle and an open line:
/// the circle's arcs are cut as arcs, material goes only from under the
/// tool's path, and what it cuts into the part is meant, so verifies.
#[test]
fn an_engraving_cuts_only_along_its_curves_and_verifies() {
    use crate::job::{Curve, Segment, TraceDirection};
    use crate::path::toolpath::{MoveKind, MoveRole};
    let line = Curve {
        start: [-15.0, -15.0, 5.0],
        segments: vec![Segment::Line {
            to: [15.0, -15.0, 5.0],
        }],
    };
    let engrave = StrategySpec::Engrave {
        depth: 0.5,
        direction: TraceDirection::Climb,
        reverse: false,
    };
    let (_, result) = drawn_on_a_plate(engrave, flat(1.0), vec![drawn_circle(0.0, 0.0, 8.0), line]);
    let op = &result.operations[0];
    let StrategyDetail::Engrave(report) = op.detail else {
        panic!("reported {:?}", op.detail);
    };
    assert_eq!((report.curves, report.closed), (2, 1));
    assert!(report.arcs >= 4, "{report:?}");
    let arcs = op
        .path
        .moves()
        .iter()
        .filter(|m| matches!(m.kind, MoveKind::Arc(_)))
        .count();
    assert_eq!(arcs as u64, report.arcs);
    let deepest = op
        .path
        .moves()
        .iter()
        .filter(|m| m.role == MoveRole::Cut)
        .map(|m| m.to.z)
        .fold(f64::MAX, f64::min);
    assert!((deepest - 4.5).abs() < 1e-9, "{deepest}");
    assert!(op.sim.removed_mm3 > 0.0);
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);

    // Every cell the material went down in is under the tool's path.
    let before = result.keyframes[0].unpack().unwrap();
    let after = result.keyframes[1].unpack().unwrap();
    let (before, after) = (before.field(), after.field());
    let near = |p: DVec2| {
        let on_circle = (p.length() - 8.0).abs();
        let on_line = if p.x.abs() <= 15.0 {
            (p.y + 15.0).abs()
        } else {
            p.distance(dvec2(15.0 * p.x.signum(), -15.0))
        };
        on_circle.min(on_line)
    };
    let reach = 0.5 + 2.0 * before.pitch();
    let mut lowered = 0;
    for iy in 0..before.ny() {
        for ix in 0..before.nx() {
            let (Some(b), Some(a)) = (before.get(ix, iy), after.get(ix, iy)) else {
                continue;
            };
            if a < b - 1e-3 {
                lowered += 1;
                let p = before.position(ix, iy).unwrap();
                assert!(near(p) <= reach, "cut at {p}, {} from the curves", near(p));
                assert!(a > 4.5 - 0.01, "cut to {a} at {p}");
            }
        }
    }
    assert!(lowered > 100, "{lowered}");
}

/// A V-carving of a circle drawn on the plate goes to its centre, as deep as
/// the V is the circle's width there; any tool but a V-bit is refused.
#[test]
fn a_v_carving_goes_as_deep_as_the_area_is_wide() {
    use crate::job::ToolKind;
    let vbit = crate::job::ToolSpec {
        kind: ToolKind::Cone,
        diameter: 12.0,
        angle: Some(90.0),
        number: 4,
        ..crate::job::ToolSpec::default()
    };
    let carve = StrategySpec::VCarve { max_depth: None };
    let (mut job, result) = drawn_on_a_plate(carve, vbit, vec![drawn_circle(0.0, 0.0, 2.0)]);
    let op = &result.operations[0];
    let StrategyDetail::VCarve(report) = op.detail else {
        panic!("reported {:?}", op.detail);
    };
    // A 90° V is as deep as it is wide either side.
    assert!((report.deepest - 2.0).abs() < 0.02, "{report:?}");
    let lowest = op
        .path
        .moves()
        .iter()
        .min_by(|a, b| a.to.z.total_cmp(&b.to.z))
        .unwrap()
        .to;
    assert!(lowest.truncate().length() < 0.05, "{lowest}");
    assert!((lowest.z - 3.0).abs() < 0.02, "{lowest}");
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);

    job.tools.insert("drawn".into(), flat(6.0));
    let refused = job.check_setup(0).unwrap_err().to_string();
    assert!(refused.contains("V-bit"), "{refused}");
}

/// A picked round edge is followed as arcs: the plate's hole engraved round
/// its rim, a chamfer's start.
#[test]
fn a_picked_round_edge_is_engraved_as_arcs() {
    use crate::job::{FeatureRef, TraceDirection};
    use crate::path::toolpath::MoveKind;
    let body = Body::derived(synthetic::plate_with_hole(20.0, 5.0, 4.0, 48));
    let engrave = StrategySpec::Engrave {
        depth: 0.5,
        direction: TraceDirection::AsDrawn,
        reverse: false,
    };
    let mut job = sheet_job(engrave);
    let op = &mut job.setups[0].operations[0];
    op.selection.edges = vec![FeatureRef {
        model: "dome".into(),
        index: u32::MAX,
        at: [4.0, 0.0, 5.0],
    }];
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    let op = &result.operations[0];
    let StrategyDetail::Engrave(report) = op.detail else {
        panic!("reported {:?}", op.detail);
    };
    assert_eq!((report.curves, report.closed), (1, 1));
    for m in op
        .path
        .moves()
        .iter()
        .filter(|m| m.role != crate::path::toolpath::MoveRole::Rapid)
    {
        if let MoveKind::Arc(a) = m.kind {
            assert!((a.centre.truncate()).length() < 1e-6, "{a:?}");
        }
    }
    assert_eq!(report.arcs, 4, "{report:?}");
}

/// A line and an arc on the plate's top, end to end: a slot's curve.
fn drawn_hook() -> crate::job::Curve {
    use crate::job::{Curve, Segment};
    Curve {
        start: [-15.0, 0.0, 5.0],
        segments: vec![
            Segment::Line {
                to: [0.0, 0.0, 5.0],
            },
            Segment::Arc {
                via: [5.0, 5.0, 5.0],
                to: [0.0, 10.0, 5.0],
            },
        ],
    }
}

/// A slot 3 mm deep along a line and an arc, a millimetre a level, both
/// ways and one way: as deep as asked, its arc an arc at every level, and
/// what it cuts into the part meant, so it verifies. Both ways it never
/// lifts; one way it lifts at the end of each level, back to the start.
#[test]
fn a_slot_goes_down_both_ways_or_one_way_and_verifies() {
    use crate::job::{SlotPattern, ToolKind};
    use crate::path::toolpath::{MoveKind, MoveRole};
    for pattern in [SlotPattern::Bidirectional, SlotPattern::Directional] {
        let slot = StrategySpec::Slot {
            depth: 3.0,
            pattern,
            reverse: false,
        };
        assert_eq!(slot.default_stepdown(), Some(1.0));
        let (mut job, result) = drawn_on_a_plate(slot, flat(4.0), vec![drawn_hook()]);
        let op = &result.operations[0];
        let StrategyDetail::Engrave(report) = op.detail else {
            panic!("reported {:?}", op.detail);
        };
        assert_eq!((report.curves, report.closed), (1, 0));
        let cuts: Vec<_> = op
            .path
            .moves()
            .iter()
            .filter(|m| matches!(m.role, MoveRole::Cut | MoveRole::Ramp))
            .collect();
        let deepest = cuts.iter().map(|m| m.to.z).fold(f64::MAX, f64::min);
        assert!((deepest - 2.0).abs() < 1e-9, "{pattern:?}: {deepest}");
        let arcs = cuts
            .iter()
            .filter(|m| matches!(m.kind, MoveKind::Arc(_)))
            .count();
        assert!(arcs >= 3, "{pattern:?}: {arcs} arcs");
        let lifts = op
            .path
            .moves()
            .iter()
            .filter(|m| m.role == MoveRole::Retract)
            .count();
        match pattern {
            // Out once, at the end.
            SlotPattern::Bidirectional => {
                assert_eq!(report.passes, 4, "three ramps and the last");
                assert!(lifts <= 1, "{lifts} lifts");
            }
            // Three levels, each its own pass, lifted out of and ramped into
            // along the slot, at its start.
            SlotPattern::Directional => {
                assert_eq!(report.passes, 3);
                assert!(lifts >= 2, "{lifts} lifts");
                assert_eq!(op.links.ramps, 3, "{:?}", op.links);
            }
        }
        assert!(op.sim.removed_mm3 > 0.0);
        assert!(result.is_clean(), "{pattern:?}: {:?}", result.warnings());
        assert_eq!(result.verify.gouged, 0, "{pattern:?}");

        // A V-bit's width changes with depth: no slot.
        job.tools.get_mut("drawn").unwrap().kind = ToolKind::Cone;
        job.tools.get_mut("drawn").unwrap().angle = Some(90.0);
        let refused = job.check_setup(0).unwrap_err().to_string();
        assert!(refused.contains("end mill its width"), "{refused}");
    }
}

/// The plate's hole's top rim deburred with a 90° V-bit, a millimetre wide
/// and the tip half a millimetre past: the tool's centre goes round inside
/// the hole, away from the plate, half a millimetre in from the rim, its tip
/// a millimetre and a half down, round it as arcs; what it cuts is meant,
/// so the run verifies. Wider than the cone reaches is refused.
#[test]
fn a_chamfer_goes_beside_its_edge_as_deep_as_it_is_wide() {
    use crate::job::{ChamferSide, FeatureRef, ToolKind};
    use crate::path::toolpath::{MoveKind, MoveRole};
    let body = Body::derived(synthetic::plate_with_hole(20.0, 5.0, 4.0, 48));
    let chamfer = StrategySpec::Chamfer {
        width: 1.0,
        tip_offset: 0.5,
        side: ChamferSide::Auto,
    };
    let mut job = sheet_job(chamfer);
    job.tools.insert(
        "vbit".into(),
        crate::job::ToolSpec {
            kind: ToolKind::Cone,
            diameter: 6.0,
            angle: Some(90.0),
            number: 5,
            ..crate::job::ToolSpec::default()
        },
    );
    let op = &mut job.setups[0].operations[0];
    op.tool = Some("vbit".into());
    op.selection.edges = vec![FeatureRef {
        model: "dome".into(),
        index: u32::MAX,
        at: [4.0, 0.0, 5.0],
    }];
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    job.check_setup(0).unwrap();
    let result = run_setup(&job, 0, &bodies).unwrap();
    let op = &result.operations[0];
    assert!(op.warnings.is_empty(), "{:?}", op.warnings);
    let mut at = op.path.moves()[0].to;
    let mut arcs = 0;
    for m in op.path.moves() {
        if matches!(m.role, MoveRole::Cut | MoveRole::Ramp)
            && let MoveKind::Arc(a) = m.kind
        {
            arcs += 1;
            assert!(a.centre.truncate().length() < 1e-6, "{a:?}");
            assert!((a.radius(at) - 3.5).abs() < 1e-6, "{}", a.radius(at));
        }
        at = m.to;
    }
    assert!(arcs >= 4, "{arcs}");
    let deepest = op
        .path
        .moves()
        .iter()
        .filter(|m| m.role == MoveRole::Cut)
        .map(|m| m.to.z)
        .fold(f64::MAX, f64::min);
    assert!((deepest - 3.5).abs() < 1e-9, "{deepest}");
    assert!(op.sim.removed_mm3 > 0.0);
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);

    // As drawn: the run's own centres, round inside the rim.
    let prepared = crate::setup::prepare(&job, 0, &bodies).unwrap();
    let (centres, place) =
        chamfer_centres(&job, &job.setups[0].operations[0], &prepared, &bodies).unwrap();
    assert_eq!(centres.len(), 1);
    assert!((place.reach - 1.5).abs() < 1e-12);
    for p in centres[0].points(0.001) {
        assert!((p.truncate().length() - 3.5).abs() < 1e-6, "{p}");
    }

    // A 3 mm cone has 1.5 mm of flank: a 1.2 mm chamfer half a millimetre
    // down needs 1.7.
    let small = job.tools.get_mut("vbit").unwrap();
    small.diameter = 3.0;
    if let StrategySpec::Chamfer { width, .. } = &mut job.setups[0].operations[0].strategy {
        *width = 1.2;
    }
    let refused = job.check_setup(0).unwrap_err().to_string();
    assert!(refused.contains("more of the cone"), "{refused}");
    // A flat end mill has no flank.
    job.tools.get_mut("vbit").unwrap().kind = ToolKind::Flat;
    let refused = job.check_setup(0).unwrap_err().to_string();
    assert!(refused.contains("chamfer mill or a V-bit"), "{refused}");
}

/// Where the material went down between two keyframes, seen from above.
fn lowered_cells(result: &SetupResult, op: usize) -> Vec<DVec2> {
    let before = result.keyframes[op].unpack().unwrap();
    let after = result.keyframes[op + 1].unpack().unwrap();
    let (before, after) = (before.field(), after.field());
    let mut out = Vec::new();
    for iy in 0..before.ny() {
        for ix in 0..before.nx() {
            if let (Some(b), Some(a)) = (before.get(ix, iy), after.get(ix, iy))
                && a < b - 1e-3
            {
                out.push(before.position(ix, iy).unwrap());
            }
        }
    }
    out
}

/// A circle engraved, arrayed three times along X and round a centre, and
/// mirrored: every copy is cut into the material, so the simulation sees
/// them all and they verify; the mirrored one goes round the same way as
/// the original, turned round. Copies off the stock are said.
#[test]
fn a_pattern_cuts_every_copy_and_a_mirror_keeps_the_direction() {
    use crate::job::{Linear, Mirror, MirrorAxis, Pattern, Polar, TraceDirection};
    use crate::path::toolpath::{MoveKind, MoveRole};
    let engrave = StrategySpec::Engrave {
        depth: 0.5,
        direction: TraceDirection::Climb,
        reverse: false,
    };
    let circle = || vec![drawn_circle(-12.0, -8.0, 3.0)];
    let (_, one) = drawn_on_a_plate(engrave, flat(1.0), circle());
    let alone = one.operations[0].sim.removed_mm3;

    // Three along X, 12 apart.
    let rows = Pattern {
        linear: Some(Linear {
            x_count: 3,
            x_spacing: 12.0,
            y_count: 1,
            y_spacing: 0.0,
        }),
        ..Pattern::default()
    };
    let (_, result) = drawn_on_a_plate_with(engrave, flat(1.0), circle(), |op| op.transform = rows);
    let op = &result.operations[0];
    assert!(
        (op.sim.removed_mm3 / alone - 3.0).abs() < 0.05,
        "{} {alone}",
        op.sim.removed_mm3
    );
    assert_eq!(op.passes, 3 * one.operations[0].passes);
    let cells = lowered_cells(&result, 0);
    for x in [-12.0, 0.0, 12.0] {
        let near = cells
            .iter()
            .filter(|p| (p.distance(dvec2(x, -8.0)) - 3.0).abs() < 1.0)
            .count();
        assert!(near > 20, "nothing cut round x = {x}");
    }
    assert!(result.is_clean(), "{:?}", result.warnings());
    assert_eq!(result.verify.gouged, 0);

    // Four round the plate's middle.
    let round = Pattern {
        polar: Some(Polar {
            count: 4,
            angle: 360.0,
            centre: [0.0, 0.0],
        }),
        ..Pattern::default()
    };
    let (_, result) =
        drawn_on_a_plate_with(engrave, flat(1.0), circle(), |op| op.transform = round);
    let op = &result.operations[0];
    assert!((op.sim.removed_mm3 / alone - 4.0).abs() < 0.05);
    assert!(result.is_clean(), "{:?}", result.warnings());

    // Mirrored about Y through the middle: beside it at x = 12, and round
    // the same way as the original, clockwise to climb.
    let mirror = Pattern {
        mirror: Some(Mirror {
            axis: MirrorAxis::Y,
            through: [0.0, 0.0],
        }),
        ..Pattern::default()
    };
    let (_, result) =
        drawn_on_a_plate_with(engrave, flat(1.0), circle(), |op| op.transform = mirror);
    let op = &result.operations[0];
    assert!((op.sim.removed_mm3 / alone - 2.0).abs() < 0.05);
    let level_arcs = |right: bool| -> Vec<f64> {
        op.path
            .moves()
            .iter()
            .filter(|m| m.role == MoveRole::Cut && (m.to.x > 0.0) == right)
            .filter_map(|m| match m.kind {
                MoveKind::Arc(a) => Some(a.sweep),
                MoveKind::Linear => None,
            })
            .collect()
    };
    let (left, right) = (level_arcs(false), level_arcs(true));
    assert!(!left.is_empty() && !right.is_empty());
    assert!(
        left.iter().chain(&right).all(|s| *s < 0.0),
        "{left:?} {right:?}"
    );
    assert!(op.warnings.is_empty(), "{:?}", op.warnings);
    assert!(result.is_clean(), "{:?}", result.warnings());

    // Three along X 20 apart: the last off the plate, and said.
    let far = Pattern {
        linear: Some(Linear {
            x_count: 3,
            x_spacing: 20.0,
            y_count: 1,
            y_spacing: 0.0,
        }),
        ..Pattern::default()
    };
    let (mut job, result) =
        drawn_on_a_plate_with(engrave, flat(1.0), circle(), |op| op.transform = far);
    let warned = &result.operations[0].warnings;
    assert!(
        warned
            .iter()
            .any(|w| w.contains("1 of the pattern's 3 copies go outside the stock")),
        "{warned:?}"
    );

    // No copies at all is no pattern.
    if let Some(l) = &mut job.setups[0].operations[0].transform.linear {
        l.x_count = 0;
    }
    let refused = job.check_setup(0).unwrap_err().to_string();
    assert!(refused.contains("x_count"), "{refused}");
}

/// A plate 80 by 30 and 1 thick, under a block 5 deeper, and a clearing of
/// `strategy` with the 6 mm flat for each of `ops`: kept to the 14 mm
/// square about `x` on the plate's middle line, with its pattern.
fn squares_in_a_block(ops: &[(f64, crate::job::Pattern)], strategy: StrategySpec) -> SetupResult {
    use crate::job::{Curve, Segment, StockSpec};
    let body = Body::derived(synthetic::cuboid(
        DVec3::new(-40.0, -15.0, 0.0),
        DVec3::new(40.0, 15.0, 1.0),
    ));
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.setups[0].operations.retain(|op| op.name == "rough");
    job.stocks[0].spec = StockSpec::AroundModels {
        models: vec![],
        margin: [0.0, 0.0, 5.0],
        below: 0.0,
    };
    let rough = job.setups[0].operations.remove(0);
    for (x, pattern) in ops {
        let mut op = rough.clone();
        op.strategy = strategy;
        op.passes.stepover = if op.strategy.uses().load { 0.6 } else { 2.0 };
        op.passes.stock_to_leave = Default::default();
        op.boundary.kind = Boundary::Selection;
        // Down to the plate: past its edge the tool would go on down, and
        // a copy over the plate with it.
        op.heights.bottom = Some(Height {
            reference: HeightRef::ModelTop,
            offset: 0.0,
        });
        let corner = |dx: f64, dy: f64| [x + dx, dy, 6.0];
        op.selection.curves = vec![Curve {
            start: corner(-7.0, -7.0),
            segments: [(7.0, -7.0), (7.0, 7.0), (-7.0, 7.0), (-7.0, -7.0)]
                .into_iter()
                .map(|(dx, dy)| Segment::Line { to: corner(dx, dy) })
                .collect(),
        }];
        op.transform = *pattern;
        job.setups[0].operations.push(op);
    }
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    let result = run_setup(&job, 0, &bodies).unwrap();
    let tool = cutting_tool(&job, &job.setups[0].operations[0]).unwrap();
    assert!((tool.cutter_radius() - 3.0).abs() < 1e-12);
    result
}

/// How the 6 mm flat's edge meets the material along `path`, from `stock`,
/// cut as the path goes.
struct Engaged {
    /// The most of it in material at any step fed along at a level, cutting
    /// or between cuts, as an angle.
    widest: f64,
    /// How much of what it met, all steps together, was on its right, as a
    /// clockwise spindle climbs, and how much on its left.
    right: usize,
    left: usize,
}

fn engagement(
    mut stock: camcraft_sim::stock::Stock,
    path: &[crate::path::toolpath::Move],
) -> Engaged {
    use crate::cutter::allowance::{Allowance, EffectiveTool};
    use crate::path::simadapt::sim_moves_of;
    use crate::path::toolpath::{MoveKind, MoveRole};
    use camcraft_sim::stock::{MoveClass, SimMove};
    use std::f64::consts::PI;
    let base = flat(6.0).build().unwrap();
    let tool = EffectiveTool::of(&base, Allowance::new(0.0, 0.0).unwrap()).unwrap();
    let from = |p: DVec3| SimMove {
        to: p,
        class: MoveClass::Rapid,
        arc: None,
    };
    let samples = 90;
    let mut out = Engaged {
        widest: 0.0,
        right: 0,
        left: 0,
    };
    let Some(mut at) = path.first().map(|m| m.to) else {
        return out;
    };
    for m in path {
        let level = (m.to.z - at.z).abs() < 1e-9;
        if m.role != MoveRole::Rapid && level && m.kind == MoveKind::Linear {
            let steps = camcraft_geom::steps::bounded_count((at.distance(m.to) / 0.1).ceil());
            let mut a = at;
            for i in 1..=steps {
                let b = at.lerp(m.to, f64::from(i) / f64::from(steps));
                // The front half of the tool's edge, just past it, from its
                // right to its left: what of it is in material it takes, a
                // slot all of it.
                let ahead = (b - a).truncate().normalize_or_zero();
                let inside: Vec<i32> = (0..=samples)
                    .filter(|k| {
                        let t = PI * (f64::from(*k) / f64::from(samples) - 0.5);
                        let p = b.truncate() + DVec2::from_angle(t).rotate(ahead) * 3.05;
                        stock.height_at(p.x, p.y).is_some_and(|h| h > b.z + 0.05)
                    })
                    .map(|k| k - samples / 2)
                    .collect();
                let e =
                    PI * f64::from(u32::try_from(inside.len()).unwrap()) / f64::from(samples + 1);
                out.widest = out.widest.max(e);
                out.right += inside.iter().filter(|k| **k < 0).count();
                out.left += inside.iter().filter(|k| **k > 0).count();
                let mut step = sim_moves_of(&[crate::path::toolpath::Move { to: b, ..*m }]);
                step.insert(0, from(a));
                stock.sweep(&step, &tool);
                a = b;
            }
        } else {
            let mut step = sim_moves_of(std::slice::from_ref(m));
            step.insert(0, from(at));
            stock.sweep(&step, &tool);
        }
        at = m.to;
    }
    out
}

fn adaptive() -> StrategySpec {
    StrategySpec::Adaptive {
        direction: crate::job::CutDirection::Climb,
        flat_areas: true,
        rest: None,
    }
}

/// Three adaptive pockets in a row 26 apart, rest machining, the first over
/// the block's edge, the last where an operation before already cleared:
/// each copy is worked out against the material where it is. The middle
/// one, all in the block, goes down a helix where the first came in from the
/// side, so is no copy of it and never takes the tool's whole width; the
/// last finds nothing to clear, where a copy of the first would cut air.
/// Not rest machining, the copies clear the blocks, less what each other
/// cut: the last clears its square again, and the original is as it is
/// alone.
#[test]
fn a_pattern_of_adaptive_clearing_clears_the_material_where_each_copy_is() {
    use crate::job::{Linear, Pattern};
    use crate::path::toolpath::{Move, MoveRole, Toolpath};
    let rows = Pattern {
        linear: Some(Linear {
            x_count: 3,
            x_spacing: 26.0,
            y_count: 1,
            y_spacing: 0.0,
        }),
        ..Pattern::default()
    };
    let resting = StrategySpec::Adaptive {
        direction: crate::job::CutDirection::Climb,
        flat_areas: true,
        rest: Some(crate::job::Rest::default()),
    };
    let earlier = (12.0, Pattern::default());
    let patterned = squares_in_a_block(&[earlier, (-40.0, rows)], resting);
    let alone = squares_in_a_block(&[earlier, (-40.0, Pattern::default())], resting);
    let before = patterned.keyframes[1].unpack().unwrap();
    let op = &patterned.operations[1];
    assert!(result_is_clean(&patterned), "{:?}", patterned.warnings());

    // The first copy is the original, worked out alone.
    let original = &alone.operations[1].path;
    let near = |m: &&Move, x: f64| m.role == MoveRole::Cut && (m.to.x - x).abs() < 10.5;
    let cuts_near = |path: &crate::path::toolpath::Toolpath, x: f64| -> Vec<DVec3> {
        path.moves()
            .iter()
            .filter(|m| near(m, x))
            .map(|m| m.to)
            .collect()
    };
    assert_eq!(cuts_near(&op.path, -40.0), cuts_near(original, -40.0));
    assert!(!cuts_near(original, -40.0).is_empty());

    // The middle one is no copy of it.
    let moved: Vec<DVec3> = cuts_near(original, -40.0)
        .iter()
        .map(|p| *p + DVec3::new(26.0, 0.0, 0.0))
        .collect();
    let middle = cuts_near(&op.path, -14.0);
    assert!(!middle.is_empty());
    assert_ne!(middle, moved);
    assert!(
        op.links.helixes > alone.operations[1].links.helixes,
        "{:?}",
        op.links
    );

    // Held to its load: never more of the tool's edge in material than the
    // original takes at its own place, where it goes round the square's
    // corners against the block left outside it, and far from the whole
    // front of it, a slot, which a copy of the original takes.
    let own = engagement(before.clone(), original.moves()).widest;
    let widest = engagement(before.clone(), op.path.moves()).widest;
    assert!(own < 1.7, "the original engaged {own} rad");
    assert!(
        widest < own + 0.25,
        "engaged {widest} rad, the original {own}"
    );
    let mut rigid = Toolpath::new();
    for m in original.moves() {
        rigid
            .push(Move {
                to: m.to + DVec3::new(26.0, 0.0, 0.0),
                kind: match m.kind {
                    crate::path::toolpath::MoveKind::Arc(mut a) => {
                        a.centre += DVec3::new(26.0, 0.0, 0.0);
                        crate::path::toolpath::MoveKind::Arc(a)
                    }
                    k => k,
                },
                ..*m
            })
            .unwrap();
    }
    let slotting = engagement(before, rigid.moves()).widest;
    assert!(slotting > 2.5, "a copy engaged only {slotting} rad");

    // The last was cleared before: nothing there to cut, and said.
    assert!(cuts_near(&op.path, 12.0).is_empty());
    assert!(
        op.warnings
            .iter()
            .any(|w| w.contains("1 of the pattern's 3 copies found nothing to clear")),
        "{:?}",
        op.warnings
    );

    let blocks = squares_in_a_block(&[earlier, (-40.0, rows)], adaptive());
    let unpatterned = squares_in_a_block(&[earlier, (-40.0, Pattern::default())], adaptive());
    let op = &blocks.operations[1];
    assert!(result_is_clean(&blocks), "{:?}", blocks.warnings());
    assert_eq!(
        cuts_near(&op.path, -40.0),
        cuts_near(&unpatterned.operations[1].path, -40.0)
    );
    assert!(cuts_near(&op.path, 12.0).len() * 2 > cuts_near(&op.path, -14.0).len());
}

fn result_is_clean(result: &SetupResult) -> bool {
    result.verify.gouged == 0
        && result
            .operations
            .iter()
            .all(|op| op.sim.rapid_contacts == 0 && !op.path.has_unplanned_links())
}

/// Mirrored, an adaptive clearing is worked out mirrored, so the copy climbs
/// as the original does: the material it meets is on its right, as a
/// clockwise spindle's climbing cut has it. The original's passes turned
/// over would have it on the left.
#[test]
fn a_mirrored_adaptive_clearing_climbs() {
    use crate::job::{Mirror, MirrorAxis, Pattern};
    use crate::path::toolpath::MoveRole;
    let mirror = Pattern {
        mirror: Some(Mirror {
            axis: MirrorAxis::Y,
            through: [0.0, 0.0],
        }),
        ..Pattern::default()
    };
    let result = squares_in_a_block(&[(-14.0, mirror)], adaptive());
    let op = &result.operations[0];
    assert!(result_is_clean(&result), "{:?}", result.warnings());
    assert!(
        !op.warnings.iter().any(|w| w.contains("turned round")),
        "{:?}",
        op.warnings
    );
    let before = result.keyframes[0].unpack().unwrap();
    // The copy alone, on the right, from where the original left it.
    let start = op
        .path
        .moves()
        .iter()
        .position(|m| m.role == MoveRole::Cut && m.to.x > 0.0)
        .unwrap();
    let original = engagement(before.clone(), op.path.moves().get(..start).unwrap());
    let both = engagement(before, op.path.moves());
    let copy = Engaged {
        widest: both.widest,
        right: both.right - original.right,
        left: both.left - original.left,
    };
    assert!(
        original.right > 10 * original.left.max(1),
        "{} {}",
        original.right,
        original.left
    );
    assert!(
        copy.right > 10 * copy.left.max(1),
        "{} {}",
        copy.right,
        copy.left
    );
    assert!(copy.widest < 1.7, "{}", copy.widest);
}

/// Pocket clearing patterned over a square an operation before cleared:
/// that copy has next to nothing to cut, the ring along the walls it
/// touches, where a copy of the original would clear it all again.
#[test]
fn a_pattern_of_pocket_clearing_skips_what_is_cleared() {
    use crate::job::{Linear, Pattern};
    use crate::path::toolpath::MoveRole;
    let rows = Pattern {
        linear: Some(Linear {
            x_count: 3,
            x_spacing: 26.0,
            y_count: 1,
            y_spacing: 0.0,
        }),
        ..Pattern::default()
    };
    let clearing = StrategySpec::PocketClearing {
        min_ring_length: 2.0,
        conventional: false,
        flat_areas: false,
    };
    let result = squares_in_a_block(&[(14.0, Pattern::default()), (-38.0, rows)], clearing);
    let op = &result.operations[1];
    assert!(result_is_clean(&result), "{:?}", result.warnings());
    // How far it cuts about `x`.
    let near = |x: f64| -> f64 {
        op.path
            .moves()
            .windows(2)
            .filter(|w| w[1].role == MoveRole::Cut && (w[1].to.x - x).abs() < 10.0)
            .map(|w| w[0].to.distance(w[1].to))
            .sum()
    };
    assert!(near(-38.0) > 0.0 && near(-12.0) > 0.0);
    assert!(
        near(14.0) * 2.0 < near(-12.0),
        "{} {}",
        near(14.0),
        near(-12.0)
    );
}

/// The plate of [`squares_in_a_block`], cleared in its 20 mm square by
/// adaptive clearing with the 6 mm flat kept inside it, then by a 2 mm flat
/// kept inside it, rest machining as `rest` says.
fn cleared_again(rest: Option<crate::job::Rest>) -> SetupResult {
    use crate::job::{Containment, Curve, Segment, StockSpec};
    let body = Body::derived(synthetic::cuboid(
        DVec3::new(-40.0, -15.0, 0.0),
        DVec3::new(40.0, 15.0, 1.0),
    ));
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.setups[0].operations.retain(|op| op.name == "rough");
    job.stocks[0].spec = StockSpec::AroundModels {
        models: vec![],
        margin: [0.0, 0.0, 5.0],
        below: 0.0,
    };
    job.tools.insert("small".into(), flat(2.0));
    let mut big = job.setups[0].operations.remove(0);
    big.strategy = adaptive();
    big.passes.stepover = 0.6;
    big.passes.stock_to_leave = Default::default();
    big.boundary.kind = Boundary::Selection;
    big.boundary.containment = Containment::Inside;
    big.heights.bottom = Some(Height {
        reference: HeightRef::ModelTop,
        offset: 0.0,
    });
    let corner = |x: f64, y: f64| [x, y, 6.0];
    big.selection.curves = vec![Curve {
        start: corner(-10.0, -10.0),
        segments: [(10.0, -10.0), (10.0, 10.0), (-10.0, 10.0), (-10.0, -10.0)]
            .into_iter()
            .map(|(x, y)| Segment::Line { to: corner(x, y) })
            .collect(),
    }];
    let mut small = big.clone();
    small.name = "small".into();
    small.tool = Some("small".into());
    small.passes.stepover = 0.2;
    small.strategy = StrategySpec::Adaptive {
        direction: crate::job::CutDirection::Climb,
        flat_areas: true,
        rest,
    };
    job.setups[0].operations = vec![big, small];
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    run_setup(&job, 0, &bodies).unwrap()
}

/// How far `op` cuts.
fn cut_length(op: &OperationResult) -> f64 {
    op.path
        .moves()
        .windows(2)
        .filter(|w| w[1].role == crate::path::toolpath::MoveRole::Cut)
        .map(|w| w[0].to.distance(w[1].to))
        .sum()
}

/// A small adaptive clearing after a big one, rest machining: it clears the
/// corners the big tool could not reach, not the whole square again, and
/// takes what is there; left to clear the stock, it cuts the square over.
/// What is thinner than the adjustment is left alone.
#[test]
fn rest_machining_clears_only_what_the_big_tool_left() {
    use crate::job::Rest;
    let whole = cleared_again(None);
    let rest = cleared_again(Some(Rest::default()));
    let adjusted = cleared_again(Some(Rest {
        adjustment: 1.5,
        ..Rest::default()
    }));
    for r in [&whole, &rest, &adjusted] {
        assert!(result_is_clean(r), "{:?}", r.warnings());
    }
    let (again, corners) = (&whole.operations[1], &rest.operations[1]);
    let (all, some) = (cut_length(again), cut_length(corners));
    assert!(
        some > 0.0 && some < 0.25 * all,
        "rest {some} mm against {all}"
    );
    // The corners: a 3 mm radius left where a 1 mm one fits, at each of the
    // square's four, two levels down. Each corner is (3² − π·3²/4) less the
    // same at 1 mm, about 1.9 mm² less 0.2, at most 5 mm deep.
    assert!(
        corners.sim.removed_mm3 > 4.0 * 1.0 * 4.0,
        "{}",
        corners.sim.removed_mm3
    );
    assert!(
        corners.sim.removed_mm3 < 4.0 * 2.0 * 5.0,
        "{}",
        corners.sim.removed_mm3
    );
    // Every cut near a corner, none down the middle of a side.
    for m in corners.path.moves() {
        if m.role == crate::path::toolpath::MoveRole::Cut {
            let p = m.to.truncate().abs();
            assert!(p.x > 5.0 && p.y > 5.0, "a rest cut at {}", m.to);
        }
    }
    // A corner left by the 6 mm flat is 3 (√2 − 1), 1.24 mm, across at its
    // thickest: thinner than 1.5, so taken as cleared.
    let thin = cut_length(&adjusted.operations[1]);
    assert!(thin < 0.5 * some, "adjusted {thin} mm against {some}");
}
