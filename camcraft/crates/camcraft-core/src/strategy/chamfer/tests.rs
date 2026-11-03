use glam::dvec3;

use super::*;

/// A 10 mm square at 5 mm up, counter-clockwise from the origin.
fn square() -> Trace {
    let to = |x, y| Piece::Line {
        to: dvec3(x, y, 5.0),
    };
    Trace {
        start: dvec3(0.0, 0.0, 5.0),
        pieces: vec![to(10.0, 0.0), to(10.0, 10.0), to(0.0, 10.0), to(0.0, 0.0)],
    }
}

/// A 90° cone puts a 1 mm chamfer 1 mm deep, and its tip the offset below;
/// its centre the cone's radius at the chamfer's foot from the edge.
#[test]
fn the_cone_goes_as_deep_as_the_chamfer_is_wide() {
    let quarter = std::f64::consts::FRAC_PI_4;
    let p = Place::new(1.0, 0.5, quarter, 0.0, 3.0).unwrap();
    assert!((p.depth - 1.5).abs() < 1e-12, "{p:?}");
    assert!((p.offset - 0.5).abs() < 1e-12, "{p:?}");
    assert!((p.reach - 1.5).abs() < 1e-12, "{p:?}");
    // A flat tip 0.4 across: the centre that much further off.
    let p = Place::new(1.0, 0.5, quarter, 0.2, 3.0).unwrap();
    assert!((p.offset - 0.7).abs() < 1e-12, "{p:?}");
    // A 60° V-bit: steeper, so deeper for the same width.
    let sixty = 30f64.to_radians();
    let p = Place::new(1.0, 0.0, sixty, 0.0, 3.0).unwrap();
    assert!((p.depth - 3f64.sqrt()).abs() < 1e-12, "{p:?}");
    // More cone than the tool has.
    let e = Place::new(2.0, 0.5, quarter, 0.0, 2.0).unwrap_err();
    assert!((e.needs - 5.0).abs() < 1e-12 && e.has == 4.0, "{e:?}");
    // A cone with no angle fits nothing.
    assert!(Place::new(1.0, 0.0, f64::NAN, 0.0, 3.0).is_err());
}

/// The part higher inside the square: its material is on the left of the
/// square as it runs counter-clockwise, on the right reversed; with the
/// part as high either side, it cannot tell.
#[test]
fn the_material_is_on_the_side_the_part_is_higher() {
    let inside = |p: DVec2| (p.x > 0.0 && p.x < 10.0 && p.y > 0.0 && p.y < 10.0).then_some(5.0);
    assert_eq!(
        material_side(&square(), &inside, 0.25, 0.01),
        Some(Side::Left)
    );
    assert_eq!(
        material_side(&square().reversed(), &inside, 0.25, 0.01),
        Some(Side::Right)
    );
    // A pocket: the part outside it.
    let outside = |p: DVec2| inside(p).map_or(Some(5.0), |_| Some(0.0));
    assert_eq!(
        material_side(&square(), &outside, 0.25, 0.01),
        Some(Side::Right)
    );
    let flat = |_: DVec2| Some(5.0);
    assert_eq!(material_side(&square(), &flat, 0.25, 0.01), None);
}

/// Chamfering the square's outside: the tool's centre goes round it the
/// offset out, its corners turned round arcs, clockwise to climb with a
/// clockwise spindle, at the edge's height. Flipped, it goes inside.
#[test]
fn the_centre_goes_beside_the_edge_away_from_the_material() {
    let left = |_: &Trace| Some(Side::Left);
    let (out, found) = centres(&[square()], &left, 1.0, false, true, 0.01);
    assert_eq!(found, Centres::default());
    let t = &out[0];
    assert!(t.closed(0.01));
    assert!(t.signed_area(0.01) < 0.0, "clockwise, climbing");
    // Out by a millimetre all round, its corners arcs about the square's.
    assert!((t.length(0.001) - (40.0 + std::f64::consts::TAU)).abs() < 1e-6);
    for p in t.points(0.001) {
        let d = (p.x.clamp(0.0, 10.0) - p.x).hypot(p.y.clamp(0.0, 10.0) - p.y);
        assert!((d - 1.0).abs() < 1e-6, "{p}");
        assert_eq!(p.z, 5.0);
    }
    assert_eq!(
        t.pieces
            .iter()
            .filter(|p| matches!(p, Piece::Arc { .. }))
            .count(),
        4
    );
    // Flipped: inside, a millimetre in.
    let (inside, _) = centres(&[square()], &left, 1.0, true, true, 0.01);
    let pts = inside[0].points(0.001);
    assert!(
        pts.iter()
            .all(|p| p.x > 0.99 && p.x < 9.01 && p.y > 0.99 && p.y < 9.01)
    );
    assert!((inside[0].length(0.001) - 32.0).abs() < 1e-6);

    // Not knowing, outside a closed curve, said.
    let unknown = |_: &Trace| None;
    let (guess, found) = centres(&[square()], &unknown, 1.0, false, true, 0.01);
    assert_eq!(found.guessed, 1);
    assert!((guess[0].length(0.001) - (40.0 + std::f64::consts::TAU)).abs() < 1e-6);

    // An open edge along +X with the material on its left, +Y: the centre
    // a millimetre down, run the other way to climb.
    let edge = Trace {
        start: dvec3(0.0, 0.0, 5.0),
        pieces: vec![Piece::Line {
            to: dvec3(10.0, 0.0, 5.0),
        }],
    };
    let (open, _) = centres(&[edge], &left, 1.0, false, true, 0.01);
    assert_eq!(open[0].start, dvec3(10.0, -1.0, 5.0));
    assert_eq!(open[0].end(), dvec3(0.0, -1.0, 5.0));
}
