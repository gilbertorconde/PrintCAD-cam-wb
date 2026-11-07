use super::*;
use crate::path::toolpath::{Move, MoveKind};
use crate::strategy::drilling::Hole;
use glam::dvec2;

/// A hole, or a boss, `radius` round, at (5, 5), from 0 down to -10.
fn round(radius: f64, boss: bool) -> Round {
    Round {
        hole: Hole {
            at: dvec2(5.0, 5.0),
            top: 0.0,
            bottom: -10.0,
            radius,
        },
        boss,
    }
}

/// Each move with where it starts.
fn with_starts(path: &Toolpath) -> Vec<(DVec3, Move)> {
    let mut at = path.moves().first().map_or(DVec3::ZERO, |m| m.to);
    path.moves()
        .iter()
        .map(|m| {
            let from = at;
            at = m.to;
            (from, *m)
        })
        .collect()
}

/// How far `p` is from the round's axis.
fn off_axis(p: DVec3) -> f64 {
    p.truncate().distance(dvec2(5.0, 5.0))
}

/// Every arc of `role`: where it starts, the arc, and where it ends.
fn arcs(path: &Toolpath, role: MoveRole) -> Vec<(DVec3, Arc, DVec3)> {
    with_starts(path)
        .into_iter()
        .filter_map(|(from, m)| match m.kind {
            MoveKind::Arc(a) if m.role == role => Some((from, a, m.to)),
            _ => None,
        })
        .collect()
}

fn bore_params() -> BoreParams {
    BoreParams {
        tool_radius: 3.0,
        leave: (0.0, 0.0),
        pitch: 1.0,
        ccw: true,
        feed: 1000.0,
        retract_feed: 2000.0,
        finishing: None,
    }
}

/// A bore goes down its helix a pitch a turn to the bottom, round once
/// there, and its tool's side reaches the hole's wall and no further: the
/// diameter, within the tolerance.
#[test]
fn a_bore_reaches_the_diameter_a_pitch_a_turn() {
    let hole = round(5.0, false);
    let (path, cores) = bore(&[hole], &bore_params()).unwrap();
    assert!(cores.is_empty(), "a 10 mm hole is less than two 6 mm tools");
    let cuts = arcs(&path, MoveRole::Cut);
    assert!(!cuts.is_empty());
    for (from, a, to) in &cuts {
        assert!(a.sweep > 0.0, "climbing inside goes counter-clockwise");
        assert!(a.sweep.abs() <= TAU + 1e-9);
        // Down a pitch for each turn, or level once at the bottom.
        let down = from.z - to.z;
        let per_turn = down / (a.sweep.abs() / TAU);
        assert!(
            (per_turn - 1.0).abs() < 1e-9 || down.abs() < 1e-12,
            "{per_turn}"
        );
        assert!(
            (off_axis(*to) + 3.0 - 5.0).abs() < 1e-9,
            "the wall, exactly"
        );
    }
    let deepest = path.moves().iter().map(|m| m.to.z).fold(f64::MAX, f64::min);
    assert!((deepest - hole.hole.bottom).abs() < 1e-9);
    // From just above it, the R plane, and back out to there.
    assert_eq!(path.moves()[0].role, MoveRole::Link);
    assert_eq!(path.moves()[0].to.z, hole.hole.retract());
    assert_eq!(path.end().unwrap().z, hole.hole.retract());
    // The turns down: from 2 above to 10 below is twelve.
    let turned: f64 = cuts.iter().map(|(_, a, _)| a.sweep).sum::<f64>() / TAU;
    assert!((turned - 13.0).abs() < 1e-9, "{turned}");

    // Conventional, the other way round.
    let (back, _) = bore(
        &[hole],
        &BoreParams {
            ccw: false,
            ..bore_params()
        },
    )
    .unwrap();
    assert!(
        arcs(&back, MoveRole::Cut)
            .iter()
            .all(|(_, a, _)| a.sweep < 0.0)
    );
}

