use super::*;
use approx::assert_abs_diff_eq;
use proptest::prelude::*;

const EPS: f64 = 1e-9;

#[test]
fn rejects_unphysical_radii() {
    assert_eq!(Flat::new(0.0), Err(ToolError::BadRadius(0.0)));
    assert_eq!(Ball::new(-1.0), Err(ToolError::BadRadius(-1.0)));
    assert!(matches!(Ball::new(f64::NAN), Err(ToolError::BadRadius(_))));
    assert!(Flat::new(3.175).is_ok());
}

#[test]
fn flat_is_a_cylinder() {
    let t = Flat::new(3.0).unwrap();
    assert_eq!(t.radius_at(0.0), 3.0);
    assert_eq!(t.radius_at(50.0), 3.0);
    assert_eq!(t.radius_at(-1.0), 0.0);
    // The entire cutting face lies in one plane.
    assert_eq!(t.height_at(0.0), Some(0.0));
    assert_eq!(t.height_at(3.0), Some(0.0));
    assert_eq!(t.height_at(3.000_001), None);
}

#[test]
fn ball_matches_closed_form() {
    let r = 3.0;
    let t = Ball::new(r).unwrap();

    // Tip.
    assert_abs_diff_eq!(t.radius_at(0.0), 0.0, epsilon = EPS);
    assert_abs_diff_eq!(t.height_at(0.0).unwrap(), 0.0, epsilon = EPS);

    // Equator: at z = R the ball reaches full radius.
    assert_abs_diff_eq!(t.radius_at(r), r, epsilon = EPS);
    assert_abs_diff_eq!(t.height_at(r).unwrap(), r, epsilon = EPS);

    // 45 degrees around the ball: d = R/sqrt(2) sits at z = R - R/sqrt(2).
    let d = r / 2.0_f64.sqrt();
    assert_abs_diff_eq!(t.height_at(d).unwrap(), r - d, epsilon = EPS);

    // Above the ball it is a plain shank.
    assert_abs_diff_eq!(t.radius_at(10.0), r, epsilon = EPS);
}

/// The duality the whole drop-cutter rests on.
fn check_inverse<T: ToolProfile>(t: &T, d: f64) -> Result<(), TestCaseError> {
    let Some(z) = t.height_at(d) else {
        return Ok(());
    };
    prop_assert!(z >= 0.0, "height must be at or above the tip, got {z}");
    // h(d) = inf { z : r(z) >= d }, so at h(d) the tool has reached radius d...
    prop_assert!(
        t.radius_at(z) >= d - 1e-7,
        "r(h(d)) = {} should be >= d = {d}",
        t.radius_at(z)
    );
    // ...and just below it, it has not.
    if z > 1e-6 {
        prop_assert!(
            t.radius_at(z - 1e-6) <= d + 1e-7,
            "r just below h(d) should not yet have reached d"
        );
    }
    Ok(())
}

proptest! {
    #[test]
    fn ball_inverse_holds(r in 0.1f64..25.0, frac in 0.0f64..=1.0) {
        let t = Ball::new(r).unwrap();
        check_inverse(&t, frac * r)?;
    }

    #[test]
    fn flat_inverse_holds(r in 0.1f64..25.0, frac in 0.0f64..=1.0) {
        let t = Flat::new(r).unwrap();
        check_inverse(&t, frac * r)?;
    }

    /// `r(z)` must be monotonically non-decreasing for flat and ball mills. That is
    /// what makes `height_at` well defined, and it is exactly the assumption an
    /// undercutting tool will later break.
    #[test]
    fn radius_is_monotonic(r in 0.1f64..25.0, a in 0.0f64..50.0, b in 0.0f64..50.0) {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let ball = Ball::new(r).unwrap();
        let flat = Flat::new(r).unwrap();
        prop_assert!(ball.radius_at(lo) <= ball.radius_at(hi) + 1e-12);
        prop_assert!(flat.radius_at(lo) <= flat.radius_at(hi) + 1e-12);
        prop_assert!(ball.is_monotonic() && flat.is_monotonic());
    }

    /// Nothing may exceed the declared maximum radius; the drop-cutter bounds
    /// its search footprint with it, so an understated value means gouges.
    #[test]
    fn never_exceeds_max_radius(r in 0.1f64..25.0, z in -5.0f64..100.0) {
        let ball = Ball::new(r).unwrap();
        let flat = Flat::new(r).unwrap();
        prop_assert!(ball.radius_at(z) <= ball.max_radius() + 1e-12);
        prop_assert!(flat.radius_at(z) <= flat.max_radius() + 1e-12);
    }

    #[test]
    fn cone_inverse_holds(
        r in 0.1f64..25.0,
        tip in 0.0f64..0.9,
        half in 0.05f64..1.5,
        frac in 0.0f64..=1.0,
    ) {
        let t = Cone::new(r, tip * r, half).unwrap();
        check_inverse(&t, frac * r)?;
    }

    #[test]
    fn cone_is_monotonic_and_bounded(
        r in 0.1f64..25.0,
        tip in 0.0f64..0.9,
        half in 0.05f64..1.5,
        a in -5.0f64..100.0,
        b in -5.0f64..100.0,
    ) {
        let t = Cone::new(r, tip * r, half).unwrap();
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        prop_assert!(t.radius_at(lo) <= t.radius_at(hi) + 1e-12);
        prop_assert!(t.radius_at(hi) <= t.max_radius() + 1e-12);
    }
}

