use super::*;
use camcraft_geom::contour::{Contour, Region};
use glam::{dvec2, dvec3};

use crate::job::{Picks, ProfileSide};
use crate::strategy::planar::{PlanarParams, areas, pocket, profile};

const R: f64 = 3.0;

fn relief(style: ReliefStyle) -> Relief {
    Relief {
        style,
        sharper_than: None,
    }
}

/// A closed ring through `points`, lines between them.
fn ring(points: &[DVec2]) -> Ring {
    Ring::from_points(points).unwrap()
}

/// A rectangle's ring, anticlockwise from its lower left corner.
fn rectangle(w: f64, h: f64) -> Ring {
    ring(&[dvec2(0.0, 0.0), dvec2(w, 0.0), dvec2(w, h), dvec2(0.0, h)])
}

/// The corner of the wall a path with `kept` on one side leaves at its
/// corner `c` of a ring running `into` then `out`, the walls `R` from it.
fn wall_corner(at: DVec2, into: DVec2, out: DVec2, kept_right: bool) -> DVec2 {
    let side = |t: DVec2| if kept_right { -t.perp() } else { t.perp() };
    // Where the two walls' lines cross.
    let (p, d) = (at + side(into) * R, into);
    let (q, e) = (at + side(out) * R, out);
    let t = (q - p).perp_dot(e) / d.perp_dot(e);
    p + d * t
}

/// A pocket's square corner: a dogbone reaches out halfway between the walls,
/// and a tool there touches the walls' sharp corner exactly, as a router user
/// expects of a 90° inside corner: a round notch reaching the corner's point.
#[test]
fn a_dogbone_reaches_the_sharp_corner() {
    let edges = ring_edges(&rectangle(20.0, 10.0));
    let found = corners(
        &edges,
        true,
        Kept::Right,
        R,
        &relief(ReliefStyle::Dogbone),
        0.01,
    );
    assert_eq!(found.len(), 4, "{found:?}");
    let c = found.iter().find(|c| c.at == dvec2(20.0, 0.0)).unwrap();
    let corner = wall_corner(c.at, DVec2::X, DVec2::Y, true);
    assert!((corner - dvec2(23.0, -3.0)).length() < 1e-12);
    assert!((c.reach.distance(corner) - R).abs() < 1e-9, "{c:?}");
    // Halfway between the walls, out of the corner.
    let way = (c.reach - c.at).normalize();
    assert!((way - dvec2(1.0, -1.0).normalize()).length() < 1e-9);
    assert!((c.reach.distance(c.at) - R * (2f64.sqrt() - 1.0)).abs() < 1e-9);
}

/// Sharper and blunter corners are reached exactly too, and the blunt ones
/// only by a little.
#[test]
fn every_angle_is_reached_exactly() {
    for deg in [30.0f64, 60.0, 90.0, 120.0, 150.0] {
        // Along X, then turning left by 180° less the corner's angle.
        let turn = (180.0 - deg).to_radians();
        let out = DVec2::from_angle(turn);
        let points = [dvec2(-10.0, 0.0), DVec2::ZERO, out * 10.0];
        let edges: Vec<Edge> = points
            .windows(2)
            .map(|w| Edge::of_span(w[0], &Span::Line { to: w[1] }))
            .collect();
        for style in [
            ReliefStyle::Dogbone,
            ReliefStyle::TBoneX,
            ReliefStyle::TBoneY,
        ] {
            let found = corners(&edges, false, Kept::Right, R, &relief(style), 0.01);
            assert_eq!(found.len(), 1, "{deg}° {style:?}");
            let corner = wall_corner(DVec2::ZERO, DVec2::X, out, true);
            let reached = found[0].reach.distance(corner);
            assert!((reached - R).abs() < 1e-9, "{deg}° {style:?}: {reached}");
        }
    }
}

/// Only the corners whose outside is the material kept: a pocket's inside
/// corners, and none round a part cut out, whose corners the tool goes round.
/// The part's own inside corner, where an L turns, is relieved.
#[test]
fn only_inside_corners_are_relieved() {
    let edges = ring_edges(&rectangle(20.0, 10.0));
    let dogbone = relief(ReliefStyle::Dogbone);
    assert!(corners(&edges, true, Kept::Left, R, &dogbone, 0.01).is_empty());
    assert_eq!(
        corners(&edges, true, Kept::Both, R, &dogbone, 0.01).len(),
        4
    );
    // An L, anticlockwise, kept inside it: the one corner turning right,
    // relieved into the L.
    let l = ring(&[
        dvec2(0.0, 0.0),
        dvec2(20.0, 0.0),
        dvec2(20.0, 10.0),
        dvec2(10.0, 10.0),
        dvec2(10.0, 20.0),
        dvec2(0.0, 20.0),
    ]);
    let found = corners(&ring_edges(&l), true, Kept::Left, R, &dogbone, 0.01);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].at, dvec2(10.0, 10.0));
    assert!(found[0].reach.x < 10.0 && found[0].reach.y < 10.0);
}

