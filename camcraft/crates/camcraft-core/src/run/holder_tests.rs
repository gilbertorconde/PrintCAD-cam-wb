//! The tool above its flutes, its shank and its holder, met by the material.

use super::*;
use crate::fixture::JOB;
use crate::job::{Holder, HolderSegment, Piece, StockSpec, StrategySpec};
use camcraft_geom::synthetic;
use camcraft_geom::topology::Body;

/// A plate 30 mm thick with a hole 12 mm across through it, its stock the
/// plate itself, cleared in Z levels with a 6 mm flat with 15 mm flutes, a
/// 6 mm shank and `stickout` out of a holder 28 mm across.
fn deep_hole(stickout: f64) -> SetupResult {
    deep_hole_with(Some(stickout))
}

/// [`deep_hole`], or with no shank or holder given when `stickout` is not.
fn deep_hole_with(stickout: Option<f64>) -> SetupResult {
    let body = Body::derived(synthetic::plate_with_hole(20.0, 30.0, 6.0, 64));
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.setups[0].operations.retain(|op| op.name == "rough");
    job.stocks[0].spec = StockSpec::AroundModels {
        models: vec![],
        margin: [0.0, 0.0, 0.0],
        below: 0.0,
    };
    job.tools.insert("rough".into(), deep_hole_spec(stickout));
    let op = &mut job.setups[0].operations[0];
    op.strategy = StrategySpec::named("pocket-clearing").unwrap();
    op.passes.stock_to_leave = Default::default();
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    run_setup(&job, 0, &bodies).unwrap()
}

/// Down a hole deeper than the tool sticks out, the holder meets the plate
/// round it, and the stickout said to clear it is the hole's depth below
/// the plate's top at the deepest the tool goes.
#[test]
fn a_short_stickout_down_a_deep_hole_meets_the_holder() {
    let result = deep_hole(20.0);
    let op = &result.operations[0];
    let c = &op.clashes;
    assert!(c.checked);
    let holder = c
        .found
        .iter()
        .find(|k| k.piece == Piece::Holder(0))
        .unwrap_or_else(|| panic!("{c:?}"));
    assert!(holder.part, "{holder:?}");
    assert!(holder.moves > 0);
    // The deepest the tip goes, and the plate's top 30 mm up.
    let deepest = op
        .path
        .moves()
        .iter()
        .map(|m| m.to.z)
        .fold(f64::MAX, f64::min);
    let needs = c.needs_stickout.unwrap();
    let want = 30.0 - deepest;
    assert!((needs - want).abs() < 0.1, "needs {needs}, want {want}");
    // The first move it meets the plate on is more than 10 mm down, the
    // holder's foot below the top.
    let first = op.path.moves()[holder.first_move].to;
    assert!(first.z < 30.0 - 20.0, "first at {first}");
    // Nothing else meets it: the flutes take only a level at a time.
    assert!(
        c.found.iter().all(|k| matches!(k.piece, Piece::Holder(_))),
        "{c:?}"
    );
    assert!(!op.is_clean());
    let line = holder.line(c.needs_stickout);
    assert!(
        line.contains("holder segment 1 (Ø28.00) meets the part"),
        "{line}"
    );
    assert!(
        line.contains(&format!(
            "needs {:.1} mm stickout",
            (needs * 10.0).ceil() / 10.0
        )),
        "{line}"
    );
    assert!(op.warnings.iter().any(|w| w.contains("holder segment 1")));
}

/// Out far enough, the same tool clears the plate, and says nothing.
#[test]
fn a_long_stickout_clears_the_hole() {
    let result = deep_hole(31.0);
    let op = &result.operations[0];
    assert!(op.clashes.checked);
    assert!(op.clashes.found.is_empty(), "{:?}", op.clashes);
    assert_eq!(op.clashes.needs_stickout, None);
    assert!(result.is_clean(), "{:?}", result.warnings());
}