/// A 90° V-bit rises one unit for each unit out; a 60° one, √3.
#[test]
fn a_cone_matches_its_angle() {
    let v90 = Cone::new(3.0, 0.0, 45f64.to_radians()).unwrap();
    assert_abs_diff_eq!(v90.height_at(2.0).unwrap(), 2.0, epsilon = EPS);
    assert_abs_diff_eq!(v90.flank_height(), 3.0, epsilon = EPS);
    assert_abs_diff_eq!(v90.radius_at(1.5), 1.5, epsilon = EPS);
    assert_abs_diff_eq!(v90.radius_at(10.0), 3.0, epsilon = EPS);
    let v60 = Cone::new(3.0, 0.5, 30f64.to_radians()).unwrap();
    assert_eq!(v60.height_at(0.5), Some(0.0));
    assert_abs_diff_eq!(v60.height_at(1.5).unwrap(), 3f64.sqrt(), epsilon = EPS);
    assert_eq!(v60.height_at(3.01), None);
}

#[test]
fn a_cone_must_be_a_cone() {
    assert!(matches!(
        Cone::new(3.0, 0.0, 0.0),
        Err(ToolError::BadAngle(_))
    ));
    assert!(matches!(
        Cone::new(3.0, 0.0, std::f64::consts::FRAC_PI_2),
        Err(ToolError::BadAngle(_))
    ));
    assert!(matches!(
        Cone::new(3.0, 3.0, 0.5),
        Err(ToolError::BadTip { .. })
    ));
    assert!(matches!(
        Cone::new(0.0, 0.0, 0.5),
        Err(ToolError::BadRadius(_))
    ));
}

/// A tapered ball: its ball's tip, then a flank tangent to it at the taper,
/// out to the full radius; the corner meets the flank `c cos α` out and
/// `c (1 - sin α)` up, and from there it rises `1 / tan α` a unit out.
#[test]
fn a_tapered_ball_is_its_ball_then_its_flank() {
    let (c, a) = (0.5, 3f64.to_radians());
    let t = Tapered::new(3.0, 0.0, c, a).unwrap();
    assert_eq!(t.height_at(0.0), Some(0.0));
    let (rt, zt) = (c * a.cos(), c * (1.0 - a.sin()));
    assert_abs_diff_eq!(t.height_at(rt).unwrap(), zt, epsilon = EPS);
    // On the ball, as a ball of its radius.
    let ball = Ball::new(c).unwrap();
    assert_abs_diff_eq!(
        t.height_at(0.3).unwrap(),
        ball.height_at(0.3).unwrap(),
        epsilon = EPS
    );
    // On the flank, straight.
    let up = t.height_at(2.0).unwrap() - t.height_at(1.0).unwrap();
    assert_abs_diff_eq!(up, 1.0 / a.tan(), epsilon = 1e-9);
    assert_abs_diff_eq!(t.height_at(3.0).unwrap(), t.flank_top(), epsilon = 1e-9);
    assert_abs_diff_eq!(t.radius_at(t.flank_top()), 3.0, epsilon = 1e-9);
    assert_eq!(t.height_at(3.01), None);
    // Smooth where the ball meets the flank: its slope is the flank's.
    let e = 1e-6;
    let below = (t.height_at(rt).unwrap() - t.height_at(rt - e).unwrap()) / e;
    assert!((below - 1.0 / a.tan()).abs() < 1e-3 * below, "{below}");
}

/// With no corner a tapered end is a cone, its flat its tip.
#[test]
fn a_tapered_end_without_a_corner_is_a_cone() {
    let half = 20f64.to_radians();
    let t = Tapered::new(3.0, 0.4, 0.0, half).unwrap();
    let cone = Cone::new(3.0, 0.4, half).unwrap();
    for i in 0..=30 {
        let d = 0.1 * f64::from(i);
        assert_abs_diff_eq!(
            t.height_at(d).unwrap(),
            cone.height_at(d).unwrap(),
            epsilon = 1e-12
        );
        assert_abs_diff_eq!(t.radius_at(d), cone.radius_at(d), epsilon = 1e-12);
    }
}