/// With an angle asked for, only corners sharper than it: an octagon's 135°
/// corners are left, a square's 90° ones relieved.
#[test]
fn obtuse_corners_are_left_under_an_angle() {
    let octagon: Vec<DVec2> = (0..8)
        .map(|i| DVec2::from_angle(f64::from(i) * std::f64::consts::FRAC_PI_4) * 20.0)
        .collect();
    let octagon = ring_edges(&ring(&octagon));
    let square = ring_edges(&rectangle(20.0, 20.0));
    let limited = Relief {
        sharper_than: Some(100.0),
        ..relief(ReliefStyle::Dogbone)
    };
    let all = relief(ReliefStyle::Dogbone);
    assert_eq!(corners(&octagon, true, Kept::Right, R, &all, 0.01).len(), 8);
    assert!(corners(&octagon, true, Kept::Right, R, &limited, 0.01).is_empty());
    assert_eq!(
        corners(&square, true, Kept::Right, R, &limited, 0.01).len(),
        4
    );
    // A curve's chords turn too little to be corners, and a corner relieved
    // by less than the tolerance is left.
    let circle: Vec<DVec2> = (0..72)
        .map(|i| DVec2::from_angle(f64::from(i) * 5f64.to_radians()) * 20.0)
        .collect();
    let circle = ring_edges(&ring(&circle));
    assert!(corners(&circle, true, Kept::Right, R, &all, 0.01).is_empty());
    assert!(corners(&octagon, true, Kept::Right, R, &all, 0.3).is_empty());
}

/// A T-bone goes on along one wall into the other, the tool's radius at a
/// square corner: along X or Y as asked, or along the longer or shorter wall.
#[test]
fn a_t_bone_goes_along_the_wall_asked() {
    // 20 along X, 10 up Y: at (20, 0) the wall along X is the longer.
    let edges = ring_edges(&rectangle(20.0, 10.0));
    let at = dvec2(20.0, 0.0);
    let reach = |style| {
        corners(&edges, true, Kept::Right, R, &relief(style), 0.01)
            .into_iter()
            .find(|c| c.at == at)
            .unwrap()
            .reach
    };
    let on_x = dvec2(20.0 + R, 0.0);
    let on_y = dvec2(20.0, -R);
    for (style, want) in [
        (ReliefStyle::TBoneX, on_x),
        (ReliefStyle::TBoneY, on_y),
        (ReliefStyle::TBoneLong, on_x),
        (ReliefStyle::TBoneShort, on_y),
    ] {
        let got = reach(style);
        assert!(got.distance(want) < 1e-9, "{style:?}: {got}");
    }
    // At (20, 10) the other way round: on along Y, or back along X.
    let at = dvec2(20.0, 10.0);
    let found = corners(
        &edges,
        true,
        Kept::Right,
        R,
        &relief(ReliefStyle::TBoneX),
        0.01,
    );
    let c = found.iter().find(|c| c.at == at).unwrap();
    assert!(c.reach.distance(dvec2(20.0 + R, 10.0)) < 1e-9, "{c:?}");
}

