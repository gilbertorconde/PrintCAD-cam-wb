use super::*;
use glam::dvec2;

fn rim(edge: u32, centre: DVec3, radius: f64) -> Chain {
    let points = (0..16)
        .map(|i| {
            let a = std::f64::consts::TAU * f64::from(i) / 16.0;
            centre + dvec3(radius * a.cos(), radius * a.sin(), 0.0)
        })
        .collect();
    Chain {
        model: "plate".into(),
        edge,
        points,
        closed: true,
        ends: None,
    }
}

fn fit(c: &Chain) -> Option<(DVec3, f64)> {
    let n = c.points.len() as f64;
    let centre = c.points.iter().copied().sum::<DVec3>() / n;
    Some((centre, c.points[0].distance(centre)))
}

#[test]
fn a_rim_alone_is_drilled_to_the_bottom_asked_for() {
    let got = holes(&[rim(1, dvec3(5.0, 5.0, 10.0), 3.0)], fit, -1.0, 0.01).unwrap();
    assert_eq!(got.len(), 1);
    assert!(got[0].at.distance(dvec2(5.0, 5.0)) < 1e-9);
    assert_eq!((got[0].top, got[0].bottom), (10.0, -1.0));
    assert!((got[0].radius - 3.0).abs() < 1e-9);
}

/// Both rims of one hole make one hole, from the top one to the bottom one.
#[test]
fn both_rims_of_a_hole_set_its_depth() {
    let got = holes(
        &[
            rim(1, dvec3(0.0, 0.0, 4.0), 2.0),
            rim(2, dvec3(20.0, 0.0, 10.0), 2.0),
            rim(3, dvec3(0.0, 0.0, 10.0), 2.0),
        ],
        fit,
        -5.0,
        0.01,
    )
    .unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!((got[0].top, got[0].bottom), (10.0, 4.0));
    assert_eq!((got[1].top, got[1].bottom), (10.0, -5.0));
}

/// Two rims of one axis at one height, a counterbore's floor's two edges,
/// are one rim: drilled from there to the bottom asked for.
#[test]
fn rims_at_one_height_are_drilled_to_the_bottom_asked_for() {
    let got = holes(
        &[
            rim(1, dvec3(0.0, 0.0, 7.0), 5.0),
            rim(2, dvec3(0.0, 0.0, 7.0), 2.0),
        ],
        fit,
        -1.0,
        0.01,
    )
    .unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].top, got[0].bottom), (7.0, -1.0));
    assert!((got[0].radius - 5.0).abs() < 1e-9);
}

#[test]
fn what_is_not_a_hole_is_said() {
    assert_eq!(holes(&[], fit, 0.0, 0.01), Err(HoleError::NothingPicked));
    let square = rim(4, dvec3(0.0, 0.0, 5.0), 2.0);
    assert!(matches!(
        holes(std::slice::from_ref(&square), |_| None, 0.0, 0.01),
        Err(HoleError::NotRound { edge: 4, .. })
    ));
    let mut tilted = square;
    tilted.points[3].z += 1.0;
    assert!(matches!(
        holes(&[tilted], fit, 0.0, 0.01),
        Err(HoleError::NotVertical { .. })
    ));
    assert!(matches!(
        holes(&[rim(5, dvec3(0.0, 0.0, 5.0), 2.0)], fit, 6.0, 0.01),
        Err(HoleError::NoDepth { .. })
    ));
}

/// Pecks go down by their depth each time, back out to the R plane between,
/// and come back to just above where the last one stopped.
#[test]
fn pecks_back_out_between() {
    let hole = Hole {
        at: dvec2(1.0, 2.0),
        top: 10.0,
        bottom: 4.0,
        radius: 3.0,
    };
    let params = DrillParams {
        peck: Some(2.5),
        chip_break: None,
        feed: 200.0,
        retract_feed: 1000.0,
        tap: None,
    };
    let path = drill(&[hole], &params).unwrap();
    let downs: Vec<f64> = path
        .moves()
        .windows(2)
        .filter(|w| w[1].role == MoveRole::Retract)
        .map(|w| w[0].to.z)
        .collect();
    assert_eq!(downs, [7.5, 5.0, 4.0]);
    let lowest = path.moves().iter().map(|m| m.to.z).fold(f64::MAX, f64::min);
    assert_eq!(lowest, 4.0);
    assert_eq!(path.end().unwrap().z, 12.0, "ends at the R plane");
    assert!(path.moves().iter().all(|m| m.role != MoveRole::Cut));

    let once = drill(
        &[hole],
        &DrillParams {
            peck: None,
            ..params
        },
    )
    .unwrap();
    assert_eq!(once.len(), 3, "link, plunge, retract");
}

#[test]
fn holes_are_drilled_nearest_first() {
    let at = |x: f64| Hole {
        at: dvec2(x, 0.0),
        top: 1.0,
        bottom: 0.0,
        radius: 1.0,
    };
    let order: Vec<f64> = nearest_first(&[at(0.0), at(10.0), at(3.0), at(4.0)])
        .iter()
        .map(|h| h.at.x)
        .collect();
    assert_eq!(order, [0.0, 3.0, 4.0, 10.0]);
    assert!(nearest_first(&[]).is_empty());
}

/// Breaking the chip, each peck backs off only so far, and goes on down from
/// there; out of the hole only once it is at the bottom.
#[test]
fn chip_breaking_backs_off_a_little_between_pecks() {
    let hole = Hole {
        at: dvec2(1.0, 2.0),
        top: 10.0,
        bottom: 4.0,
        radius: 3.0,
    };
    let params = DrillParams {
        peck: Some(2.5),
        chip_break: Some(0.4),
        feed: 200.0,
        retract_feed: 1000.0,
        tap: None,
    };
    let path = drill(&[hole], &params).unwrap();
    let ups: Vec<(f64, f64)> = path
        .moves()
        .windows(2)
        .filter(|w| w[1].role == MoveRole::Retract)
        .map(|w| (w[0].to.z, w[1].to.z))
        .collect();
    assert_eq!(ups, [(7.5, 7.9), (5.0, 5.4), (4.0, 12.0)]);
    // Each peck from where it backed off, at the drilling feed.
    let downs: Vec<f64> = path
        .moves()
        .iter()
        .filter(|m| m.role == MoveRole::Plunge)
        .map(|m| m.to.z)
        .collect();
    assert_eq!(downs, [7.5, 5.0, 4.0]);
    assert!(path.moves().iter().all(|m| m.to.truncate() == hole.at));
}

/// A tap goes down to the bottom and back out to the R plane, both at the
/// feed its pitch makes, without pecking.
#[test]
fn a_tap_goes_in_and_out_at_its_feed() {
    let hole = Hole {
        at: dvec2(0.0, 0.0),
        top: 0.0,
        bottom: -12.0,
        radius: 4.0,
    };
    let tap = Tap {
        pitch: 1.25,
        hand: ThreadHand::Right,
    };
    assert_eq!(tap.feed(400.0), 500.0);
    let params = DrillParams {
        peck: None,
        chip_break: None,
        feed: tap.feed(400.0),
        retract_feed: 3000.0,
        tap: Some(tap),
    };
    let path = drill(&[hole], &params).unwrap();
    let moves: Vec<(MoveRole, f64, Option<f64>)> = path
        .moves()
        .iter()
        .map(|m| (m.role, m.to.z, m.feed))
        .collect();
    assert_eq!(
        moves,
        [
            (MoveRole::Link, 2.0, Some(500.0)),
            (MoveRole::Plunge, -12.0, Some(500.0)),
            (MoveRole::Retract, 2.0, Some(500.0)),
        ]
    );
}
