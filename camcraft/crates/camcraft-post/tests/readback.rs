//! Every post read back as a controller would, against the path it was posted
//! from: each built-in profile in every way it can be set to write, in both
//! units, over paths with arcs of every size, way and plane. And programs
//! broken on purpose, one fault each, so the check is shown to catch what it
//! is there to catch.

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use camcraft_core::job::ThreadHand;
use camcraft_core::path::toolpath::{Arc, MoveRole, Toolpath};
use camcraft_post::op::{ProgramSpec, lower};
use camcraft_post::profile::{ArcStyle, Profile, ProgramUnits, ToolChangeStyle};
use camcraft_post::readback::check;
use camcraft_post::readback::check_setup;
use camcraft_post::render::render;
use camcraft_post::setup::{Joining, Section, lower_setup};
use common::{chip_breaking, drilling, fixture, setup, spec, tapping};
use glam::{DVec3, dvec3};

fn post(path: &Toolpath, spec: &ProgramSpec, profile: &Profile) -> String {
    render(&lower(path, spec).unwrap(), profile, "readback").0
}

fn reads_back(path: &Toolpath, spec: &ProgramSpec, profile: &Profile, what: &str) {
    let text = post(path, spec, profile);
    if let Err(p) = check(&text, path, spec, profile) {
        panic!("{what} does not read back: {p}\n{text}");
    }
}

/// Each built-in profile, and each way a profile can be set to write.
fn profiles() -> Vec<(String, Profile)> {
    let mut out = Vec::new();
    for name in Profile::builtin_names() {
        let p = Profile::builtin(name).unwrap().unwrap();
        let mut variant = |what: &str, change: &dyn Fn(&mut Profile)| {
            let mut v = p.clone();
            change(&mut v);
            out.push((format!("{name} {what}"), v));
        };
        variant("as built", &|_| {});
        variant("with radius arcs", &|v| v.arc_style = ArcStyle::R);
        variant("with centre arcs", &|v| v.arc_style = ArcStyle::Ij);
        variant("numbered", &|v| v.numbering.enabled = true);
        variant("with trailing zeros", &|v| {
            v.format.strip_trailing_zeros = false
        });
        variant("with nothing suppressed", &|v| {
            v.modal.suppress_motion = false;
            v.modal.suppress_axes = false;
            v.modal.suppress_feed = false;
        });
        variant("without going up or home", &|v| {
            v.safe_retract.clear();
            v.end_home.clear();
        });
        variant("changing by hand", &|v| {
            v.tool_change = ToolChangeStyle::Pause
        });
        variant("changing by M6", &|v| v.tool_change = ToolChangeStyle::M6);
        variant("with a length offset", &|v| v.length_offset = true);
        variant("without a length offset", &|v| v.length_offset = false);
    }
    out
}

/// Arcs a path can hold, chained: level, upright about X and about Y, each
/// way, from a hair of a turn to a whole one, small and large, level and as
/// helices.
fn arcs() -> Toolpath {
    let mut t = Toolpath::new();
    t.rapid_to(dvec3(0.0, 0.0, 30.0)).unwrap();
    t.feed_to(dvec3(0.0, 0.0, 0.0), MoveRole::Plunge, 300.0)
        .unwrap();
    let mut from = DVec3::ZERO;
    let sweeps = [0.05, 0.5, FRAC_PI_2, 2.0, PI, 4.0, 3.0 * FRAC_PI_2, TAU];
    let radii = [0.8, 6.0, 250.0];
    let mut i = 0u32;
    for axis in [DVec3::Z, DVec3::X, DVec3::Y, -DVec3::Z] {
        for &r in &radii {
            for &s in &sweeps {
                for sign in [1.0, -1.0] {
                    i += 1;
                    // The centre square to the axis from where the arc starts.
                    let side = axis.any_orthogonal_vector().normalize();
                    let centre = from + side * r;
                    // Every third a helix, moving along its axis as it turns.
                    let drop = if i.is_multiple_of(3) { -0.7 } else { 0.0 };
                    let arc = Arc {
                        centre,
                        axis,
                        sweep: sign * s,
                    };
                    let to = arc.point_at(from, from + axis * drop, 1.0);
                    t.arc_to(to, arc, MoveRole::Cut, 900.0 + 10.0 * f64::from(i % 4))
                        .unwrap();
                    from = to;
                    // A straight between some, so arcs follow lines too.
                    if i.is_multiple_of(5) {
                        from += dvec3(1.25, -0.5, 0.0);
                        t.feed_to(from, MoveRole::Cut, 1200.0).unwrap();
                    }
                }
            }
        }
    }
    t.feed_to(from + DVec3::Z * 30.0, MoveRole::Retract, 900.0)
        .unwrap();
    t
}

