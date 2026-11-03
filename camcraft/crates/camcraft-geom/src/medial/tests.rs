use glam::dvec2;

use super::*;
use crate::contour::Contour;

fn rectangle(w: f64, h: f64) -> Region {
    Region::rectangle(dvec2(0.0, 0.0), dvec2(w, h))
}

/// A rectangle's middle is the line down its length, half its width from
/// either long side, and at its ends the corners' bisectors down to them.
#[test]
fn a_rectangles_middle_is_half_its_width_in() {
    let rings = inscribed(&rectangle(20.0, 4.0), 0.1, 10.0, 0.0);
    assert_eq!(rings.len(), 1);
    let touches = &rings[0];
    // Along the long sides, away from the ends, every circle is the width's.
    let mid: Vec<&Touch> = touches
        .iter()
        .filter(|t| t.at.y.abs() < 1e-9 || (t.at.y - 4.0).abs() < 1e-9)
        .filter(|t| t.at.x > 2.5 && t.at.x < 17.5)
        .collect();
    assert!(mid.len() > 100, "{}", mid.len());
    for t in mid {
        assert!((t.radius - 2.0).abs() < 1e-9, "{t:?}");
        assert!((t.centre.y - 2.0).abs() < 1e-9, "{t:?}");
        assert!(!t.capped);
    }
    // Every circle is inside, and none larger than the half width.
    for t in touches {
        assert!(t.radius <= 2.0 + 1e-9, "{t:?}");
        let c = t.centre;
        let room = c.x.min(20.0 - c.x).min(c.y).min(4.0 - c.y);
        assert!(room >= t.radius - 1e-9, "{t:?}");
    }
    // The corners are points.
    let least = touches
        .iter()
        .map(|t| t.radius)
        .fold(f64::INFINITY, f64::min);
    assert!(least < 1e-3, "{least}");
}

/// A circle's middle is its centre, as far from every point of it.
#[test]
fn a_circles_middle_is_its_centre() {
    let n = 360;
    let points: Vec<DVec2> = (0..n)
        .map(|k| {
            let a = f64::from(k) / f64::from(n) * std::f64::consts::TAU;
            dvec2(5.0 * a.cos(), 5.0 * a.sin())
        })
        .collect();
    let region = Region::new(vec![Contour::new(points).unwrap()]);
    let touches = &inscribed(&region, 0.2, 10.0, 0.01)[0];
    for t in touches {
        assert!(t.centre.length() < 0.01, "{t:?}");
        assert!((t.radius - 5.0).abs() < 0.01, "{t:?}");
    }
}

/// Wider than the most asked for, a circle that size touches the edge.
#[test]
fn a_capped_circle_touches_where_it_is_asked() {
    let touches = &inscribed(&rectangle(20.0, 10.0), 0.5, 1.5, 0.0)[0];
    let t = touches
        .iter()
        .find(|t| t.at.y.abs() < 1e-9 && (t.at.x - 10.0).abs() < 0.3)
        .unwrap();
    assert!(t.capped);
    assert_eq!(t.radius, 1.5);
    assert!((t.centre - dvec2(t.at.x, 1.5)).length() < 1e-9);
}

/// A hole is a wall too: round a ring's middle the circles touch both.
#[test]
fn a_ring_between_a_hole_and_its_outer_is_half_its_width() {
    let outer = Region::rectangle(dvec2(0.0, 0.0), dvec2(10.0, 10.0));
    let hole = Region::rectangle(dvec2(2.0, 2.0), dvec2(8.0, 8.0));
    let ring = outer.difference(&hole).unwrap();
    let rings = inscribed(&ring, 0.1, 5.0, 0.0);
    assert_eq!(rings.len(), 2);
    // Wider only across the corners.
    for t in rings.iter().flatten() {
        assert!(t.radius <= 2f64.sqrt() + 1e-9, "{t:?}");
    }
    let middle = rings[0]
        .iter()
        .find(|t| t.at.y.abs() < 1e-9 && (t.at.x - 5.0).abs() < 0.1)
        .unwrap();
    assert!((middle.radius - 1.0).abs() < 1e-9, "{middle:?}");
}