/// Finishing, the helix leaves the passes' stepovers on the wall, and each
/// pass goes in and out along a half circle tangent to it, at the bottom,
/// the last at the wall less the stock to leave; the repeat cuts it again.
#[test]
fn a_bores_finishing_passes_lead_in_and_out_along_arcs() {
    let hole = round(6.0, false);
    let params = BoreParams {
        leave: (0.1, 0.2),
        finishing: Some((2, 0.25, 500.0, true)),
        ..bore_params()
    };
    let (path, _) = bore(&[hole], &params).unwrap();
    let last = 6.0 - 3.0 - 0.1;
    let radii: Vec<f64> = arcs(&path, MoveRole::Cut)
        .iter()
        .map(|(_, _, to)| (off_axis(*to) * 1e6).round() / 1e6)
        .collect();
    let helix = last - 0.5;
    assert!(radii.iter().all(|r| *r <= last + 1e-9));
    let finishing: Vec<f64> = radii.into_iter().filter(|r| *r > helix + 1e-6).collect();
    assert_eq!(finishing.len(), 3, "two passes and the repeat");
    assert!((finishing[0] - (helix + 0.25)).abs() < 1e-6);
    assert!((finishing[2] - last).abs() < 1e-6);
    let leads = arcs(&path, MoveRole::Lead);
    assert_eq!(leads.len(), 6, "in and out of each");
    for (from, a, to) in &leads {
        assert!((a.sweep.abs() - PI).abs() < 1e-12);
        // Tangent to the circle it meets, going its way: counter-clockwise.
        let (on, t) = if off_axis(*to) > off_axis(*from) {
            (*to, 1.0)
        } else {
            (*from, 0.0)
        };
        let along = a.tangent_at(*from, *to, t).truncate();
        let radial = (on.truncate() - dvec2(5.0, 5.0)).normalize();
        assert!(along.dot(radial).abs() < 1e-9, "tangent");
        assert!(radial.perp_dot(along) > 0.0, "counter-clockwise");
        assert_eq!(to.z, -10.0 + 0.2, "at the floor, less what it leaves");
    }
}

/// A hole wider than twice the tool leaves a core; one too small for the
/// tool is refused, as is finishing that leaves no room for the helix.
#[test]
fn a_bore_says_what_it_cannot_do() {
    let (_, cores) = bore(&[round(7.0, false)], &bore_params()).unwrap();
    assert_eq!(cores, [0]);
    assert!(matches!(
        bore(&[round(3.0, false)], &bore_params()),
        Err(MillError::TooSmall { .. })
    ));
    let greedy = BoreParams {
        finishing: Some((4, 1.0, 500.0, false)),
        ..bore_params()
    };
    assert!(matches!(
        bore(&[round(5.0, false)], &greedy),
        Err(MillError::NoRoomToFinish { .. })
    ));
}

fn circular_params() -> CircularParams {
    CircularParams {
        tool_radius: 3.0,
        leave: (0.0, 0.0),
        stepover: 2.0,
        stepdown: Some(4.0),
        ramp_angle: 5f64.to_radians(),
        boss_width: 0.0,
        ccw: true,
        feed: 1000.0,
        ramp_feed: 600.0,
        retract_feed: 2000.0,
    }
}

/// A hole is cleared in circles from near its middle out to its wall, a
/// stepover apart at most, at each level a stepdown apart, the first close
/// enough in for the tool to cover the middle.
#[test]
fn circular_clears_a_hole_out_to_its_wall() {
    let hole = round(10.0, false);
    let path = circular(&[hole], &circular_params()).unwrap();
    let cuts = arcs(&path, MoveRole::Cut);
    let mut levels: Vec<f64> = cuts.iter().map(|(_, _, to)| to.z).collect();
    levels.dedup();
    assert_eq!(levels.len(), 3, "10 mm in levels of 4 at most: {levels:?}");
    assert_eq!(*levels.last().unwrap(), -10.0);
    for z in &levels {
        let mut radii: Vec<f64> = cuts
            .iter()
            .filter(|(_, _, to)| to.z == *z)
            .map(|(_, _, to)| off_axis(*to))
            .collect();
        radii.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        assert!(radii[0] < 3.0, "the first covers the middle");
        assert!(
            (radii.last().unwrap() - 7.0).abs() < 1e-9,
            "out to the wall"
        );
        assert!(radii.windows(2).all(|w| w[1] - w[0] <= 2.0 + 1e-9));
        assert!(cuts.iter().all(|(_, a, _)| a.sweep > 0.0));
    }
    // Down a helix at the ramp angle between them, never straight down.
    let ramps = arcs(&path, MoveRole::Ramp);
    assert!(!ramps.is_empty());
    for (from, a, to) in ramps {
        let run = a.radius(from) * a.sweep.abs();
        assert!((from.z - to.z) / run <= 5f64.to_radians().tan() + 1e-9);
    }
    assert!(
        path.moves()
            .iter()
            .all(|m| m.role != MoveRole::Plunge || m.kind == MoveKind::Linear)
    );
}