/// Every profile, set every way, in both units: the fixture and the arcs.
#[test]
fn every_post_reads_back_to_its_path() {
    let paths = [("the fixture", fixture()), ("the arcs", arcs())];
    for (name, profile) in profiles() {
        for units in [ProgramUnits::Mm, ProgramUnits::Inch] {
            let spec = ProgramSpec { units, ..spec() };
            for (what, path) in &paths {
                reads_back(
                    path,
                    &spec,
                    &profile,
                    &format!("{what} for {name} in {units:?}"),
                );
            }
        }
    }
}

/// A setup's operations as one program for `profile`, in `units`, with an
/// optional stop between them or not.
fn post_setup(ops: &[(Toolpath, ProgramSpec)], profile: &Profile) -> (String, Vec<ProgramSpec>) {
    let specs: Vec<ProgramSpec> = ops
        .iter()
        .map(|(_, s)| ProgramSpec {
            units: profile.units,
            ..s.clone()
        })
        .collect();
    let sections: Vec<Section> = ops
        .iter()
        .zip(&specs)
        .map(|((path, _), spec)| Section { path, spec })
        .collect();
    let (lowered, _) = lower_setup(&sections, Joining::of(profile)).unwrap();
    (render(&lowered, profile, "setup").0, specs)
}

fn sections<'a>(ops: &'a [(Toolpath, ProgramSpec)], specs: &'a [ProgramSpec]) -> Vec<Section<'a>> {
    ops.iter()
        .zip(specs)
        .map(|((path, _), spec)| Section { path, spec })
        .collect()
}

/// A setup of three operations on two tools as one program, every profile
/// set every way, in both units, with optional stops and without, and
/// holes drilled as cycles after them: each operation cut as it was posted,
/// the tool going between them only up, across clear and down.
#[test]
fn every_post_reads_back_a_setup_as_one_program() {
    let mut ops = setup();
    ops.push((Toolpath::new(), drilling(Some(2.0), None)));
    for (name, profile) in profiles() {
        for units in [ProgramUnits::Mm, ProgramUnits::Inch] {
            for stop in [false, true] {
                let mut p = profile.clone();
                p.units = units;
                p.optional_stop = stop;
                p.drill_cycles = true;
                let (text, specs) = post_setup(&ops, &p);
                if let Err(e) = check_setup(&text, &sections(&ops, &specs), &p) {
                    panic!(
                        "the setup for {name} in {units:?}, stop {stop}, does not read back: {e}\n{text}"
                    );
                }
            }
        }
    }
}

/// A setup's program broken on purpose, one fault each: each is caught,
/// and said of the operation it is in.
#[test]
fn a_setups_program_broken_is_refused() {
    let ops = setup();
    let fault = |profile: &Profile, from: &str, to: &str, what: &str| {
        let (text, specs) = post_setup(&ops, profile);
        let sections = sections(&ops, &specs);
        check_setup(&text, &sections, profile).unwrap();
        assert_eq!(text.matches(from).count(), 1, "{from:?} in\n{text}");
        let broken = text.replacen(from, to, 1);
        let e = check_setup(&broken, &sections, profile).unwrap_err();
        assert!(e.what.contains(what), "{from:?} -> {to:?}: {e}\n{broken}");
    };
    let linuxcnc = Profile::builtin("linuxcnc").unwrap().unwrap();
    // The second tool never changed to.
    fault(
        &linuxcnc,
        "T3 M6\n",
        "",
        "finish (operation 2): moves before tool 3",
    );
    // The engraving cut at the finishing's speed.
    fault(
        &linuxcnc,
        "S12000 M3\n",
        "",
        "engrave (operation 3): cuts before the spindle",
    );
    // Or with the coolant still on.
    fault(&linuxcnc, "S12000 M3\nM9\n", "S12000 M3\n", "coolant");
    // Or its first operation's moves left out.
    fault(&linuxcnc, "Y40\n", "", "rough (operation 1)");
    // A tool changed with the spindle turning.
    fault(
        &linuxcnc,
        "M9\nM5\nG53 G0 Z0\n(finish",
        "M9\nG53 G0 Z0\n(finish",
        "without stopping",
    );
    // With no way up of its own, the tool goes up and across between
    // operations: across low, or down before it is over the next, is
    // refused.
    let mut low = linuxcnc.clone();
    low.safe_retract.clear();
    fault(
        &low,
        "G0 X0 Y0\n(engrave",
        "G0 X0 Y0 Z10\n(engrave",
        "across and down",
    );
    fault(
        &low,
        "G0 X0 Y0\n(engrave - engrave - T3 G54)\nS12000 M3\nM9\nZ10\n",
        "G0 Z10\n(engrave - engrave - T3 G54)\nS12000 M3\nM9\nX0 Y0\n",
        "down between operations",
    );
    // In Klartext, the second tool never called, or the engraving's speed
    // never set.
    let heidenhain = Profile::builtin("heidenhain").unwrap().unwrap();
    fault(
        &heidenhain,
        "TOOL CALL 3 Z S18000",
        ";gone",
        "moves before tool 3",
    );
    fault(
        &heidenhain,
        "TOOL CALL Z S12000",
        ";gone",
        "engrave (operation 3)",
    );
    // An optional stop with the spindle turning.
    let mut stop = linuxcnc;
    stop.optional_stop = true;
    fault(
        &stop,
        "M5\nG53 G0 Z0\nM1\n(finish",
        "G53 G0 Z0\nM1\n(finish",
        "spindle",
    );
}

