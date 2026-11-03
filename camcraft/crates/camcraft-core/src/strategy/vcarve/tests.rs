use camcraft_geom::contour::Contour;
use glam::{DVec2, dvec2};

use super::*;

/// A 60° V-bit 12 mm across, cutting into a top at 10.
fn params() -> VCarveParams {
    VCarveParams {
        top: 10.0,
        half_angle: 30f64.to_radians(),
        tip_radius: 0.0,
        radius: 6.0,
        max_depth: None,
        stepdown: None,
        feed: 1000.0,
        tolerance: 0.01,
    }
}

fn cut_points(path: &Toolpath) -> Vec<DVec3> {
    path.moves().iter().map(|m| m.to).collect()
}

/// Down the middle of a strip 4 mm wide, as deep as the V is 4 mm wide:
/// 2 / tan 30°, within the tolerance the walls may be reached past.
#[test]
fn a_strip_is_carved_down_its_middle_as_deep_as_it_is_wide() {
    let area = Region::rectangle(dvec2(0.0, 0.0), dvec2(20.0, 4.0));
    let (path, report) = vcarve(&area, &params()).unwrap();
    let want = 2.0 / 30f64.to_radians().tan();
    assert!((report.deepest - want).abs() < 0.01, "{}", report.deepest);
    assert_eq!(report.passes, 1);
    assert!(report.floor_mm2 < 1e-6);
    let points = cut_points(&path);
    // It starts at a corner, at the top.
    assert!(points[0].z > 10.0 - 0.01, "{:?}", points[0]);
    let low: Vec<&DVec3> = points
        .iter()
        .filter(|p| (p.z - (10.0 - report.deepest)).abs() < 1e-6)
        .collect();
    assert!(!low.is_empty());
    for p in &low {
        assert!((p.y - 2.0).abs() < 0.01, "{p:?}");
        assert!(p.x > 1.99 && p.x < 18.01, "{p:?}");
    }
    // Nowhere does the V reach past the strip: each point's depth is no more
    // than the room round it.
    for p in &points {
        let room = p.x.min(20.0 - p.x).min(p.y).min(4.0 - p.y);
        let width = (10.0 - p.z) * 30f64.to_radians().tan();
        assert!(width <= room + 0.01, "{p:?}: {width} in {room}");
    }
    // Long straight runs are one move.
    assert!(points.len() < 60, "{}", points.len());
}

/// A round hole's middle is its centre, as deep as its radius on a 90° V.
#[test]
fn a_circle_is_carved_at_its_centre_its_radius_deep() {
    let points: Vec<DVec2> = (0..256)
        .map(|k| {
            let a = f64::from(k) / 256.0 * std::f64::consts::TAU;
            dvec2(30.0 + 3.0 * a.cos(), 3.0 * a.sin())
        })
        .collect();
    let area = Region::new(vec![Contour::new(points).unwrap()]);
    let p = VCarveParams {
        half_angle: 45f64.to_radians(),
        ..params()
    };
    let (path, report) = vcarve(&area, &p).unwrap();
    assert!((report.deepest - 3.0).abs() < 0.01, "{}", report.deepest);
    for q in cut_points(&path) {
        assert!((q.truncate() - dvec2(30.0, 0.0)).length() < 0.02, "{q:?}");
        assert!((q.z - 7.0).abs() < 0.01, "{q:?}");
    }
}

/// Kept to 1 mm deep, a wide area's walls are cut that deep a millimetre in,
/// and the floor between is left, and measured.
#[test]
fn past_its_deepest_it_cuts_the_walls_and_leaves_the_floor() {
    let area = Region::rectangle(dvec2(0.0, 0.0), dvec2(20.0, 10.0));
    let p = VCarveParams {
        half_angle: 45f64.to_radians(),
        max_depth: Some(1.0),
        ..params()
    };
    let (path, report) = vcarve(&area, &p).unwrap();
    assert!((report.deepest - 1.0).abs() < 1e-9);
    assert!((report.widest - 1.0).abs() < 1e-9);
    assert!(
        (report.floor_mm2 - 18.0 * 8.0).abs() < 0.5,
        "{}",
        report.floor_mm2
    );
    for q in cut_points(&path) {
        assert!(q.z >= 9.0 - 1e-9, "{q:?}");
        if (q.z - 9.0).abs() < 1e-9 && q.x > 2.0 && q.x < 18.0 {
            assert!(
                (q.y - 1.0).abs() < 1e-6 || (q.y - 9.0).abs() < 1e-6,
                "{q:?}"
            );
        }
    }
}

/// With a stepdown, each level goes round as deep as the level, and the last
/// all the way.
#[test]
fn levels_go_round_no_deeper_than_each() {
    let area = Region::rectangle(dvec2(0.0, 0.0), dvec2(20.0, 4.0));
    let p = VCarveParams {
        stepdown: Some(1.5),
        ..params()
    };
    let (path, report) = vcarve(&area, &p).unwrap();
    // 3.46 deep: at 1.5, 3, then the bottom.
    assert_eq!(report.passes, 3);
    let mut lows = Vec::new();
    for pass in path.passes() {
        lows.push(pass.iter().map(|m| m.to.z).fold(f64::INFINITY, f64::min));
    }
    assert!((lows[0] - 8.5).abs() < 1e-9, "{lows:?}");
    assert!((lows[1] - 7.0).abs() < 1e-9, "{lows:?}");
    assert!((lows[2] - (10.0 - report.deepest)).abs() < 1e-9, "{lows:?}");
}