/// A slot keeps both its walls: each bend is relieved on its outside, its
/// ends are not.
#[test]
fn a_slot_bend_is_relieved_outside() {
    let trace = Trace {
        start: dvec3(0.0, 0.0, 0.0),
        pieces: vec![
            Piece::Line {
                to: dvec3(10.0, 0.0, 0.0),
            },
            Piece::Line {
                to: dvec3(10.0, 10.0, 0.0),
            },
        ],
    };
    let (edges, closed) = trace_edges(&trace, 0.01);
    assert!(!closed);
    let found = corners(
        &edges,
        closed,
        Kept::Both,
        R,
        &relief(ReliefStyle::Dogbone),
        0.01,
    );
    assert_eq!(found.len(), 1);
    assert!(
        found[0].reach.x > 10.0 && found[0].reach.y < 0.0,
        "{found:?}"
    );
    // An arc's ends run along it: one tangent to a line is no corner.
    let rounded = Trace {
        start: dvec3(0.0, 0.0, 0.0),
        pieces: vec![
            Piece::Line {
                to: dvec3(10.0, 0.0, 0.0),
            },
            Piece::Arc {
                centre: dvec2(10.0, 5.0),
                ccw: true,
                to: dvec3(15.0, 5.0, 0.0),
            },
        ],
    };
    let (edges, _) = trace_edges(&rounded, 0.01);
    assert!((edges[1].length - 5.0 * std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    assert!(
        corners(
            &edges,
            false,
            Kept::Both,
            R,
            &relief(ReliefStyle::Dogbone),
            0.01
        )
        .is_empty()
    );
}

fn square(lo: f64, hi: f64) -> Contour {
    Contour::new(vec![
        dvec2(lo, lo),
        dvec2(hi, lo),
        dvec2(hi, hi),
        dvec2(lo, hi),
    ])
    .unwrap()
}

fn params(stepdown: f64) -> PlanarParams {
    PlanarParams {
        stepdown: Some(stepdown),
        stepover: 2.0,
        feed: 1000.0,
        arc: 0.01,
        radius: R,
        kept_on_right: true,
        leads: None,
        spiral: None,
        finishing: None,
    }
}

/// A square pocket cut in three levels: every level's wall ring goes out
/// into each corner and back, level, and nothing else does; the corners are
/// its areas' rings' own, with the wall on their right as the region winds
/// them.
#[test]
fn a_pocket_is_relieved_at_every_level() {
    let shape = Region::new(vec![square(0.0, 30.0)]);
    let floor = |_: Option<Picks>| (0.0, -6.0);
    let found = areas(&shape, &[], -R, 0.01, &[], &floor).unwrap();
    let edges: Vec<Edge> = found
        .iter()
        .flat_map(|a| &a.rings)
        .flat_map(ring_edges)
        .collect();
    let dogbone = relief(ReliefStyle::Dogbone);
    let wanted = corners(&edges, true, Kept::Right, R, &dogbone, 0.01);
    assert_eq!(wanted.len(), 4);
    assert!(corners(&edges, true, Kept::Left, R, &dogbone, 0.01).is_empty());
    for c in &wanted {
        // Each reaches the pocket's own corner.
        let corner = dvec2(
            if c.at.x > 15.0 { 30.0 } else { 0.0 },
            if c.at.y > 15.0 { 30.0 } else { 0.0 },
        );
        assert!((c.reach.distance(corner) - R).abs() < 1e-6, "{c:?}");
    }
    let (path, _) = pocket(&found, &params(2.0)).unwrap();
    let (relieved, count) = relieve(&path, &wanted).unwrap();
    assert_eq!(count, 4 * 3);
    assert_eq!(relieved.moves().len(), path.moves().len() + 2 * 4 * 3);
    // Out and back, level, at the corner's height.
    let moves = relieved.moves();
    for (i, m) in moves.iter().enumerate() {
        if let Some(c) = wanted
            .iter()
            .find(|c| m.to.truncate().distance(c.reach) < 1e-9)
        {
            let (before, after) = (moves[i - 1].to, moves[i + 1].to);
            assert_eq!(before, after);
            assert_eq!(before.truncate(), c.at);
            assert_eq!(m.to.z, before.z);
            assert_eq!(m.role, MoveRole::Cut);
        }
    }
}

/// A part profiled out of a square hole: the hole's corners are relieved
/// into the part, the outline's are not; and leads, which end where a ring
/// starts, are never relieved.
#[test]
fn a_profile_relieves_its_hole_and_not_its_outline() {
    let part = Region::new(vec![square(0.0, 40.0), square(10.0, 30.0)]);
    let floor = |_: Option<Picks>| (0.0, -2.0);
    let found = areas(&part, &[], R, 0.01, &[], &floor).unwrap();
    let edges: Vec<Vec<Edge>> = found
        .iter()
        .flat_map(|a| &a.rings)
        .map(ring_edges)
        .collect();
    let dogbone = relief(ReliefStyle::Dogbone);
    let wanted: Vec<Corner> = edges
        .iter()
        .flat_map(|e| corners(e, true, Kept::Left, R, &dogbone, 0.01))
        .collect();
    assert_eq!(wanted.len(), 4, "{wanted:?}");
    for c in &wanted {
        assert!(c.at.x > 10.0 && c.at.x < 30.0, "{c:?}");
        let corner = dvec2(
            if c.at.x > 20.0 { 30.0 } else { 10.0 },
            if c.at.y > 20.0 { 30.0 } else { 10.0 },
        );
        assert!((c.reach.distance(corner) - R).abs() < 1e-6, "{c:?}");
    }
    let (path, _) = profile(&found, ProfileSide::Outside, &params(2.0)).unwrap();
    let (_, count) = relieve(&path, &wanted).unwrap();
    assert_eq!(count, 4);
    // A lead ending at a corner is not cutting along the wall.
    let mut led = Toolpath::new();
    led.feed_to(dvec3(5.0, 5.0, 0.0), MoveRole::Link, 100.0)
        .unwrap();
    led.feed_to(dvec3(13.0, 13.0, 0.0), MoveRole::Lead, 100.0)
        .unwrap();
    assert_eq!(relieve(&led, &wanted).unwrap().1, 0);
}