/// A boss is cleared from its width out in to its wall, the other way
/// round from a hole, and left outward along a half circle.
#[test]
fn circular_clears_round_a_boss_in_to_it() {
    let boss = round(8.0, true);
    let params = CircularParams {
        boss_width: 5.0,
        ..circular_params()
    };
    let path = circular(&[boss], &params).unwrap();
    let cuts = arcs(&path, MoveRole::Cut);
    let radii: Vec<f64> = cuts
        .iter()
        .take_while(|(_, _, to)| to.z == cuts[0].2.z)
        .map(|(_, _, to)| off_axis(*to))
        .collect();
    assert!((radii[0] - 16.0).abs() < 1e-9, "from the boss's width out");
    assert!(
        (radii.last().unwrap() - 11.0).abs() < 1e-9,
        "in to its wall"
    );
    assert!(radii.windows(2).all(|w| w[1] <= w[0] + 1e-9));
    assert!(
        cuts.iter().all(|(_, a, _)| a.sweep < 0.0),
        "climbing outside"
    );
    let out = arcs(&path, MoveRole::Lead);
    let (_, _, end) = out.last().unwrap();
    assert!(off_axis(*end) > 11.0, "left outward, away from the boss");
    // From level to level straight out along the ray it left on, never
    // across the boss.
    let across: Vec<_> = with_starts(&path)
        .into_iter()
        .filter(|(_, m)| m.role == MoveRole::Transition)
        .collect();
    assert!(across.len() <= 2, "three levels");
    for (from, m) in across {
        let (a, b) = (
            from.truncate() - dvec2(5.0, 5.0),
            m.to.truncate() - dvec2(5.0, 5.0),
        );
        assert!(a.perp_dot(b).abs() < 1e-9 && a.dot(b) > 0.0);
    }
}

fn thread_params() -> ThreadParams {
    ThreadParams {
        pitch: 1.25,
        hand: ThreadHand::Right,
        diameter: None,
        passes: 1,
        spring_pass: false,
        tool_radius: 2.5,
        tip_length: 0.5,
        teeth: 1,
        ccw: true,
        feed: 400.0,
        plunge_feed: 300.0,
        retract_feed: 2000.0,
    }
}

/// An M8 thread in its 6.8 mm tap drill's hole, single-point, right-hand,
/// climbing: up from the bottom, counter-clockwise, a pitch each turn, its
/// teeth reaching the major diameter; its tool's end no lower than the
/// bottom, and in and out along helical half circles.
#[test]
fn a_thread_goes_a_pitch_a_turn_to_its_major_diameter() {
    let hole = round(3.4, false);
    let params = ThreadParams {
        diameter: Some(8.0),
        ..thread_params()
    };
    let (path, done) = thread(&[hole], &params).unwrap();
    assert!(done[0].rising);
    let cuts = arcs(&path, MoveRole::Cut);
    for (from, a, to) in &cuts {
        assert!(a.sweep > 0.0);
        let per_turn = (to.z - from.z) / (a.sweep / TAU);
        assert!((per_turn - 1.25).abs() < 1e-9, "{per_turn}");
        assert!(
            (off_axis(*to) + 2.5 - 4.0).abs() < 1e-9,
            "to the major diameter"
        );
    }
    let deepest = path.moves().iter().map(|m| m.to.z).fold(f64::MAX, f64::min);
    assert!(deepest >= hole.hole.bottom - 1e-9, "{deepest}");
    let leads = arcs(&path, MoveRole::Lead);
    assert_eq!(leads.len(), 2);
    for (from, a, to) in leads {
        assert!(
            (a.sweep - PI).abs() < 1e-12,
            "half circles, the thread's way"
        );
        assert!(
            (to.z - from.z - 1.25 / 4.0).abs() < 1e-9,
            "a quarter pitch up"
        );
    }
    // Its own diameter unless given: the hole's, plus the thread's depth.
    let (own, _) = thread(&[hole], &thread_params()).unwrap();
    let r = off_axis(arcs(&own, MoveRole::Cut)[0].2) + 2.5;
    assert!((r - (3.4 + thread_depth(1.25, false))).abs() < 1e-9);
}

