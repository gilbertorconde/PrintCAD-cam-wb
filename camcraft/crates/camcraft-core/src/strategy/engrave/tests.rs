use glam::{dvec2, dvec3};

use super::*;
use crate::path::toolpath::MoveKind;

fn params() -> EngraveParams {
    EngraveParams {
        top: 10.0,
        depth: 0.5,
        stepdown: None,
        ramp_angle: 2f64.to_radians(),
        feed: 1000.0,
        ramp_feed: 600.0,
        plunge_feed: 300.0,
        direction: TraceDirection::AsDrawn,
        reverse: false,
        climb_right: true,
        tolerance: 0.01,
    }
}

/// A 10 mm square, counter-clockwise from the origin.
fn square() -> Trace {
    let to = |x, y| Piece::Line {
        to: dvec3(x, y, 0.0),
    };
    Trace {
        start: dvec3(0.0, 0.0, 0.0),
        pieces: vec![to(10.0, 0.0), to(10.0, 10.0), to(0.0, 10.0), to(0.0, 0.0)],
    }
}

/// A circle of radius 5 about (20, 0), as two half turns counter-clockwise.
fn circle() -> Trace {
    let centre = dvec2(20.0, 0.0);
    Trace {
        start: dvec3(25.0, 0.0, 0.0),
        pieces: vec![
            Piece::Arc {
                centre,
                ccw: true,
                to: dvec3(15.0, 0.0, 0.0),
            },
            Piece::Arc {
                centre,
                ccw: true,
                to: dvec3(25.0, 0.0, 0.0),
            },
        ],
    }
}

/// The moves after the first link, each with where it starts.
fn cuts(path: &Toolpath) -> Vec<(DVec3, crate::path::toolpath::Move)> {
    let moves = path.moves();
    let mut at = moves[0].to;
    let mut out = Vec::new();
    for m in &moves[1..] {
        out.push((at, *m));
        at = m.to;
    }
    out
}

#[test]
fn a_closed_curve_ramps_round_then_goes_round_at_the_depth() {
    let (path, report) = engrave(&[square()], &params()).unwrap();
    assert_eq!(report.curves, 1);
    assert_eq!(report.closed, 1);
    // 40 mm round at 2° goes down 1.4 mm: the half millimetre in one lap.
    assert_eq!(report.passes, 2);
    let moves = path.moves();
    assert_eq!(moves[0].role, MoveRole::Link);
    assert_eq!(moves[0].to, dvec3(0.0, 0.0, 10.0));
    assert_eq!(
        moves.iter().filter(|m| m.role == MoveRole::Link).count(),
        1,
        "it never leaves the curve between its levels"
    );
    let lowest = moves.iter().map(|m| m.to.z).fold(f64::INFINITY, f64::min);
    assert!((lowest - 9.5).abs() < 1e-9, "{lowest}");
    // The last lap is all at the depth, and ends where it started.
    for m in &moves[moves.len() - 4..] {
        assert_eq!(m.role, MoveRole::Cut);
        assert!((m.to.z - 9.5).abs() < 1e-9);
    }
    assert_eq!(path.end().unwrap(), dvec3(0.0, 0.0, 9.5));
    // Every move is on the square.
    for m in moves {
        let (x, y) = (m.to.x, m.to.y);
        let on = [x, 10.0 - x, y, 10.0 - y].iter().any(|d| d.abs() < 1e-9);
        assert!(on, "{:?}", m.to);
    }
}

#[test]
fn arcs_stay_arcs_and_ramp_as_helixes() {
    let p = EngraveParams {
        depth: 1.0,
        stepdown: Some(0.4),
        ..params()
    };
    let (path, report) = engrave(&[circle()], &p).unwrap();
    // 0.4, 0.4 and 0.2 down, then once at the depth: two arcs each.
    assert_eq!(report.passes, 4);
    assert_eq!(report.arcs, 8);
    let mut helixes = 0;
    for (from, m) in cuts(&path) {
        let MoveKind::Arc(a) = m.kind else {
            panic!("a line in a circle: {m:?}");
        };
        assert_eq!(a.axis, DVec3::Z);
        assert!((a.radius(from) - 5.0).abs() < 1e-9);
        assert!((a.centre.truncate() - dvec2(20.0, 0.0)).length() < 1e-9);
        if (m.to.z - from.z).abs() > 1e-9 {
            helixes += 1;
            assert_eq!(m.role, MoveRole::Ramp);
        }
    }
    assert_eq!(helixes, 6);
    assert!((path.end().unwrap().z - 9.0).abs() < 1e-9);
}

#[test]
fn an_open_curve_goes_back_and_forth_down_to_the_depth() {
    let line = Trace {
        start: dvec3(0.0, 0.0, 0.0),
        pieces: vec![Piece::Line {
            to: dvec3(10.0, 0.0, 0.0),
        }],
    };
    let p = EngraveParams {
        depth: 1.0,
        stepdown: Some(0.25),
        ..params()
    };
    let (path, report) = engrave(&[line], &p).unwrap();
    assert_eq!(report.closed, 0);
    // 10 mm at 2° is 0.35 mm down: a stepdown each way, four, then the last.
    assert_eq!(report.passes, 5);
    let ends: Vec<DVec3> = path.moves()[1..].iter().map(|m| m.to).collect();
    let xs: Vec<f64> = ends.iter().map(|p| p.x).collect();
    assert_eq!(xs, [10.0, 0.0, 10.0, 0.0, 10.0]);
    let zs: Vec<f64> = ends.iter().map(|p| p.z).collect();
    for (z, want) in zs.iter().zip([9.75, 9.5, 9.25, 9.0, 9.0]) {
        assert!((z - want).abs() < 1e-9, "{zs:?}");
    }
}

#[test]
fn the_direction_and_reverse_turn_a_closed_curve() {
    let first_corner = |direction, reverse| {
        let p = EngraveParams {
            direction,
            reverse,
            ..params()
        };
        engrave(&[square()], &p).unwrap().0.moves()[1].to.truncate()
    };
    // Drawn counter-clockwise: first along +X.
    assert_eq!(
        first_corner(TraceDirection::AsDrawn, false),
        dvec2(10.0, 0.0)
    );
    assert_eq!(
        first_corner(TraceDirection::AsDrawn, true),
        dvec2(0.0, 10.0)
    );
    // Climbing with a clockwise spindle goes round clockwise: up +Y first.
    assert_eq!(first_corner(TraceDirection::Climb, false), dvec2(0.0, 10.0));
    assert_eq!(
        first_corner(TraceDirection::Conventional, false),
        dvec2(10.0, 0.0)
    );
    assert_eq!(first_corner(TraceDirection::Climb, true), dvec2(10.0, 0.0));
}

#[test]
fn curves_are_cut_nearest_first() {
    let far = Trace {
        start: dvec3(100.0, 0.0, 0.0),
        pieces: vec![Piece::Line {
            to: dvec3(110.0, 0.0, 0.0),
        }],
    };
    let (path, report) = engrave(&[square(), far, circle()], &params()).unwrap();
    assert_eq!(report.curves, 3);
    let links: Vec<DVec2> = path
        .moves()
        .iter()
        .filter(|m| m.role == MoveRole::Link)
        .map(|m| m.to.truncate())
        .collect();
    assert_eq!(
        links,
        [dvec2(0.0, 0.0), dvec2(25.0, 0.0), dvec2(100.0, 0.0)]
    );
}