/// Holes as canned cycles, plain, with a pause, pecked, breaking the chip and
/// tapped either hand, read back as the holes they drill, in every profile
/// that is set to take cycles.
#[test]
fn drilling_cycles_read_back_to_their_holes() {
    for (name, mut profile) in profiles() {
        profile.drill_cycles = true;
        for units in [ProgramUnits::Mm, ProgramUnits::Inch] {
            let specs = [
                ("plain", drilling(None, None)),
                ("pecked", drilling(Some(2.0), None)),
                ("with a pause", drilling(None, Some(0.5))),
                ("chip breaking", chip_breaking()),
                ("tapped", tapping(ThreadHand::Right)),
                ("tapped left-hand", tapping(ThreadHand::Left)),
            ];
            for (what, spec) in specs {
                let spec = ProgramSpec { units, ..spec };
                reads_back(
                    &Toolpath::new(),
                    &spec,
                    &profile,
                    &format!("holes {what} for {name} in {units:?}"),
                );
            }
        }
    }
}

/// The fixture posted for `profile`, with one fault put in: `from` (which
/// must be there, once, as written) replaced by `to`.
fn broken(
    profile: &str,
    units: ProgramUnits,
    from: &str,
    to: &str,
) -> (String, Toolpath, ProgramSpec, Profile) {
    let profile = Profile::builtin(profile).unwrap().unwrap();
    let path = fixture();
    let spec = ProgramSpec { units, ..spec() };
    let text = post(&path, &spec, &profile);
    assert_eq!(
        text.matches(from).count(),
        1,
        "{from:?} is not in the program once:\n{text}"
    );
    check(&text, &path, &spec, &profile).unwrap();
    (text.replace(from, to), path, spec, profile)
}

