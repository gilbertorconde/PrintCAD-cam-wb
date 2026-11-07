use std::f64::consts::PI;

use glam::{DVec2, dvec2, dvec3};

use super::*;
use crate::path::toolpath::{Arc, MoveRole};

/// A 3 x 2 grid on a plane rising 0.01 a millimetre along X and 0.02
/// along Y, 1 high at the origin, in serpentine order as probed.
fn tilted() -> Vec<[f64; 3]> {
    let z = |x: f64, y: f64| 1.0 + 0.01 * x + 0.02 * y;
    let mut out = Vec::new();
    for (row, y) in [0.0, 10.0].into_iter().enumerate() {
        let mut xs = vec![0.0, 10.0, 20.0];
        if row == 1 {
            xs.reverse();
        }
        for x in xs {
            out.push([x, y, z(x, y)]);
        }
    }
    out
}

#[test]
fn reads_a_linuxcnc_probe_log() {
    // X Y Z A B C U V W, as (PROBEOPEN) writes them.
    let text = "0.000000 0.000000 -0.012000 0.000000 0.000000 0.000000 0.000000 0.000000 0.000000\n\
                10.000000 0.000000 0.034000 0.000000 0.000000 0.000000 0.000000 0.000000 0.000000\n";
    assert_eq!(
        parse(text).unwrap(),
        vec![[0.0, 0.0, -0.012], [10.0, 0.0, 0.034]]
    );
}

#[test]
fn reads_a_csv_with_a_heading_and_comments() {
    let text = "x,y,z\n# probed on the left\n0,0,0.1\n\n5, 0, 0.2\n";
    assert_eq!(parse(text).unwrap(), vec![[0.0, 0.0, 0.1], [5.0, 0.0, 0.2]]);
}

#[test]
fn refuses_a_line_that_is_not_a_point_after_the_first() {
    let e = parse("0 0 0\nok\n1 0 0\n").unwrap_err();
    assert_eq!(
        e,
        ProbeMapError::NotNumbers {
            line: 2,
            text: "ok".into()
        }
    );
    assert!(parse("0 0\n").is_err());
}

#[test]
fn points_in_any_order_make_the_grid() {
    let map = ProbeMap::from_points(&tilted(), 1.0).unwrap();
    assert_eq!(map.size(), (3, 2));
    assert_eq!(map.bounds(), (dvec2(0.0, 0.0), dvec2(20.0, 10.0)));
    // At every probed point, what was probed there.
    for p in tilted() {
        assert!((map.height(dvec2(p[0], p[1])) - p[2]).abs() < 1e-12);
    }
}

#[test]
fn a_controllers_rounding_is_the_same_column() {
    let points = [
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 1.0],
        [0.002, 5.0, 0.0],
        [9.999, 5.0, 1.0],
    ];
    let map = ProbeMap::from_points(&points, 1.0).unwrap();
    assert_eq!(map.size(), (2, 2));
    assert!((map.height(dvec2(5.0, 2.5)) - 0.5).abs() < 1e-3);
}

#[test]
fn points_that_are_no_grid_are_refused() {
    let missing = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]];
    assert_eq!(
        ProbeMap::from_points(&missing, 1.0).unwrap_err(),
        ProbeMapError::NotAGrid {
            points: 3,
            columns: 2,
            rows: 2
        }
    );
    let twice = [
        [0.0, 0.0, 0.0],
        [0.0, 0.0, 0.1],
        [10.0, 0.0, 0.0],
        [5.0, 3.0, 0.0],
    ];
    assert!(matches!(
        ProbeMap::from_points(&twice, 1.0),
        Err(ProbeMapError::Twice { .. }) | Err(ProbeMapError::NotAGrid { .. })
    ));
    let same = [[0.0, 0.0, 0.0], [0.0, 0.0, 0.1]];
    assert_eq!(
        ProbeMap::from_points(&same, 1.0).unwrap_err(),
        ProbeMapError::NotAGrid {
            points: 2,
            columns: 1,
            rows: 1
        }
    );
    assert_eq!(
        ProbeMap::from_points(&[], 1.0).unwrap_err(),
        ProbeMapError::Empty
    );
}

#[test]
fn a_plane_is_interpolated_exactly() {
    let map = ProbeMap::from_points(&tilted(), 1.0).unwrap();
    for (x, y) in [(3.0, 7.0), (15.5, 0.5), (10.0, 10.0), (19.9, 9.9)] {
        let (z, off) = map.at(dvec2(x, y));
        assert!((z - (1.0 + 0.01 * x + 0.02 * y)).abs() < 1e-12, "{x} {y}");
        assert_eq!(off, 0.0);
    }
}

#[test]
fn a_cell_is_bilinear() {
    // A saddle: three corners at 0 and one at 1.
    let points = [
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [0.0, 2.0, 0.0],
        [2.0, 2.0, 1.0],
    ];
    let map = ProbeMap::from_points(&points, 1.0).unwrap();
    assert!((map.height(dvec2(1.0, 1.0)) - 0.25).abs() < 1e-12);
    assert!((map.height(dvec2(2.0, 1.0)) - 0.5).abs() < 1e-12);
    assert!((map.height(dvec2(1.5, 2.0)) - 0.75).abs() < 1e-12);
}

#[test]
fn outside_the_grid_is_its_edge_and_says_how_far() {
    let map = ProbeMap::from_points(&tilted(), 1.0).unwrap();
    let (z, off) = map.at(dvec2(-3.0, 14.0));
    assert!((z - map.height(dvec2(0.0, 10.0))).abs() < 1e-12);
    assert!((off - 5.0).abs() < 1e-12);
}