/// What a tool's lengths say stands above its flutes, and what they do not.
#[test]
fn the_rings_above_the_flutes_follow_the_tools_lengths() {
    let mut t = crate::job::ToolSpec {
        diameter: 3.0,
        ..Default::default()
    };
    let none = t.above_flutes();
    assert!(none.body.rings.is_empty());
    assert_eq!(none.unchecked.len(), 2, "{:?}", none.unchecked);
    t.flute_length = Some(8.0);
    t.shoulder_length = Some(12.0);
    t.shank_diameter = Some(6.0);
    t.stickout = Some(25.0);
    t.holder = Some(Holder {
        name: String::new(),
        segments: vec![HolderSegment {
            diameter: 20.0,
            length: 10.0,
            top_diameter: Some(30.0),
        }],
    });
    let a = t.above_flutes();
    assert_eq!(a.pieces, [Piece::Body, Piece::Shank, Piece::Holder(0)]);
    let r = &a.body.rings;
    assert_eq!((r[0].from, r[0].to, r[0].radius_from), (8.0, 12.0, 1.5));
    assert_eq!((r[1].from, r[1].to, r[1].radius_from), (12.0, 25.0, 3.0));
    assert_eq!((r[2].from, r[2].to, r[2].radius_to), (25.0, 35.0, 15.0));
    assert_eq!(a.stickout, Some(25.0));
    assert!(a.unchecked.is_empty());
    // A cutter with no shank of its own is as wide all the way up.
    t.shank_diameter = None;
    t.holder = None;
    let a = t.above_flutes();
    assert_eq!(a.pieces, [Piece::Body]);
    assert_eq!(a.body.rings[0].to, 25.0);
    // A thread mill's neck, behind its teeth, goes into the hole: its shank
    // is checked from its shoulder, and nothing between.
    let mill = crate::job::ToolSpec {
        kind: crate::job::ToolKind::ThreadMill,
        diameter: 4.8,
        flute_length: Some(2.0),
        shoulder_length: Some(14.0),
        shank_diameter: Some(6.0),
        stickout: Some(30.0),
        ..Default::default()
    };
    let a = mill.above_flutes();
    assert_eq!(a.pieces, [Piece::Shank]);
    let r = &a.body.rings[0];
    assert_eq!((r.from, r.to, r.radius_from), (14.0, 30.0, 3.0));
    let no_neck = crate::job::ToolSpec {
        shoulder_length: None,
        ..mill
    }
    .above_flutes();
    assert!(no_neck.body.rings.is_empty());
    assert!(
        no_neck.unchecked.iter().any(|u| u.contains("neck")),
        "{:?}",
        no_neck.unchecked
    );
}

/// A slot 10 mm wide and 28 mm deep across a block 40 mm square, roughed
/// out, then finished in parallel passes across it with the 4 mm ball out
/// 20 mm from a holder 28 mm across, `avoid` asking it to keep the holder
/// off the part.
fn finished_slot(avoid: bool) -> SetupResult {
    use glam::dvec3;
    let floor = synthetic::cuboid(dvec3(-20.0, -20.0, 0.0), dvec3(20.0, 20.0, 2.0));
    let left = synthetic::cuboid(dvec3(-20.0, -20.0, 2.0), dvec3(-5.0, 20.0, 30.0));
    let right = synthetic::cuboid(dvec3(5.0, -20.0, 2.0), dvec3(20.0, 20.0, 30.0));
    let mesh = camcraft_geom::mesh::TriMesh::merged(&[&floor, &left, &right]).unwrap();
    let body = Body::derived(mesh);
    let mut job: Job = toml::from_str(JOB).unwrap();
    job.stocks[0].spec = StockSpec::AroundModels {
        models: vec![],
        margin: [0.0, 0.0, 0.0],
        below: 0.0,
    };
    job.setups[0].operations[0].passes.stock_to_leave = Default::default();
    let ball = job.tools.get_mut("finish").unwrap();
    ball.flute_length = Some(10.0);
    ball.shank_diameter = Some(4.0);
    ball.stickout = Some(20.0);
    ball.holder = Some(Holder {
        name: "ER20".into(),
        segments: vec![HolderSegment::cylinder(28.0, 19.0)],
    });
    let finish = &mut job.setups[0].operations[1];
    finish.passes.stepover = 2.0;
    finish.passes.use_holder = avoid;
    let bodies: Bodies<'_> = [("dome", &body)].into_iter().collect();
    run_setup(&job, 0, &bodies).unwrap()
}

