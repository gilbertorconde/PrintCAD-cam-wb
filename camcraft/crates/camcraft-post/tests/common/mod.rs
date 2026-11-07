//! What the post's integration tests share: one path exercising every move
//! role and every kind of arc, the program it is posted as, and three holes.

#![allow(dead_code, clippy::unwrap_used)]

use camcraft_core::job::{Coolant, SpindleDirection, ThreadHand};
use camcraft_core::path::toolpath::Arc;
use camcraft_core::path::toolpath::{MoveRole, Toolpath};
use camcraft_core::strategy::drilling::{Hole, Tap};
use camcraft_post::op::{DrillCycles, ProgramSpec};
use camcraft_post::profile::{ProgramUnits, TapStyle};
use glam::{DVec3, dvec2, dvec3};

/// A path exercising every move role the engine currently produces, plus the
/// coordinate cases that formatting gets wrong: negatives, a value that rounds, and
/// a move that changes only one axis.
pub fn fixture() -> Toolpath {
    let mut t = Toolpath::new();
    t.rapid_to(dvec3(0.0, 0.0, 25.0)).unwrap();
    t.rapid_to(dvec3(-12.3456, 7.5, 25.0)).unwrap();
    t.feed_to(dvec3(-12.3456, 7.5, 1.0), MoveRole::Plunge, 250.0)
        .unwrap();
    t.feed_to(dvec3(-12.3456, 7.5, -0.0001), MoveRole::Plunge, 250.0)
        .unwrap();
    t.feed_to(dvec3(10.0, 7.5, -0.0001), MoveRole::Cut, 1200.0)
        .unwrap();
    t.feed_to(dvec3(10.0, 9.5, -0.0001), MoveRole::Cut, 1200.0)
        .unwrap();
    // A quarter turn round a corner, level; a full turn of helix down; and a
    // quarter turn in a vertical plane, as a lead.
    t.arc_to(
        dvec3(12.0, 11.5, -0.0001),
        Arc {
            centre: dvec3(12.0, 9.5, -0.0001),
            axis: DVec3::Z,
            sweep: -std::f64::consts::FRAC_PI_2,
        },
        MoveRole::Cut,
        1200.0,
    )
    .unwrap();
    t.arc_to(
        dvec3(12.0, 11.5, -1.0),
        Arc {
            centre: dvec3(12.0, 9.5, 0.0),
            axis: DVec3::Z,
            sweep: std::f64::consts::TAU,
        },
        MoveRole::Ramp,
        400.0,
    )
    .unwrap();
    t.arc_to(
        dvec3(13.0, 11.5, -2.0),
        Arc {
            centre: dvec3(13.0, 11.5, -1.0),
            axis: DVec3::Y,
            sweep: -std::f64::consts::FRAC_PI_2,
        },
        MoveRole::Lead,
        1200.0,
    )
    .unwrap();
    t.feed_to(dvec3(10.0, 9.5, -2.5), MoveRole::Cut, 800.0)
        .unwrap();
    t.feed_to(dvec3(10.0, 9.5, 25.0), MoveRole::Retract, 900.0)
        .unwrap();
    t
}

pub fn spec() -> ProgramSpec {
    ProgramSpec {
        op_name: "finish".into(),
        strategy: "parallel".into(),
        tool: 3,
        tool_comment: "6mm ball (carbide)".into(),
        wcs: 54,
        extended_offsets: 0,
        rpm: 18000,
        spin: SpindleDirection::Clockwise,
        coolant: Coolant::Flood,
        units: ProgramUnits::Mm,
        drill: None,
        taps: false,
        tilt: None,
        rotary: Vec::new(),
        next_tool: None,
        stock: None,
        before: Vec::new(),
        after: Vec::new(),
    }
}

/// Three holes, drilled as canned cycles on a controller that takes them: across
/// to the first at the clearance height, one line a hole returning there, then
/// G80. The path is ignored when the holes are given.
pub fn drilling(peck: Option<f64>, dwell: Option<f64>) -> ProgramSpec {
    let hole = |x: f64, y: f64| Hole {
        at: dvec2(x, y),
        top: 5.0,
        bottom: -1.5,
        radius: 3.0,
    };
    ProgramSpec {
        op_name: "holes".into(),
        strategy: "drill".into(),
        tool: 4,
        tool_comment: "6mm drill".into(),
        coolant: Coolant::Off,
        drill: Some(DrillCycles {
            holes: vec![hole(10.0, 10.0), hole(30.0, 10.0), hole(-12.345, 40.0)],
            peck,
            chip_break: None,
            dwell,
            tap: None,
            tap_style: TapStyle::Cycle,
            feed: 250.0,
            clearance: 20.0,
        }),
        ..spec()
    }
}

/// The three holes drilled in pecks of 2 mm, backing off 0.3 mm between to
/// break the chip.
pub fn chip_breaking() -> ProgramSpec {
    let mut spec = drilling(Some(2.0), None);
    if let Some(d) = &mut spec.drill {
        d.chip_break = Some(0.3);
    }
    spec
}

/// The three holes tapped M8 × 1.25 at 500 rev/min, so fed at 625 mm/min, the
/// spindle turning the thread's way.
pub fn tapping(hand: ThreadHand) -> ProgramSpec {
    let tap = Tap { pitch: 1.25, hand };
    let mut spec = ProgramSpec {
        op_name: "threads".into(),
        tool_comment: "M8 tap".into(),
        rpm: 500,
        spin: hand.spindle(),
        taps: true,
        ..drilling(None, None)
    };
    if let Some(d) = &mut spec.drill {
        d.tap = Some(tap);
        d.feed = tap.feed(500.0);
    }
    spec
}