/// Each fault is caught, and said: the first thing the program does wrong.
#[test]
fn faults_are_caught() {
    use ProgramUnits::{Inch, Mm};
    let faults: &[(&str, ProgramUnits, &str, &str, &str)] = &[
        // An arc the wrong way round.
        (
            "masso",
            Mm,
            "G17 G2 X12. Y11.5",
            "G17 G3 X12. Y11.5",
            "strays",
        ),
        // Its centre on the other side.
        ("masso", Mm, "I2. J0.", "I-2. J0.", "strays"),
        // A lead in the wrong plane.
        ("masso", Mm, "G18 G2", "G19 G2", "not a centre offset"),
        // Inches read as millimetres.
        (
            "masso-inch",
            Inch,
            "N15 G20",
            "N15 G21",
            "on its way to the path's start",
        ),
        // A cut made a rapid.
        (
            "grbl",
            Mm,
            "G1 Z1 F250",
            "G0 Z1 F250",
            "a rapid where the path cuts",
        ),
        // The wrong feed.
        ("grbl", Mm, "X10 F1200", "X10 F1300", "feeds at 1300.000"),
        // A cut left out.
        (
            "masso-inch",
            Inch,
            "N85 Y0.374\n",
            "",
            "where move 5 of the path goes",
        ),
        ("grbl", Mm, "Y9.5\n", "", "where move 5 of the path goes"),
        // Not raised before the tool change, so across at a height not known.
        (
            "masso",
            Mm,
            "N25 G28 G91 Z0\nN30 G90\n",
            "",
            "goes across at a height not known",
        ),
        // Across and down together from nowhere known.
        (
            "grbl",
            Mm,
            "G0 Z25\nX0 Y0",
            "G0 X0 Y0 Z25",
            "across and down at once",
        ),
        // The wrong tool, the wrong offset, the wrong length.
        ("centroid-avid", Mm, "T3 M6", "T4 M6", "before tool 3 is in"),
        ("centroid-avid", Mm, "\nG54\n", "\nG55\n", "G54 is selected"),
        (
            "centroid-avid",
            Mm,
            "G43 Z25 H3",
            "G43 Z25 H4",
            "tool 4's length",
        ),
        // Z not sent home before the change, so across at a height not known.
        (
            "centroid-avid",
            Mm,
            "M5\nG53 G0 Z0\n(6MM",
            "M5\n(6MM",
            "height not known",
        ),
        // The spindle not stopped for the change, not started, at the wrong
        // speed; the coolant left off.
        (
            "centroid-avid",
            Mm,
            "M5\nG53 G0 Z0\n(6MM",
            "G53 G0 Z0\n(6MM",
            "without stopping the spindle",
        ),
        ("grbl", Mm, "S18000 M3\n", "", "before the spindle turns"),
        (
            "centroid-avid",
            Inch,
            "S18000 M3",
            "S12000 M3",
            "before the spindle turns",
        ),
        ("masso", Mm, "N50 M8\n", "", "coolant not Flood"),
        // Down after the last cut, and an end with the spindle on.
        (
            "grbl",
            Mm,
            "M9\n",
            "M9\nG0 Z0\n",
            "goes down after the path has ended",
        ),
        (
            "centroid-avid",
            Mm,
            "M5\nG53 G0 Z0\nG0 X0 Y0",
            "G53 G0 Z0\nG0 X0 Y0",
            "ends with the spindle on",
        ),
        // No end at all.
        ("masso", Mm, "N155 M30\n", "", "no end"),
        // Something the reader does not know.
        (
            "grbl",
            Mm,
            "G90 G94 G17",
            "G90 G94 G17 G41",
            "G41 is not understood",
        ),
        // Mach told arc centres are absolute.
        (
            "mach",
            Mm,
            "G90 G94 G91.1",
            "G90 G94 G90.1",
            "not a whole number",
        ),
        // Haas's smoothing tolerance with no smoothing.
        ("haas", Mm, "G90 G94 G17", "G90 G94 G17 E0.01", "E word"),
        // Klartext: an arc the wrong way round, its centre elsewhere.
        (
            "heidenhain",
            Mm,
            "C X+12 Y+11.5 DR-",
            "C X+12 Y+11.5 DR+",
            "strays",
        ),
        ("heidenhain", Mm, "CC X+12 Y+9.5", "CC X+11 Y+9.5", "strays"),
        // A helix the wrong way, and turning against its DR.
        (
            "heidenhain",
            Mm,
            "IPA+180 IZ-0.5 DR+ F400",
            "IPA-180 IZ-0.5 DR- F400",
            "strays",
        ),
        (
            "heidenhain",
            Mm,
            "IPA+180 IZ-0.5 DR+ F400",
            "IPA+180 IZ-0.5 DR- F400",
            "IPA and DR turn different ways",
        ),
        // A cut made a rapid, the wrong feed, in tenths of an inch.
        (
            "heidenhain",
            Mm,
            "L X+10 R0 F1200",
            "L X+10 R0 FMAX",
            "a rapid where the path cuts",
        ),
        (
            "heidenhain-inch",
            Inch,
            "L X+0.3937 R0 F472.4",
            "L X+0.3937 R0 F47.2",
            "feeds at 119.888",
        ),
        // The wrong tool, the wrong preset, inches read as millimetres.
        (
            "heidenhain",
            Mm,
            "TOOL CALL 3 Z",
            "TOOL CALL 4 Z",
            "before tool 3 is in",
        ),
        ("heidenhain", Mm, "Q339=+1", "Q339=+2", "G54 is selected"),
        (
            "heidenhain-inch",
            Inch,
            "BEGIN PGM readback INCH",
            "BEGIN PGM readback MM",
            "on its way to the path's start",
        ),
        // Not raised before the change; the spindle not started; no end.
        (
            "heidenhain",
            Mm,
            "4 L Z+0 R0 FMAX M91\n",
            "",
            "goes across at a height not known",
        ),
        ("heidenhain", Mm, "8 M3\n", "", "before the spindle turns"),
        (
            "heidenhain",
            Mm,
            "29 END PGM readback MM\n",
            "",
            "no END PGM",
        ),
    ];
    for &(profile, units, from, to, said) in faults {
        let (text, path, spec, profile) = broken(profile, units, from, to);
        match check(&text, &path, &spec, &profile) {
            Ok(()) => panic!("{from:?} made {to:?} was not caught:\n{text}"),
            Err(p) => assert!(
                p.to_string().contains(said),
                "{from:?} made {to:?} was caught as {p:?}, not {said:?}\n{text}"
            ),
        }
    }
}

