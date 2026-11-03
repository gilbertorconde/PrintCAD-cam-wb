use glam::dvec3;

use super::*;
use crate::job::{Linear, Mirror, Polar};
use crate::path::toolpath::Arc;

/// A ring round a 10 mm square from (1, 1), counter-clockwise, at z = -1,
/// linked into: a level pass.
fn ring() -> Toolpath {
    let mut t = Toolpath::new();
    t.feed_to(dvec3(1.0, 1.0, -1.0), MoveRole::Link, 500.0)
        .unwrap();
    for (x, y) in [(11.0, 1.0), (11.0, 11.0), (1.0, 11.0), (1.0, 1.0)] {
        t.feed_to(dvec3(x, y, -1.0), MoveRole::Cut, 1000.0).unwrap();
    }
    t
}

/// The signed area round the cuts of one pass, seen from above.
fn area(moves: &[Move]) -> f64 {
    let pts: Vec<DVec2> = moves.iter().map(|m| m.to.truncate()).collect();
    pts.iter()
        .zip(pts.iter().cycle().skip(1))
        .map(|(a, b)| a.perp_dot(*b))
        .sum::<f64>()
        / 2.0
}

#[test]
fn rows_are_spaced_and_polar_copies_spread_round_their_centre() {
    let rows = Pattern {
        linear: Some(Linear {
            x_count: 3,
            x_spacing: 20.0,
            y_count: 2,
            y_spacing: -15.0,
        }),
        ..Pattern::default()
    };
    let all = copies(&rows);
    assert_eq!(all.len(), 6);
    assert_eq!(rows.copies(), 6);
    let starts: Vec<DVec2> = all
        .iter()
        .map(|c| c.point(DVec3::ZERO).truncate())
        .collect();
    assert_eq!(starts[0], DVec2::ZERO);
    assert_eq!(starts[2], DVec2::new(40.0, 0.0));
    assert_eq!(starts[5], DVec2::new(40.0, -15.0));

    // Four round (10, 0), a whole turn: a quarter apart.
    let round = Pattern {
        polar: Some(Polar {
            count: 4,
            angle: 360.0,
            centre: [10.0, 0.0],
        }),
        ..Pattern::default()
    };
    let at: Vec<DVec3> = copies(&round)
        .iter()
        .map(|c| c.point(dvec3(20.0, 0.0, 3.0)))
        .collect();
    for (p, want) in at
        .iter()
        .zip([(20.0, 0.0), (10.0, 10.0), (0.0, 0.0), (10.0, -10.0)])
    {
        assert!((p.truncate() - DVec2::from(want)).length() < 1e-9, "{at:?}");
        assert_eq!(p.z, 3.0, "heights kept");
    }
    // Three over a quarter turn: the last a quarter turn from the first.
    let fan = Pattern {
        polar: Some(Polar {
            count: 3,
            angle: 90.0,
            centre: [0.0, 0.0],
        }),
        ..Pattern::default()
    };
    let last = copies(&fan)[2].point(dvec3(1.0, 0.0, 0.0));
    assert!((last - dvec3(0.0, 1.0, 0.0)).length() < 1e-9, "{last}");
}

/// Mirrored about a line along Y through x = 20: the copy is the ring
/// turned over, beside it, and turned round, so it goes counter-clockwise
/// as the original does and still climbs; linked to from its other end.
#[test]
fn a_mirrored_ring_is_turned_round_to_cut_the_same_way() {
    let mirror = Pattern {
        mirror: Some(Mirror {
            axis: MirrorAxis::Y,
            through: [20.0, 0.0],
        }),
        ..Pattern::default()
    };
    let r = repeat(&ring(), &mirror, false).unwrap();
    assert_eq!(r.copies, 2);
    assert_eq!(r.unturned, 0);
    let passes: Vec<&[Move]> = r.path.passes().collect();
    assert_eq!(passes.len(), 2);
    assert_eq!(passes[0], ring().moves());
    let copy = passes[1];
    assert_eq!(copy[0].role, MoveRole::Link);
    assert!(
        copy.iter()
            .all(|m| m.to.x >= 29.0 - 1e-9 && m.to.x <= 39.0 + 1e-9)
    );
    assert!(area(&copy[1..]) > 0.0, "counter-clockwise, as the original");
    assert!(area(&passes[0][1..]) > 0.0);
    // Not turned round, it would run clockwise.
    let plain = repeat(&ring(), &mirror, true).unwrap();
    assert_eq!(plain.unturned, 1);
    let copy: Vec<&[Move]> = plain.path.passes().collect();
    assert!(area(&copy[1][1..]) < 0.0);
}

/// A pass that ramps down onto a ring of arcs: mirrored, the ring is run
/// backwards in place, from and back to where the ramp left it, its arcs
/// arcs the original way round; the ramp is as it was, turned over.
#[test]
fn a_ring_within_a_pass_is_turned_round_in_place() {
    let centre = dvec3(0.0, 0.0, -1.0);
    let half = |sweep: f64| Arc {
        centre,
        axis: DVec3::Z,
        sweep,
    };
    let mut t = Toolpath::new();
    t.feed_to(dvec3(5.0, 0.0, 0.0), MoveRole::Link, 500.0)
        .unwrap();
    t.feed_to(dvec3(0.0, 5.0, -1.0), MoveRole::Ramp, 400.0)
        .unwrap();
    t.feed_to(dvec3(5.0, 0.0, -1.0), MoveRole::Ramp, 400.0)
        .unwrap();
    let pi = std::f64::consts::PI;
    t.arc_to(dvec3(-5.0, 0.0, -1.0), half(pi), MoveRole::Cut, 1000.0)
        .unwrap();
    t.arc_to(dvec3(5.0, 0.0, -1.0), half(pi), MoveRole::Cut, 1000.0)
        .unwrap();
    let mirror = Pattern {
        mirror: Some(Mirror {
            axis: MirrorAxis::X,
            through: [0.0, 20.0],
        }),
        ..Pattern::default()
    };
    let r = repeat(&t, &mirror, false).unwrap();
    assert_eq!(r.unturned, 0);
    let copy: Vec<&[Move]> = r.path.passes().collect();
    let copy = copy[1];
    assert_eq!(copy.len(), 5);
    // The ramp turned over: about y = 20.
    assert!((copy[1].to - dvec3(0.0, 35.0, -1.0)).length() < 1e-9);
    assert_eq!(copy[1].role, MoveRole::Ramp);
    for m in &copy[3..] {
        let MoveKind::Arc(a) = m.kind else {
            panic!("{m:?}");
        };
        assert!(a.sweep > 0.0, "counter-clockwise, as the original");
        assert!((a.centre.truncate() - DVec2::new(0.0, 40.0)).length() < 1e-9);
    }
    assert!((copy[4].to - dvec3(5.0, 40.0, -1.0)).length() < 1e-9);
}

#[test]
fn how_far_outside_a_box_a_path_goes() {
    let lo = DVec2::new(0.0, 0.0);
    let hi = DVec2::new(10.0, 10.0);
    assert!((overhang(ring().moves(), lo, hi) - 2f64.sqrt()).abs() < 1e-12);
    assert_eq!(overhang(ring().moves(), lo, DVec2::new(20.0, 20.0)), 0.0);
}
