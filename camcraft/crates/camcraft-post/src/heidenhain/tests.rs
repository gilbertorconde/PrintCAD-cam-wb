//! Klartext read back as the control reads it, written as its manual writes
//! it, and custom lines kept to what cannot move the machine.

use super::*;

fn refused(line: &str) -> bool {
    matches!(concern(line), Some(Concern::Refused(_)))
}

fn moves(steps: &[Step]) -> Vec<&Motion> {
    steps
        .iter()
        .filter_map(|s| match &s.action {
            Action::Move(m) => Some(m),
            _ => None,
        })
        .collect()
}

/// A program as Heidenhain's manual prints one: its cycle's parameters with
/// no `~`, called by `CYCL CALL` and by `M99` alone, and ended by `L M30`.
#[test]
fn reads_a_program_as_the_manual_writes_it() {
    let text = "0  BEGIN PGM PAR MM
1  BLK FORM 0.1 Z X+0 Y+0 Z-20
2  BLK FORM 0.2  X+100  Y+100  Z+0
3  TOOL CALL 5 Z S2222
4  L  Z+100 R0 FMAX M3
5  CYCL DEF 200 DRILLING
   Q200=+2 ;SET-UP CLEARANCE
   Q201=-20 ;DEPTH
   Q206=+150 ;FEED RATE FOR PLNGNG
   Q202=+5 ;PLUNGING DEPTH
   Q210=+0 ;DWELL TIME AT TOP
   Q203=+0 ;SURFACE COORDINATE
   Q204=+50 ;2ND SET-UP CLEARANCE
   Q211=+0 ;DWELL TIME AT DEPTH
   Q395=+0 ;DEPTH REFERENCE
6  L  X+50  Y+50 R0 FMAX
7  CYCL CALL
8  L  X+70 R0 FMAX
9  M99
10 L M30
11 END PGM PAR MM
";
    let steps = read(text).unwrap();
    assert!(steps.iter().any(|s| s.action == Action::Change(5)));
    assert!(steps.iter().any(|s| s.action
        == Action::SpindleOn {
            rpm: 2222,
            dir: SpindleDirection::Clockwise
        }));
    let m = moves(&steps);
    // Each hole down to Z-20 in pecks of 5 at 150 mm/min, then up to Z+50.
    let deepest: Vec<f64> = m
        .iter()
        .filter(|m| m.feed == Some(150.0))
        .filter_map(|m| m.to[2])
        .collect();
    assert_eq!(deepest.iter().filter(|z| **z == -20.0).count(), 2, "{m:?}");
    assert!(deepest.iter().all(|z| *z >= -20.0 && *z <= -5.0));
    let last = m.last().unwrap();
    assert_eq!(last.to, [Some(70.0), Some(50.0), Some(50.0)]);
    assert_eq!(steps.last().unwrap().action, Action::End);
}

/// A circle in the XY plane by its centre, one in XZ, and a helix by its
/// angle, as the manual's examples write them.
#[test]
fn reads_arcs_in_each_plane_and_helices() {
    let text = "0 BEGIN PGM ARCS MM
1 TOOL CALL 1 Z S4000
2 L X+45 Y+25 Z+0 R0 FMAX M3
3 CC X+25 Y+25
4 C X+5 Y+25 DR+ F200
5 CC X+15 Z+0
6 C X+25 Z+0 DR-
7 CC X+15 Y+25
8 CP IPA-1800 IZ+5 DR-
9 END PGM ARCS MM
";
    let m: Vec<Motion> = moves(&read(text).unwrap()).into_iter().cloned().collect();
    let half = &m[1];
    let t = half.arc.unwrap();
    assert_eq!(t.normal, DVec3::Z);
    assert!((t.sweep - PI).abs() < 1e-9, "{t:?}");
    assert_eq!(half.to, [Some(5.0), Some(25.0), Some(0.0)]);
    let upright = m[2].arc.unwrap();
    assert_eq!(upright.normal, DVec3::Y);
    assert!((upright.sweep + PI).abs() < 1e-9, "{upright:?}");
    let helix = &m[3];
    let t = helix.arc.unwrap();
    assert!((t.sweep + 5.0 * TAU).abs() < 1e-9);
    let to = known(&helix.to).unwrap();
    assert!(to.distance(dvec3(25.0, 25.0, 5.0)) < 1e-9, "{to}");
}

/// Feeds in an inch program are tenths of an inch a minute.
#[test]
fn inch_feeds_are_in_tenths() {
    let text = "0 BEGIN PGM IN INCH
1 TOOL CALL 1 Z S4000
2 L X+1 Y+1 Z+0 R0 FMAX
3 L X+2 R0 F100
4 END PGM IN INCH
";
    let m: Vec<Motion> = moves(&read(text).unwrap()).into_iter().cloned().collect();
    assert_eq!(m[1].feed, Some(254.0));
    assert_eq!(m[1].to[0], Some(50.8));
}

/// What it cannot read, it refuses, saying where.
#[test]
fn refuses_what_it_does_not_understand() {
    for (text, said) in [
        ("1 L X+1 R0 FMAX\n", "before BEGIN PGM"),
        (
            "0 BEGIN PGM A MM\n1 L X+1 RL F100\n2 END PGM A MM\n",
            "radius compensation",
        ),
        (
            "0 BEGIN PGM A MM\n1 APPR LT X+1\n2 END PGM A MM\n",
            "APPR blocks",
        ),
        (
            "0 BEGIN PGM A MM\nL X+1 R0 FMAX\n2 END PGM A MM\n",
            "no number",
        ),
        (
            "0 BEGIN PGM A MM\n1 C X+1 Y+0 DR+ F100\n2 END PGM A MM\n",
            "no CC",
        ),
        (
            "0 BEGIN PGM A MM\n1 CYCL DEF 262 THREAD MILLING\n2 END PGM A MM\n",
            "cycle 262",
        ),
        ("0 BEGIN PGM A MM\n1 L X+1 R0 FMAX\n", "no END PGM"),
        ("0 BEGIN PGM A MM\n1 END PGM A MM\n2 M30\n", "after END PGM"),
    ] {
        let e = read(text).unwrap_err();
        assert!(e.to_string().contains(said), "{text:?}: {e}");
    }
}

/// Custom lines may stop, turn the spindle and coolant, and say things; a
/// path, a tool, a cycle or an end are refused, a call warned about.
#[test]
fn custom_lines_are_kept_from_moving_the_machine() {
    for line in [
        "M0",
        "STOP",
        "M8",
        "5 M3",
        "; check the clamps",
        "* - clamps",
        "",
    ] {
        assert_eq!(concern(line), None, "{line}");
    }
    for line in [
        "L X+10 R0 FMAX",
        "12 L Z+5 R0 F100",
        "CC X+0 Y+0",
        "TOOL CALL 2 Z S1000",
        "CYCL DEF 200 DRILLING",
        "M30",
        "L M140 MB MAX",
        "M91",
    ] {
        assert!(refused(line), "{line}: {:?}", concern(line));
    }
    assert!(matches!(
        concern("CALL PGM CLAMP"),
        Some(Concern::Unread(_))
    ));
}

/// A name a control can hold, whatever the program was called.
#[test]
fn program_names_hold_only_what_a_control_takes() {
    assert_eq!(program_name("top-01-rough"), "top-01-rough");
    assert_eq!(program_name("a b(c)"), "a_b_c_");
    assert_eq!(program_name(""), "PGM");
}