/// Faults in drilling: back only to R between holes, pecks too deep, a hole
/// not drilled to its bottom, a rapid into what is not drilled.
#[test]
fn drilling_faults_are_caught() {
    let mut profile = Profile::builtin("masso").unwrap().unwrap();
    profile.drill_cycles = true;
    let faults: &[(Option<f64>, &str, &str, &str)] = &[
        (
            Some(2.0),
            "N60 G98",
            "N60 G99",
            "below the clearance height",
        ),
        (
            Some(2.0),
            "X10. Y10. Z-1.5 R7. Q2.",
            "X10. Y10. Z-1.5 R7. Q5.",
            "pecks deeper",
        ),
        (
            None,
            "X30. Y10. Z-1.5",
            "X30. Y10. Z-1.",
            "before hole 2 is drilled to its bottom",
        ),
        (
            None,
            "X30. Y10. Z-1.5 R7.",
            "X30. Y10. Z-1.5 R-1.",
            "rapids to Z-1.000 in hole 2",
        ),
        (None, "X30. Y10.", "X31. Y10.", "where hole 2 is"),
    ];
    for &(peck, from, to, said) in faults {
        let spec = drilling(peck, None);
        let text = post(&Toolpath::new(), &spec, &profile);
        assert_eq!(text.matches(from).count(), 1, "{from:?}:\n{text}");
        check(&text, &Toolpath::new(), &spec, &profile).unwrap();
        let text = text.replace(from, to);
        match check(&text, &Toolpath::new(), &spec, &profile) {
            Ok(()) => panic!("{from:?} made {to:?} was not caught:\n{text}"),
            Err(p) => assert!(p.to_string().contains(said), "{p}\n{text}"),
        }
    }
}

/// Faults in breaking the chip and in tapping: pecks too deep, a tap fed
/// faster than its pitch a turn, a tap backed out at a rapid, a tap in with
/// the spindle turning against it.
#[test]
fn chip_breaking_and_tapping_faults_are_caught() {
    let mut profile = Profile::builtin("masso").unwrap().unwrap();
    profile.drill_cycles = true;
    let faults: &[(ProgramSpec, &str, &str, &str)] = &[
        (
            chip_breaking(),
            "X10. Y10. Z-1.5 R7. Q2.",
            "X10. Y10. Z-1.5 R7. Q4.",
            "pecks deeper",
        ),
        (
            tapping(ThreadHand::Right),
            "G84 X30. Y10. Z-1.5 R7. F625.",
            "G84 X30. Y10. Z-1.5 R7. F700.",
            "drills at 700.000",
        ),
        (
            tapping(ThreadHand::Right),
            "G84 X30. Y10.",
            "G81 X30. Y10.",
            "rapids out of tapped hole 2",
        ),
        (
            tapping(ThreadHand::Right),
            "S500 M3",
            "S500 M4",
            "G84 without the spindle turning M3",
        ),
        (
            tapping(ThreadHand::Left),
            "G74 X10. Y10.",
            "G84 X10. Y10.",
            "G84 without the spindle turning M3",
        ),
    ];
    for (spec, from, to, said) in faults {
        let text = post(&Toolpath::new(), spec, &profile);
        assert_eq!(text.matches(from).count(), 1, "{from:?}:\n{text}");
        check(&text, &Toolpath::new(), spec, &profile).unwrap();
        let text = text.replace(from, to);
        match check(&text, &Toolpath::new(), spec, &profile) {
            Ok(()) => panic!("{from:?} made {to:?} was not caught:\n{text}"),
            Err(p) => assert!(p.to_string().contains(said), "{p}\n{text}"),
        }
    }
    // A tap whose feed is not its pitch a turn is caught before the program
    // is read.
    let mut fast = tapping(ThreadHand::Right);
    let text = post(&Toolpath::new(), &fast, &profile);
    if let Some(d) = &mut fast.drill {
        d.feed = 700.0;
    }
    let p = check(&text, &Toolpath::new(), &fast, &profile).unwrap_err();
    assert!(p.to_string().contains("1.250 mm a turn"), "{p}");
}