/// The three holes tapped as [`tapping`] does, by rigid tapping (`G33.1`)
/// a hole at a time.
pub fn rigid_tapping(hand: ThreadHand) -> ProgramSpec {
    let mut spec = tapping(hand);
    if let Some(d) = &mut spec.drill {
        d.tap_style = TapStyle::G331;
    }
    spec
}

/// An engraving round a circle 5 mm across and along a line, half a
/// millimetre deep, ramping down the circle as a helix: its links made as
/// the linker would, straight down from above.
pub fn engraving() -> Toolpath {
    use camcraft_core::job::TraceDirection;
    use camcraft_core::strategy::engrave::{EngraveParams, Piece, Trace, engrave};
    let circle = Trace {
        start: dvec3(2.5, 0.0, 0.0),
        pieces: vec![
            Piece::Arc {
                centre: dvec2(0.0, 0.0),
                ccw: true,
                to: dvec3(-2.5, 0.0, 0.0),
            },
            Piece::Arc {
                centre: dvec2(0.0, 0.0),
                ccw: true,
                to: dvec3(2.5, 0.0, 0.0),
            },
        ],
    };
    let line = Trace {
        start: dvec3(5.0, -5.0, 0.0),
        pieces: vec![Piece::Line {
            to: dvec3(15.0, -5.0, 0.0),
        }],
    };
    let params = EngraveParams {
        top: 0.0,
        depth: 0.5,
        stepdown: None,
        ramp_angle: 5f64.to_radians(),
        feed: 800.0,
        ramp_feed: 400.0,
        plunge_feed: 250.0,
        direction: TraceDirection::Climb,
        reverse: false,
        levels: Default::default(),
        climb_right: true,
        tolerance: 0.01,
    };
    let (path, _) = engrave(&[circle, line], &params).unwrap();
    linked(&path)
}

/// `path`'s links made as the linker would, straight down from above, from
/// and back to 10 mm up.
fn linked(path: &Toolpath) -> Toolpath {
    let mut t = Toolpath::new();
    t.rapid_to(dvec3(0.0, 0.0, 10.0)).unwrap();
    for m in path.moves() {
        if m.role == MoveRole::Link {
            let at = t.end().unwrap();
            if at.z < 5.0 {
                t.feed_to(at.truncate().extend(5.0), MoveRole::Retract, 900.0)
                    .unwrap();
            }
            t.rapid_to(m.to.truncate().extend(5.0)).unwrap();
            t.feed_to(m.to, MoveRole::Plunge, 250.0).unwrap();
        } else {
            t.push(*m).unwrap();
        }
    }
    let end = t.end().unwrap();
    t.feed_to(end.truncate().extend(10.0), MoveRole::Retract, 900.0)
        .unwrap();
    t
}

/// A slot 2 mm deep, one way, a millimetre a level: along a line, then a
/// half turn to the left about (0, 5), each level from the start.
pub fn slot() -> Toolpath {
    use camcraft_core::job::TraceDirection;
    use camcraft_core::strategy::engrave::{EngraveParams, Levels, Piece, Trace, engrave};
    let hook = Trace {
        start: dvec3(-10.0, 0.0, 0.0),
        pieces: vec![
            Piece::Line {
                to: dvec3(0.0, 0.0, 0.0),
            },
            Piece::Arc {
                centre: dvec2(0.0, 5.0),
                ccw: true,
                to: dvec3(0.0, 10.0, 0.0),
            },
        ],
    };
    let params = EngraveParams {
        top: 0.0,
        depth: 2.0,
        stepdown: Some(1.0),
        ramp_angle: 5f64.to_radians(),
        feed: 800.0,
        ramp_feed: 400.0,
        plunge_feed: 250.0,
        direction: TraceDirection::AsDrawn,
        reverse: false,
        levels: Levels::OneWay,
        climb_right: true,
        tolerance: 0.01,
    };
    let (path, _) = engrave(&[hook], &params).unwrap();
    linked(&path)
}

/// A square pocket 3 mm deep, roughed with a flat end mill.
pub fn roughing() -> Toolpath {
    let mut t = Toolpath::new();
    t.rapid_to(dvec3(0.0, 0.0, 15.0)).unwrap();
    t.rapid_to(dvec3(20.0, 20.0, 15.0)).unwrap();
    t.feed_to(dvec3(20.0, 20.0, -3.0), MoveRole::Plunge, 300.0)
        .unwrap();
    for (x, y) in [(40.0, 20.0), (40.0, 40.0), (20.0, 40.0), (20.0, 20.0)] {
        t.feed_to(dvec3(x, y, -3.0), MoveRole::Cut, 1500.0).unwrap();
    }
    t.feed_to(dvec3(20.0, 20.0, 15.0), MoveRole::Retract, 1000.0)
        .unwrap();
    t
}

/// A setup of three operations on two tools: roughing with tool 1 and
/// flood, then finishing ([`fixture`], [`spec`]) and engraving
/// ([`engraving`]) both with tool 3, the engraving slower and dry.
pub fn setup() -> Vec<(Toolpath, ProgramSpec)> {
    vec![
        (
            roughing(),
            ProgramSpec {
                op_name: "rough".into(),
                strategy: "pocket".into(),
                tool: 1,
                tool_comment: "6mm flat (carbide)".into(),
                rpm: 16000,
                ..spec()
            },
        ),
        (fixture(), spec()),
        (
            engraving(),
            ProgramSpec {
                op_name: "engrave".into(),
                strategy: "engrave".into(),
                rpm: 12000,
                coolant: Coolant::Off,
                ..spec()
            },
        ),
    ]
}