/// Down the slot, the holder meets its walls' tops; asked to, the finish
/// leaves out the cuts where it would, and then meets the part nowhere.
#[test]
fn a_finish_keeps_its_holder_off_the_part_when_asked() {
    let hit = |r: &SetupResult| {
        r.operations[1]
            .clashes
            .found
            .iter()
            .any(|c| matches!(c.piece, Piece::Holder(_)) && c.part)
    };
    let plain = finished_slot(false);
    assert!(hit(&plain), "{:?}", plain.operations[1].clashes);
    assert_eq!(plain.operations[1].clashes.trimmed, 0);
    let kept = finished_slot(true);
    let finish = &kept.operations[1];
    assert!(finish.clashes.trimmed > 0, "{:?}", finish.clashes);
    assert!(!hit(&kept), "{:?}", finish.clashes);
    assert!(!finish.path.has_unplanned_links());
    assert!(finish.warnings.iter().any(|w| w.contains("left out")));
}

/// How long the deep hole takes to run with nothing above the flutes to
/// check, with a holder that clears it, and with one that meets it on most
/// moves. Run by hand: `cargo test --release -p camcraft-core
/// holder_timing -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement, not a test"]
fn holder_timing() {
    for (what, stickout) in [
        ("nothing to check", None),
        ("clear", Some(31.0)),
        ("meets it", Some(20.0)),
    ] {
        let t = std::time::Instant::now();
        let r = deep_hole_with(stickout);
        let run = t.elapsed().as_secs_f64();
        let op = &r.operations[0];
        // The operation's path cut again into the material it started from,
        // alone and watched, so the run's other work does not hide it.
        let moves = crate::path::simadapt::sim_moves(&op.path);
        let spec = deep_hole_spec(stickout);
        let tool = crate::cutter::allowance::EffectiveTool::of(
            &spec.build().unwrap(),
            crate::cutter::allowance::Allowance::none(),
        )
        .unwrap();
        let mut plain = r.keyframes[0].unpack().unwrap();
        let t = std::time::Instant::now();
        plain.sweep(&moves, &tool);
        let swept = t.elapsed().as_secs_f64();
        let mut watched = r.keyframes[0].unpack().unwrap();
        let body = spec.above_flutes().body;
        let mut watch = watched.watch(camcraft_sim::stock::Reach::new(&body, 0.05, None));
        let t = std::time::Instant::now();
        watched.sweep_watching(&moves, &tool, &mut watch, &mut |_, _| {});
        let checked = t.elapsed().as_secs_f64();
        println!(
            "{what}: run {run:.2} s; the path swept {swept:.3} s, swept and checked {checked:.3} s; \
             {} moves, {} cells across, {:?}",
            op.path.len(),
            plain.field().nx(),
            op.clashes.found.iter().map(|c| c.moves).collect::<Vec<_>>()
        );
    }
}

/// The tool [`deep_hole_with`] cuts with.
fn deep_hole_spec(stickout: Option<f64>) -> crate::job::ToolSpec {
    let mut job: Job = toml::from_str(JOB).unwrap();
    let mut tool = job.tools.remove("rough").unwrap();
    if let Some(stickout) = stickout {
        tool.flute_length = Some(15.0);
        tool.shank_diameter = Some(6.0);
        tool.stickout = Some(stickout);
        tool.holder = Some(Holder {
            name: "ER20".into(),
            segments: vec![
                HolderSegment::cylinder(28.0, 19.0),
                HolderSegment::cylinder(42.0, 15.0),
            ],
        });
    }
    tool
}