/// The path holes are drilled along, each way the engine drills them, joined
/// at a clearance height.
fn drilled(peck: Option<f64>, chip_break: Option<f64>) -> Toolpath {
    use camcraft_core::strategy::drilling::{DrillParams, Hole, drill};
    let holes: Vec<Hole> = [(10.0, 10.0), (30.0, 10.0), (-12.345, 40.0)]
        .into_iter()
        .map(|(x, y)| Hole {
            at: glam::dvec2(x, y),
            top: 5.0,
            bottom: -1.5,
            radius: 3.0,
        })
        .collect();
    let params = DrillParams {
        peck,
        chip_break,
        feed: 250.0,
        retract_feed: 900.0,
        tap: None,
    };
    let mut t = Toolpath::new();
    for m in drill(&holes, &params).unwrap().moves() {
        if m.role == MoveRole::Link {
            t.rapid_to(dvec3(m.to.x, m.to.y, 20.0)).unwrap();
            t.rapid_to(m.to).unwrap();
        } else {
            t.feed_to(m.to, m.role, m.feed.unwrap()).unwrap();
        }
    }
    t
}

/// Holes posted as their moves, for a controller without cycles, read back
/// to the path: pecked and breaking the chip. A tapped hole is not posted as
/// moves at all.
#[test]
fn holes_as_moves_read_back_and_taps_are_refused() {
    let paths = [
        ("pecked", drilled(Some(2.0), None)),
        ("chip breaking", drilled(Some(2.0), Some(0.3))),
    ];
    for (name, profile) in profiles() {
        for units in [ProgramUnits::Mm, ProgramUnits::Inch] {
            let spec = ProgramSpec { units, ..spec() };
            for (what, path) in &paths {
                reads_back(
                    path,
                    &spec,
                    &profile,
                    &format!("holes {what} as moves for {name} in {units:?}"),
                );
            }
            let taps = ProgramSpec {
                units,
                drill: None,
                ..tapping(ThreadHand::Right)
            };
            assert_eq!(
                lower(&paths[0].1, &taps),
                Err(camcraft_post::op::LowerError::TapsAsMoves),
                "{name}"
            );
        }
    }
}

/// An arc too short across for its ends to stay apart once rounded would
/// read as a full circle; it is written as straight moves instead, as Fusion
/// writes it, and reads back to the arc.
#[test]
fn an_arc_too_short_to_write_reads_back_to_itself() {
    let mut t = Toolpath::new();
    t.rapid_to(dvec3(0.0, 0.0, 10.0)).unwrap();
    t.feed_to(DVec3::ZERO, MoveRole::Plunge, 300.0).unwrap();
    // A hair of a turn, either way; and whole turns of a tiny helix.
    for (r, sweep, drop) in [
        (0.8, 0.0005, 0.0),
        (5.0, 0.0001, 0.0),
        (0.8, -0.0004, 0.0),
        (0.1, TAU, -0.2),
    ] {
        let from = t.end().unwrap();
        let arc = Arc {
            centre: from + DVec3::X * r,
            axis: DVec3::Z,
            sweep,
        };
        let to = arc.point_at(from, from + DVec3::Z * drop, 1.0);
        t.arc_to(to, arc, MoveRole::Cut, 1000.0).unwrap();
    }
    t.feed_to(dvec3(5.0, 0.0, -0.2), MoveRole::Cut, 1000.0)
        .unwrap();
    for (name, profile) in profiles() {
        for units in [ProgramUnits::Mm, ProgramUnits::Inch] {
            let spec = ProgramSpec { units, ..spec() };
            let text = post(&t, &spec, &profile);
            let arc = |w: &str| matches!(w, "G2" | "G3" | "G02" | "G03");
            assert!(
                !text.split_whitespace().any(arc),
                "{name} wrote a short arc as an arc:\n{text}"
            );
            reads_back(
                &t,
                &spec,
                &profile,
                &format!("short arcs for {name} in {units:?}"),
            );
        }
    }
}