#[test]
fn one_point_is_a_level_offset_and_one_row_is_level_across_it() {
    let one = ProbeMap::from_points(&[[4.0, 4.0, -0.3]], 1.0).unwrap();
    assert!(one.is_level());
    assert_eq!(one.height(dvec2(100.0, -50.0)), -0.3);
    let row = ProbeMap::from_points(&[[0.0, 5.0, 0.0], [10.0, 5.0, 1.0]], 1.0).unwrap();
    assert!((row.height(dvec2(2.5, 0.0)) - 0.25).abs() < 1e-12);
    assert!((row.height(dvec2(2.5, 9.0)) - 0.25).abs() < 1e-12);
}

#[test]
fn inches_are_read_as_millimetres() {
    let map = ProbeMap::from_points(&[[0.0, 0.0, 0.01], [1.0, 0.0, 0.02]], 25.4).unwrap();
    assert_eq!(map.bounds().1, dvec2(25.4, 0.0));
    assert!((map.height(dvec2(12.7, 0.0)) - 0.015 * 25.4).abs() < 1e-12);
}

#[test]
fn a_straight_cut_follows_the_map_a_segment_at_a_time() {
    let map = ProbeMap::from_points(&tilted(), 1.0).unwrap();
    let mut path = Toolpath::new();
    path.rapid_to(dvec3(0.0, 5.0, 5.0)).unwrap();
    path.feed_to(dvec3(0.0, 5.0, -0.2), MoveRole::Plunge, 100.0)
        .unwrap();
    path.feed_to(dvec3(20.0, 5.0, -0.2), MoveRole::Cut, 600.0)
        .unwrap();
    let c = correct(&path, &map, 1.0, 0.01).unwrap();
    assert_eq!(c.outside, 0);
    // The rapid and the plunge, then twenty pieces a millimetre long.
    assert_eq!(c.path.len(), 22);
    for m in c.path.moves() {
        let want = map.height(m.to.truncate());
        let depth = if m.role == MoveRole::Rapid { 5.0 } else { -0.2 };
        assert!((m.to.z - (want + depth)).abs() < 1e-12);
    }
    assert_eq!(c.path.moves()[1].role, MoveRole::Plunge);
    assert!(c.path.moves()[2..].iter().all(|m| m.role == MoveRole::Cut));
}

#[test]
fn an_arc_stays_one_over_a_level_map_and_is_chords_over_a_tilted_one() {
    let mut path = Toolpath::new();
    path.rapid_to(dvec3(15.0, 5.0, 0.0)).unwrap();
    let arc = Arc {
        centre: dvec3(10.0, 5.0, 0.0),
        axis: glam::DVec3::Z,
        sweep: PI,
    };
    path.arc_to(dvec3(5.0, 5.0, 0.0), arc, MoveRole::Cut, 600.0)
        .unwrap();

    let level = ProbeMap::from_points(&[[0.0, 0.0, 0.4]], 1.0).unwrap();
    let c = correct(&path, &level, 1.0, 0.01).unwrap();
    assert_eq!(c.path.len(), 2);
    let MoveKind::Arc(a) = c.path.moves()[1].kind else {
        panic!("not an arc")
    };
    assert_eq!(a.centre.z, 0.4);
    assert_eq!(c.path.moves()[1].to.z, 0.4);
    assert_eq!(c.outside, 0);

    let map = ProbeMap::from_points(&tilted(), 1.0).unwrap();
    let c = correct(&path, &map, 1.0, 0.01).unwrap();
    assert!(c.path.len() > 10);
    let mut at = c.path.moves()[0].to;
    for m in &c.path.moves()[1..] {
        assert_eq!(m.kind, MoveKind::Linear);
        assert!(at.truncate().distance(m.to.truncate()) <= 1.0 + 1e-9);
        // On the circle, within the tolerance, and on the map.
        let r = m.to.truncate().distance(DVec2::new(10.0, 5.0));
        assert!((r - 5.0).abs() <= 0.011);
        assert!((m.to.z - map.height(m.to.truncate())).abs() < 1e-12);
        at = m.to;
    }
}

#[test]
fn a_hole_is_moved_as_its_moves_are() {
    let map = ProbeMap::from_points(&tilted(), 1.0).unwrap();
    let mut holes = [Hole {
        at: dvec2(10.0, 10.0),
        top: 0.0,
        bottom: -3.0,
        radius: 2.0,
    }];
    correct_holes(&mut holes, &map);
    assert!((holes[0].top - 1.3).abs() < 1e-12);
    assert!((holes[0].bottom + 1.7).abs() < 1e-12);
}

#[test]
fn moves_outside_the_map_are_counted() {
    let map = ProbeMap::from_points(&tilted(), 1.0).unwrap();
    let mut path = Toolpath::new();
    path.rapid_to(dvec3(-2.0, 5.0, 5.0)).unwrap();
    path.feed_to(dvec3(-2.0, 5.0, 0.0), MoveRole::Plunge, 100.0)
        .unwrap();
    path.feed_to(dvec3(2.0, 5.0, 0.0), MoveRole::Cut, 600.0)
        .unwrap();
    let c = correct(&path, &map, 1.0, 0.01).unwrap();
    // The rapid, the plunge and the cut's first piece; its second ends on
    // the edge.
    assert_eq!(c.outside, 3);
    assert!((c.furthest - 2.0).abs() < 1e-12);
}