#[test]
fn a_tapered_end_must_be_one() {
    assert!(matches!(
        Tapered::new(3.0, 0.0, 0.5, 0.0),
        Err(ToolError::BadTaper(_))
    ));
    assert!(matches!(
        Tapered::new(3.0, 2.8, 0.5, 0.1),
        Err(ToolError::BadTip { .. })
    ));
    assert!(matches!(
        Tapered::new(3.0, 0.0, -0.1, 0.1),
        Err(ToolError::BadCorner { .. })
    ));
}

/// Grown by stock to leave, a tapered end's corner grows by the stock and
/// its flat by the radial stock past the axial: the rounded cylinder's sum.
#[test]
fn a_tapered_end_grows_by_its_stock() {
    let t = Cutter::from(Tapered::new(3.0, 0.2, 0.5, 0.1).unwrap());
    let Some(Ok(Cutter::Tapered(g))) = t.grown(0.3, 0.3) else {
        panic!("not grown");
    };
    assert_abs_diff_eq!(g.corner(), 0.8, epsilon = EPS);
    assert_abs_diff_eq!(g.flat(), 0.2, epsilon = EPS);
    assert_abs_diff_eq!(g.max_radius(), 3.3, epsilon = EPS);
    let Some(Ok(Cutter::Tapered(g))) = t.grown(0.5, 0.2) else {
        panic!("not grown");
    };
    assert_abs_diff_eq!(g.corner(), 0.7, epsilon = EPS);
    assert_abs_diff_eq!(g.flat(), 0.5, epsilon = EPS);
    // A cone does not grow.
    let cone = Cutter::from(Cone::new(3.0, 0.0, 0.5).unwrap());
    assert!(cone.grown(0.1, 0.1).is_none());
}

/// A slot cutter is a disc on a neck: its radius drops to the neck above the
/// disc, so it is not monotonic, and from below it is a flat end its head's
/// size. A lollipop's ball narrows back in to its neck above its middle.
/// The material is swept only as wide as the neck.
#[test]
fn an_undercutting_head_stands_on_its_neck() {
    let slot = Undercut::new(Bullnose::new(5.0, 0.0).unwrap(), 2.0, 3.0).unwrap();
    assert_eq!(slot.radius_at(1.0), 5.0);
    assert_eq!(slot.radius_at(3.5), 2.0);
    assert!(!slot.is_monotonic());
    assert_eq!(slot.max_radius(), 5.0);
    assert_eq!(slot.height_at(4.0), Some(0.0));
    let cut = Cutter::from(slot);
    assert!(!cut.is_monotonic());
    assert_eq!(cut.swept_max_radius(), 2.0);
    assert_eq!(cut.swept_height_at(1.5), Some(0.0));
    assert_eq!(cut.swept_height_at(2.5), None);
    let neck = 1.5_f64;
    let top = 3.0 + (9.0 - neck * neck).sqrt();
    let pop = Undercut::new(Bullnose::new(3.0, 3.0).unwrap(), neck, top).unwrap();
    assert_abs_diff_eq!(pop.radius_at(3.0), 3.0, epsilon = EPS);
    assert_abs_diff_eq!(pop.radius_at(4.0), 8f64.sqrt(), epsilon = EPS);
    assert_abs_diff_eq!(pop.radius_at(top + 1.0), neck, epsilon = EPS);
    assert_abs_diff_eq!(pop.height_at(3.0).unwrap(), 3.0, epsilon = EPS);
    assert!(matches!(
        Undercut::new(Bullnose::new(3.0, 0.0).unwrap(), 3.0, 1.0),
        Err(ToolError::BadNeck { .. })
    ));
}

proptest! {
    #[test]
    fn tapered_inverse_holds(
        r in 0.5f64..25.0,
        flat in 0.0f64..0.4,
        corner in 0.0f64..0.4,
        half in 0.02f64..1.4,
        frac in 0.0f64..=1.0,
    ) {
        let t = Tapered::new(r, flat * r, corner * r, half).unwrap();
        check_inverse(&t, frac * r)?;
    }

    #[test]
    fn tapered_is_monotonic_and_bounded(
        r in 0.5f64..25.0,
        corner in 0.0f64..0.5,
        half in 0.02f64..1.4,
        a in -5.0f64..400.0,
        b in -5.0f64..400.0,
    ) {
        let t = Tapered::new(r, 0.0, corner * r, half).unwrap();
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        prop_assert!(t.radius_at(lo) <= t.radius_at(hi) + 1e-12);
        prop_assert!(t.radius_at(hi) <= t.max_radius() + 1e-12);
    }
}