/// A tilted setup posted for a machine with an A axis reads back for every
/// profile and every way it can be set to write, the A positioned before it
/// cuts; and each fault in the positioning is caught.
#[test]
fn rotary_positioning_reads_back_and_its_faults_are_caught() {
    let (s, c) = 30f64.to_radians().sin_cos();
    let tool = dvec3(0.0, -s, c);
    let tilted = |p: &mut Profile| {
        p.rotary = Some(toml::from_str("axes = [\"A\"]\n").unwrap());
        p.rotary.as_ref().unwrap().position(tool, DVec3::X).unwrap()
    };
    for (what, mut p) in profiles() {
        // A profile with rotaries has a safe retract: it is refused without.
        if p.safe_retract.is_empty() {
            continue;
        }
        let spec = ProgramSpec {
            units: p.units,
            tilt: Some(tool),
            rotary: tilted(&mut p),
            ..spec()
        };
        reads_back(&fixture(), &spec, &p, &what);
    }

    let mut p = Profile::builtin("masso").unwrap().unwrap();
    let spec = ProgramSpec {
        tilt: Some(tool),
        rotary: tilted(&mut p),
        ..spec()
    };
    let path = fixture();
    let text = post(&path, &spec, &p);
    check(&text, &path, &spec, &p).unwrap();
    let fault = |from: &str, to: &str, spec: &ProgramSpec, what: &str| {
        assert_eq!(text.matches(from).count(), 1, "{from:?} in\n{text}");
        let broken = text.replace(from, to);
        let e = check(&broken, &path, spec, &p).unwrap_err();
        assert!(e.what.contains(what), "{from:?} -> {to:?}: {e}");
    };
    fault("N45 G0 A-30.\n", "", &spec, "cuts before A is at -30");
    fault("G0 A-30.", "G0 A-20.", &spec, "cuts before A is at -30");
    fault("G0 A-30.", "G0 A-30. X0", &spec, "positioning");
    fault("G0 A-30.", "G1 A-30. F100", &spec, "rapid");
    fault("N25 G28 G91 Z0\n", "", &spec, "before the tool is clear");
    let upright = ProgramSpec {
        rotary: Vec::new(),
        ..spec.clone()
    };
    fault("G0 A-30.", "G0 A-30.", &upright, "none to position");
    // Turned once it is cutting, it is refused.
    fault(
        "N90 Y9.5\n",
        "N90 Y9.5\nG0 A0\n",
        &spec,
        "in the middle of the path",
    );
}

/// A profile with rotaries and no way to send the tool clear is refused,
/// since the part would turn under it.
#[test]
fn rotaries_need_a_safe_retract() {
    let text = include_str!("../profiles/grbl.toml").to_string() + "\n[rotary]\naxes = [\"A\"]\n";
    let mut p: toml::Table = toml::from_str(&text).unwrap();
    p.insert("safe_retract".into(), toml::Value::Array(Vec::new()));
    let e = Profile::from_toml(&toml::to_string(&p).unwrap()).unwrap_err();
    assert!(e.to_string().contains("safe_retract"), "{e}");
}

/// Custom G-code is not read, so a person's lines nothing here understands
/// read back, in every profile; but what moves the machine or changes a mode
/// is refused in it, on its line; a block written where the program has
/// none, or left out, is caught; and after it the program must set the
/// tool, the work offset and the spindle again before it cuts.
#[test]
fn custom_gcode_reads_back_and_cannot_hide_a_move() {
    use camcraft_post::op::CustomGcode;
    let path = fixture();
    let custom = |text: &str| ProgramSpec {
        before: vec![CustomGcode::new("Probe", text)],
        after: vec![CustomGcode::new("End", "M64 P1\n#100 = 5")],
        ..spec()
    };
    let unread = custom("o100 call [1]\nM98 P2000\n(MSG, @operation@)\n#<depth> = 3\nM0");
    for (what, profile) in profiles() {
        reads_back(
            &path,
            &unread,
            &profile,
            &format!("{what} with custom G-code"),
        );
    }

    let profile = Profile::builtin("linuxcnc").unwrap().unwrap();
    for bad in ["G0 Z5", "M0\nG91", "X10", "G28 Z0", "M6 T2", "M30", "G20"] {
        let spec = custom(bad);
        let text = post(&path, &spec, &profile);
        let p = check(&text, &path, &spec, &profile).unwrap_err();
        assert!(p.what.starts_with("custom G-code: "), "{bad}: {p}");
        let line = text.lines().nth(p.line.unwrap() - 1).unwrap();
        assert!(bad.lines().any(|b| b == line), "{bad}: on {line:?}");
    }

    // Posted with a block, checked as if it had none; and the other way.
    let text = post(&path, &unread, &profile);
    assert!(check(&text, &path, &spec(), &profile).is_err());
    let plain = post(&path, &spec(), &profile);
    let p = check(&plain, &path, &unread, &profile).unwrap_err();
    assert!(p.what.contains("custom G-code"), "{p}");

    // Moved after the operation's start, which sets everything again, to
    // just before its first move: the tool, the work offset and the spindle
    // it set are no longer known to be as they were.
    let mut lines: Vec<&str> = text.lines().collect();
    let begin = lines
        .iter()
        .position(|l| *l == "(custom gcode: Probe)")
        .unwrap();
    let end = begin
        + lines[begin..]
            .iter()
            .position(|l| *l == "(end custom gcode)")
            .unwrap();
    let block: Vec<&str> = lines.drain(begin..=end).collect();
    let first = lines.iter().position(|l| l.starts_with("G0 ")).unwrap();
    lines.splice(first..first, block);
    let moved = lines.join("\n") + "\n";
    let p = check(&moved, &path, &unread, &profile).unwrap_err();
    assert!(p.what.contains("before tool 3 is in"), "{p}\n{moved}");
}