/// Radial passes go deeper by equal parts of the thread's depth, the spring
/// pass is the last again, and a left-hand thread goes down as it climbs.
#[test]
fn a_threads_passes_go_deeper_and_a_left_hand_goes_down() {
    let hole = round(3.4, false);
    let params = ThreadParams {
        diameter: Some(8.0),
        passes: 3,
        spring_pass: true,
        ..thread_params()
    };
    let (path, _) = thread(&[hole], &params).unwrap();
    let mut radii: Vec<f64> = arcs(&path, MoveRole::Cut)
        .iter()
        .map(|(_, _, to)| ((off_axis(*to) + 2.5) * 1e9).round() / 1e9)
        .collect();
    radii.dedup();
    let depth = thread_depth(1.25, false);
    let want = [4.0 - 2.0 * depth / 3.0, 4.0 - depth / 3.0, 4.0];
    assert_eq!(radii.len(), 3);
    for (r, w) in radii.iter().zip(want) {
        assert!((r - w).abs() < 1e-6, "{radii:?}");
    }
    let cut_turns = arcs(&path, MoveRole::Cut).len();
    assert_eq!(cut_turns % 4, 0, "three passes and the spring pass alike");

    let left = ThreadParams {
        hand: ThreadHand::Left,
        ..thread_params()
    };
    let (path, done) = thread(&[hole], &left).unwrap();
    assert!(!done[0].rising);
    for (from, a, to) in arcs(&path, MoveRole::Cut) {
        assert!(a.sweep > 0.0 && to.z < from.z);
    }
}

/// A multi-form mill cuts as many turns as it has rows of teeth each time
/// round: one turn each, its lowest tooth stepping up by all of them.
#[test]
fn a_multi_form_mill_goes_round_once_for_its_teeth() {
    let hole = round(3.4, false);
    let params = ThreadParams {
        teeth: 4,
        ..thread_params()
    };
    let (path, _) = thread(&[hole], &params).unwrap();
    let cuts = arcs(&path, MoveRole::Cut);
    // 10 mm less the tip and a quarter pitch, in 5 mm spans: two.
    assert_eq!(cuts.len(), 2);
    for (_, a, _) in &cuts {
        assert!((a.sweep - TAU).abs() < 1e-9);
    }
    let rise = cuts[1].0.z - cuts[0].0.z;
    assert!((rise - 4.0 * 1.25).abs() < 1e-9, "{rise}");
}

/// An external thread on a boss: its teeth reach its minor diameter, from
/// outside, clockwise to climb, so a right-hand one goes down.
#[test]
fn a_thread_on_a_boss_is_cut_from_outside() {
    let boss = round(4.0, true);
    let (path, done) = thread(&[boss], &thread_params()).unwrap();
    assert!(!done[0].rising);
    let minor = 4.0 - thread_depth(1.25, true);
    for (from, a, to) in arcs(&path, MoveRole::Cut) {
        assert!(a.sweep < 0.0 && to.z < from.z);
        assert!((off_axis(to) - 2.5 - minor).abs() < 1e-9);
    }
    assert!(
        path.moves()
            .iter()
            .filter(|m| m.role == MoveRole::Plunge)
            .all(|m| off_axis(m.to) > 4.0 + 2.5),
        "down outside it"
    );
}

#[test]
fn a_thread_says_what_it_cannot_do() {
    assert!(matches!(
        thread(&[round(2.0, false)], &thread_params()),
        Err(MillError::TooSmall { .. })
    ));
    let narrow = ThreadParams {
        diameter: Some(6.0),
        ..thread_params()
    };
    assert!(matches!(
        thread(&[round(3.4, false)], &narrow),
        Err(MillError::NoThread { .. })
    ));
    let mut short = round(3.4, false);
    short.hole.bottom = -0.5;
    assert!(matches!(
        thread(&[short], &thread_params()),
        Err(MillError::NoLength { .. })
    ));
}