/// Faults in Klartext's drilling cycles: back only to the set-up clearance
/// between holes, pecks too deep, a hole drilled past its bottom or not to
/// it, no pause at the bottom, a tap with its hand against the spindle's.
#[test]
fn heidenhain_cycle_faults_are_caught() {
    let mut profile = Profile::builtin("heidenhain").unwrap().unwrap();
    profile.drill_cycles = true;
    let faults: &[(ProgramSpec, &str, &str, &str)] = &[
        (
            drilling(Some(2.0), None),
            "Q204=+15",
            "Q204=+2",
            "below the clearance height",
        ),
        (
            drilling(Some(2.0), None),
            "Q202=+2 ",
            "Q202=+5 ",
            "pecks deeper",
        ),
        (chip_breaking(), "Q202=+2 ", "Q202=+4 ", "pecks deeper"),
        (
            drilling(None, None),
            "Q201=-6.5",
            "Q201=-7",
            "past its bottom",
        ),
        (
            drilling(None, None),
            "Q201=-6.5",
            "Q201=-6",
            "before hole 1 is drilled to its bottom",
        ),
        (
            drilling(None, Some(0.5)),
            "Q211=+0.5",
            "Q211=+0",
            "no pause of 0.5 s",
        ),
        (
            tapping(ThreadHand::Right),
            "Q239=+1.25",
            "Q239=-1.25",
            "without the spindle turning M4",
        ),
        (
            tapping(ThreadHand::Right),
            "Q239=+1.25",
            "Q239=+1.4",
            "drills at 700.000",
        ),
        (
            drilling(None, None),
            "L X+30 Y+10 R0 FMAX M99",
            "L X+31 Y+10 R0 FMAX M99",
            "where hole 2 is",
        ),
    ];
    for (spec, from, to, said) in faults {
        let text = post(&Toolpath::new(), spec, &profile);
        assert_eq!(text.matches(from).count(), 1, "{from:?}:\n{text}");
        check(&text, &Toolpath::new(), spec, &profile).unwrap();
        let text = text.replace(from, to);
        match check(&text, &Toolpath::new(), spec, &profile) {
            Ok(()) => panic!("{from:?} made {to:?} was not caught:\n{text}"),
            Err(p) => assert!(p.to_string().contains(said), "{p}\n{text}"),
        }
    }
}

/// Fanuc's and Haas's work offsets past G59, `G54.1 P..` and `G154 P..`,
/// are selected and read back as the setup's; another P is caught.
#[test]
fn extended_work_offsets_read_back() {
    for (name, wcs, word, other) in [
        ("fanuc", 61, "G54.1 P2", "G54.1 P3"),
        ("fanuc-inch", 107, "G54.1 P48", "G54.1 P47"),
        ("haas", 60, "G154 P1", "G154 P2"),
        ("haas-inch", 158, "G154 P99", "G154 P98"),
    ] {
        let profile = Profile::builtin(name).unwrap().unwrap();
        let spec = ProgramSpec {
            wcs,
            extended_offsets: u8::try_from(profile.extended_offsets.len()).unwrap(),
            units: profile.units,
            ..spec()
        };
        let text = post(&fixture(), &spec, &profile);
        assert_eq!(text.matches(&format!("\n{word}\n")).count(), 1, "{text}");
        check(&text, &fixture(), &spec, &profile).unwrap();
        let wrong = text.replace(&format!("\n{word}\n"), &format!("\n{other}\n"));
        let p = check(&wrong, &fixture(), &spec, &profile).unwrap_err();
        assert!(
            p.to_string().contains(&format!("{word} is selected")),
            "{p}"
        );
    }
}
