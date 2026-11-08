//! Golden-file tests for the post processors.
//!
//! One committed instruction stream, rendered through every profile, diffed against
//! committed output. This is what catches dialect drift: a change to the renderer
//! that looks harmless in isolation shows up here as a line that moved.
//!
//! It also proves the IR stays machine-independent. The same fixture feeds every
//! profile; if a profile needed the fixture changed, the abstraction would be
//! leaking.
//!
//! Each program is also read back as a controller would (`readback`) and must
//! cut the path it was posted from, so a golden file cannot be updated to
//! something wrong without a test saying so.
//!
//! Regenerate deliberately, never casually:
//!
//! ```sh
//! UPDATE_GOLDEN=1 cargo test -p camcraft-post --test golden
//! ```
//!
//! Then read the diff. A golden file updated without reading the diff is a test
//! that has been switched off.

#![allow(clippy::unwrap_used, clippy::panic)]

mod common;

use camcraft_core::job::ThreadHand;
use camcraft_core::path::toolpath::Toolpath;
use camcraft_post::op::{ProgramSpec, lower};
use camcraft_post::profile::Profile;
use camcraft_post::readback;
use camcraft_post::render::{Title, render, render_dated, render_program};
use camcraft_post::setup::{Joining, Section, lower_setup};
use common::{
    chip_breaking, dogboned, drilling, engraving, fixture, milled_holes, setup, slot, spec,
    tapping, wrapped, wrapping,
};
use glam::dvec3;

fn check(profile_name: &str) {
    let profile = Profile::builtin(profile_name)
        .unwrap_or_else(|| panic!("no builtin profile {profile_name}"))
        .unwrap();
    // Written in the profile's units, as a session posts it, with the tool
    // the next program takes and the stock, for the profiles that say them.
    let spec = ProgramSpec {
        units: profile.units,
        next_tool: Some(5),
        stock: Some((dvec3(-20.0, -10.0, -12.0), dvec3(20.0, 20.0, 0.0))),
        ..spec()
    };
    check_against(&profile, &fixture(), &spec, profile_name);
}

/// Render `path` for `profile` and compare with `tests/golden/<file>.nc`.
fn check_against(profile: &Profile, path: &Toolpath, spec: &ProgramSpec, file: &str) {
    check_dated(profile, path, spec, file, None);
}

/// [`check_against`], posted on `date`.
fn check_dated(
    profile: &Profile,
    path: &Toolpath,
    spec: &ProgramSpec,
    file: &str,
    date: Option<&str>,
) {
    let profile_name = file;
    let ops = lower(path, spec).unwrap();
    let (got, report) = render_dated(&ops, profile, "golden", date);

    assert!(report.lines > 10, "suspiciously short program");
    // Read back as a controller would, it cuts the path it was posted from.
    if let Err(p) = readback::check(&got, path, spec, profile) {
        panic!("{profile_name}.nc does not read back to its path: {p}\n{got}");
    }
    compare(&got, file);
}

/// `sections` posted as one program for `profile`, compared with
/// `tests/golden/<file>.nc`; it must read back to every section's path.
fn check_setup_against(profile: &Profile, sections: &[Section], file: &str) {
    let (ops, _) = lower_setup(sections, Joining::of(profile)).unwrap();
    let title = Title {
        name: "golden",
        number: 2207,
        date: None,
    };
    let (got, _) = render_program(&ops, profile, &title);
    if let Err(p) = readback::check_setup(&got, sections, profile) {
        panic!("{file}.nc does not read back to its paths: {p}\n{got}");
    }
    compare(&got, file);
}

/// `got` against `tests/golden/<file>.nc`, or written there with
/// `UPDATE_GOLDEN`.
fn compare(got: &str, file: &str) {
    let profile_name = file;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{file}.nc"));

    if std::env::var("UPDATE_GOLDEN").is_ok() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(&path, got).unwrap();
        return;
    }

    let want = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}\nrun with UPDATE_GOLDEN=1 to create it",
            path.display()
        )
    });
    // Compare line by line so a failure names the line rather than dumping two files.
    let want = want.replace("\r\n", "\n");
    for (i, (a, b)) in want.lines().zip(got.lines()).enumerate() {
        assert_eq!(
            a,
            b,
            "{profile_name}.nc line {}: golden {a:?} but rendered {b:?}",
            i + 1
        );
    }
    assert_eq!(
        want.lines().count(),
        got.lines().count(),
        "{profile_name}.nc has a different number of lines"
    );
}

#[test]
fn masso_output_is_stable() {
    check("masso");
}

#[test]
fn masso_inch_output_is_stable() {
    check("masso-inch");
}

#[test]
fn grbl_output_is_stable() {
    check("grbl");
}

#[test]
fn centroid_avid_output_is_stable() {
    check("centroid-avid");
}

#[test]
fn centroid_avid_inch_output_is_stable() {
    check("centroid-avid-inch");
}

#[test]
fn linuxcnc_output_is_stable() {
    check("linuxcnc");
}

#[test]
fn linuxcnc_inch_output_is_stable() {
    check("linuxcnc-inch");
}

#[test]
fn fanuc_output_is_stable() {
    check("fanuc");
}

#[test]
fn fanuc_inch_output_is_stable() {
    check("fanuc-inch");
}

#[test]
fn haas_output_is_stable() {
    check("haas");
}

#[test]
fn haas_inch_output_is_stable() {
    check("haas-inch");
}

#[test]
fn mach_output_is_stable() {
    check("mach");
}

#[test]
fn mach_inch_output_is_stable() {
    check("mach-inch");
}

#[test]
fn heidenhain_output_is_stable() {
    check("heidenhain");
}

#[test]
fn heidenhain_inch_output_is_stable() {
    check("heidenhain-inch");
}

/// Fanuc's canned cycles, as the G-code profiles' are, but a pause in
/// milliseconds.
#[test]
fn fanuc_drilling_cycles_are_stable() {
    let mut p = Profile::builtin("fanuc").unwrap().unwrap();
    p.drill_cycles = true;
    check_against(
        &p,
        &Toolpath::new(),
        &drilling(Some(2.0), None),
        "fanuc-peck",
    );
    check_against(
        &p,
        &Toolpath::new(),
        &drilling(None, Some(0.5)),
        "fanuc-dwell",
    );
}

/// Heidenhain's cycles: each kind of hole a cycle defined once and called at
/// each hole, in millimetres and inches.
#[test]
fn heidenhain_drilling_cycles_are_stable() {
    for name in ["heidenhain", "heidenhain-inch"] {
        let mut p = Profile::builtin(name).unwrap().unwrap();
        p.drill_cycles = true;
        let units = p.units;
        let with = |s: ProgramSpec| ProgramSpec { units, ..s };
        check_against(
            &p,
            &Toolpath::new(),
            &with(drilling(None, Some(0.5))),
            &format!("{name}-dwell"),
        );
        check_against(
            &p,
            &Toolpath::new(),
            &with(drilling(Some(2.0), None)),
            &format!("{name}-peck"),
        );
        check_against(
            &p,
            &Toolpath::new(),
            &with(chip_breaking()),
            &format!("{name}-chip-break"),
        );
        check_against(
            &p,
            &Toolpath::new(),
            &with(tapping(ThreadHand::Left)),
            &format!("{name}-tap-left"),
        );
    }
}

/// An engraving's arcs in Klartext: round the circle by its centre, `CC`
/// and `C`, and down the helix as `CP`, by the angle it turns.
#[test]
fn heidenhain_writes_arcs_and_helices() {
    let p = Profile::builtin("heidenhain").unwrap().unwrap();
    let spec = ProgramSpec {
        strategy: "engrave".into(),
        ..spec()
    };
    check_against(&p, &engraving(), &spec, "heidenhain-engrave");
    let (text, _) = render(&lower(&engraving(), &spec).unwrap(), &p, "golden");
    let blocks = |w: &str| {
        text.lines()
            .filter(|l| l.split_whitespace().nth(1) == Some(w))
            .count()
    };
    assert!(blocks("CC") >= 1 && blocks("C") >= 1, "{text}");
    assert!(blocks("CP") >= 1, "no helix: {text}");
}

/// Masso takes the cycles, its G82's pause in milliseconds; it does not
/// tap.
#[test]
fn drilling_cycles_are_stable() {
    let p = Profile::builtin("masso").unwrap().unwrap();
    check_against(&p, &Toolpath::new(), &drilling(None, None), "masso-drill");
    check_against(
        &p,
        &Toolpath::new(),
        &drilling(Some(2.0), None),
        "masso-peck",
    );
    check_against(
        &p,
        &Toolpath::new(),
        &drilling(None, Some(0.5)),
        "masso-dwell",
    );
    check_against(&p, &Toolpath::new(), &chip_breaking(), "masso-chip-break");
}

/// Each built-in machine that taps, tapping as a session posts it for that
/// machine: LinuxCNC rigidly, `G33.1` a hole at a time, CNC12 and Haas
/// with `G84` and `G74`, Fanuc with them after `M29 S..` (Heidenhain's
/// cycle 207 is in its own drilling goldens); and LinuxCNC's other cycles,
/// now it takes them.
#[test]
fn builtin_tapping_is_stable() {
    for (name, hand, file) in [
        ("linuxcnc", ThreadHand::Right, "linuxcnc-tap"),
        ("linuxcnc-inch", ThreadHand::Left, "linuxcnc-inch-tap-left"),
        ("centroid-avid", ThreadHand::Right, "centroid-avid-tap"),
        (
            "centroid-avid-inch",
            ThreadHand::Left,
            "centroid-avid-inch-tap-left",
        ),
        ("fanuc", ThreadHand::Right, "fanuc-tap"),
        ("fanuc-inch", ThreadHand::Left, "fanuc-inch-tap-left"),
        ("haas", ThreadHand::Right, "haas-tap"),
        ("haas-inch", ThreadHand::Left, "haas-inch-tap-left"),
    ] {
        let p = Profile::builtin(name).unwrap().unwrap();
        let mut spec = ProgramSpec {
            units: p.units,
            ..tapping(hand)
        };
        if let Some(d) = &mut spec.drill {
            d.tap_style = p.tapping();
        }
        check_against(&p, &Toolpath::new(), &spec, file);
    }
    let p = Profile::builtin("linuxcnc").unwrap().unwrap();
    check_against(
        &p,
        &Toolpath::new(),
        &chip_breaking(),
        "linuxcnc-chip-break",
    );
}

/// An engraving's arcs are written as arcs: round the circle `G2` or `G3` by
/// its centre, and down it as a helix, each half turn a line.
#[test]
fn an_engraving_writes_its_arcs_as_arcs() {
    let p = Profile::builtin("grbl").unwrap().unwrap();
    let spec = ProgramSpec {
        strategy: "engrave".into(),
        ..spec()
    };
    check_against(&p, &engraving(), &spec, "grbl-engrave");
    let ops = lower(&engraving(), &spec).unwrap();
    let (text, _) = render(&ops, &p, "golden");
    let arcs: Vec<&str> = text
        .lines()
        .filter(|l| l.split_whitespace().any(|w| w == "G2" || w == "G3"))
        .collect();
    // Climbing round with a clockwise spindle: clockwise, G2, a half turn a
    // line, down the helix then round at the depth.
    assert!(arcs.len() >= 4, "{text}");
    assert!(
        arcs.iter().all(|l| l.contains("G2") && l.contains('I')),
        "{arcs:?}"
    );
    assert!(arcs.iter().any(|l| l.contains('Z')), "no helix: {arcs:?}");
}

/// Holes bored, a boss milled round and a thread milled, on two posts: each
/// helix written as `G2` or `G3` with a `Z`, a half turn a line, round a
/// hole counter-clockwise to climb, round a boss clockwise.
#[test]
fn milled_holes_write_their_helixes_as_arcs() {
    let spec = ProgramSpec {
        strategy: "bore".into(),
        ..spec()
    };
    for (post, file) in [("grbl", "grbl-holes"), ("linuxcnc", "linuxcnc-holes")] {
        let p = Profile::builtin(post).unwrap().unwrap();
        check_against(&p, &milled_holes(), &spec, file);
        let ops = lower(&milled_holes(), &spec).unwrap();
        let (text, _) = render(&ops, &p, "golden");
        let arcs: Vec<&str> = text
            .lines()
            .filter(|l| l.split_whitespace().any(|w| w == "G2" || w == "G3"))
            .collect();
        let helixes = arcs.iter().filter(|l| l.contains('Z')).count();
        assert!(helixes > 10, "{post}: {helixes} helixes\n{text}");
        assert!(arcs.iter().any(|l| l.contains("G2")) && arcs.iter().any(|l| l.contains("G3")));
    }
    let p = Profile::builtin("fanuc").unwrap().unwrap();
    check_against(&p, &milled_holes(), &spec, "fanuc-holes");
    // Klartext: each helix `CP` round its centre by an angle and down or up
    // by `IZ`, its way `DR+` or `DR-`, and a level circle `C` about a `CC`.
    let p = Profile::builtin("heidenhain").unwrap().unwrap();
    check_against(&p, &milled_holes(), &spec, "heidenhain-holes");
    let (text, _) = render(&lower(&milled_holes(), &spec).unwrap(), &p, "golden");
    let helixes: Vec<&str> = text
        .lines()
        .filter(|l| l.split_whitespace().nth(1) == Some("CP"))
        .collect();
    assert!(helixes.len() > 10, "{text}");
    assert!(
        helixes
            .iter()
            .all(|l| l.contains("IPA") && l.contains("IZ"))
    );
    assert!(helixes.iter().any(|l| l.contains("DR+")) && helixes.iter().any(|l| l.contains("DR-")));
}

/// A slot along a line and a half turn, one way in two levels: the half
/// turn is one `G3` by its centre at each level, and each level starts
/// again from the line's start.
#[test]
fn a_slot_writes_its_arcs_as_arcs_at_every_level() {
    let p = Profile::builtin("grbl").unwrap().unwrap();
    let spec = ProgramSpec {
        strategy: "slot".into(),
        ..spec()
    };
    check_against(&p, &slot(), &spec, "grbl-slot");
    let ops = lower(&slot(), &spec).unwrap();
    let (text, _) = render(&ops, &p, "golden");
    let arcs: Vec<&str> = text
        .lines()
        .filter(|l| l.split_whitespace().any(|w| w == "G2" || w == "G3"))
        .collect();
    assert_eq!(arcs.len(), 2, "{text}");
    assert!(
        arcs.iter().all(|l| l.contains("G3") && l.contains('J')),
        "{arcs:?}"
    );
}

/// A mortise's corners relieved: at each level, out from each corner of
/// the ring along the diagonal by the tool's radius times √2 − 1, and back,
/// as straight cuts at the level's height.
#[test]
fn dogbones_are_cut_at_every_level() {
    let p = Profile::builtin("grbl").unwrap().unwrap();
    let spec = ProgramSpec {
        strategy: "profile".into(),
        ..spec()
    };
    check_against(&p, &dogboned(), &spec, "grbl-dogbone");
    let ops = lower(&dogboned(), &spec).unwrap();
    let (text, _) = render(&ops, &p, "golden");
    // 3 (√2 − 1) along each axis, from (10, -6): the relief's end.
    let reach = 10.0 + 3.0 * (2f64.sqrt() - 1.0) / 2f64.sqrt();
    let end = format!("X{reach:.3}");
    assert_eq!(
        text.lines().filter(|l| l.contains(&end)).count(),
        4,
        "{text}"
    );
}

/// A setup tilted 30° about X, for a machine with an A axis: the header says
/// how it is tilted, and the A stands it up at rapid, alone, before it cuts.
#[test]
fn an_a_axis_positions_a_tilted_setup() {
    let mut p = Profile::builtin("masso").unwrap().unwrap();
    p.rotary = Some(toml::from_str("axes = [\"A\"]\n[A]\nlimits = [-120, 120]\n").unwrap());
    let (s, c) = 30f64.to_radians().sin_cos();
    let tool = glam::dvec3(0.0, -s, c);
    let rotary = p
        .rotary
        .as_ref()
        .unwrap()
        .position(tool, glam::DVec3::X)
        .unwrap();
    let spec = ProgramSpec {
        tilt: Some(tool),
        rotary,
        ..spec()
    };
    check_against(&p, &fixture(), &spec, "masso-rotary-a");
}

/// An engraving wrapped round a bar on LinuxCNC's A: X, Z and A at once,
/// fed in inverse time between `G93` and `G94`, each move with its own F;
/// the header says the work zero is on the axis.
#[test]
fn a_wrapped_engraving_turns_a_as_it_cuts() {
    let mut p = Profile::builtin("linuxcnc").unwrap().unwrap();
    let wrap = wrapping(&mut p, "axes = [\"A\"]\n[A]\ncontinuous = true\n");
    let spec = ProgramSpec {
        strategy: "engrave".into(),
        wrap: Some(wrap),
        ..spec()
    };
    check_against(&p, &wrapped(), &spec, "linuxcnc-wrap-a");
}

/// The same for a controller without inverse time: the degrees counted
/// with the millimetres in each move's F.
#[test]
fn a_wrapped_engraving_is_fed_in_degrees_where_asked() {
    let mut p = Profile::builtin("mach").unwrap().unwrap();
    let wrap = wrapping(&mut p, "axes = [\"A\"]\nfeed = \"degrees\"\n");
    let spec = ProgramSpec {
        strategy: "engrave".into(),
        wrap: Some(wrap),
        ..spec()
    };
    check_against(&p, &wrapped(), &spec, "mach-wrap-a-degrees");
}

/// An A and B table stands any tool axis up, both turning, the setup's X
/// lying where they leave the machine's.
#[test]
fn an_a_and_b_table_positions_a_tilted_setup() {
    use camcraft_post::rotary::RotaryError;
    let mut p = Profile::builtin("linuxcnc").unwrap().unwrap();
    p.rotary = Some(
        toml::from_str(
            "axes = [\"A\", \"B\"]\n[A]\nsign = -1\nlimits = [-30, 110]\n\
             [B]\nlimits = [-180, 180]\n",
        )
        .unwrap(),
    );
    let rotary = p.rotary.as_ref().unwrap();
    let tool = glam::dvec3(0.3, -0.4, 0.8).normalize();
    let Err(RotaryError::Twisted { want, .. }) = rotary.position(tool, glam::DVec3::X) else {
        panic!("X turned off the machine's");
    };
    let spec = ProgramSpec {
        tilt: Some(tool),
        rotary: rotary.position(tool, want).unwrap(),
        ..spec()
    };
    check_against(&p, &fixture(), &spec, "linuxcnc-rotary-ab");
}

/// A machine profile of one's own with blocks of its own, every one with
/// placeholders, its own tool change and travel, and Custom G-code before
/// the operation and after it: what it writes, and that it reads back.
#[test]
fn custom_blocks_are_stable() {
    use camcraft_post::op::CustomGcode;
    let linuxcnc = include_str!("../profiles/linuxcnc.toml");
    let (_, tables) = linuxcnc.split_at(linuxcnc.find("[format]").unwrap());
    let text = format!(
        r#"
name = "My mill"
extension = "ngc"
comment = ["(", ")"]
arc_style = "ij"
tool_change = "m6"
spindle = [6000, 24000]
header = [
  "%",
  "(@name@: @operation@, @strategy@)",
  "(posted @date@ in @units@ on @wcs@)",
  "G90 G94 G17 G40 G49 G80",
]
tool_change_lines = [
  "(change to tool @tool@: @tool_comment@)",
  "T@tool@ M6",
  "G43 H@tool@",
  "M0 (check tool @tool@ at @rpm@ rev/min, @coolant@)",
]
safe_retract = ["G53 G0 Z0"]
end_home = ["G53 G0 X0 Y0 (park after @operation@)"]
footer = ["(end of @name@, @@ the shop)", "M30", "%"]

[travel]
x = [-50, 300]
y = [-50, 200]
z = [-80, 60]

{tables}"#
    );
    let profile = Profile::from_toml(&text).unwrap();
    let spec = ProgramSpec {
        before: vec![CustomGcode::new(
            "Check clamps",
            "(MSG, check the clamps before @operation@)\nM0\nM7\nG4 P2",
        )],
        after: vec![CustomGcode::new("Signal", "M64 P0 (done with T@tool@)\nM1")],
        ..spec()
    };
    let (low, high) = camcraft_post::op::extents(&fixture()).unwrap();
    let at = camcraft_post::profile::Placement::default();
    assert!(profile.travel_check(low, high, &at).past.is_empty());
    check_dated(
        &profile,
        &fixture(),
        &spec,
        "linuxcnc-custom",
        Some("2026-11-03"),
    );
}

/// LinuxCNC on its eighth work offset, `G59.2`, which it selects and reads
/// back, with its travel in machine coordinates: the program kept inside it
/// where its work offset puts it, and past it when moved.
#[test]
fn an_extended_work_offset_is_stable() {
    use camcraft_post::profile::Placement;
    use glam::dvec3;
    let linuxcnc = include_str!("../profiles/linuxcnc.toml");
    let text = format!(
        "{linuxcnc}\n[travel]\ncoordinates = \"machine\"\nx = [0, 300]\ny = [-200, 0]\nz = [-150, 0]\n"
    );
    let profile = Profile::from_toml(&text).unwrap();
    let spec = ProgramSpec {
        wcs: 61,
        extended_offsets: 3,
        ..spec()
    };
    let (low, high) = camcraft_post::op::extents(&fixture()).unwrap();
    // The tip 60 mm below the gauge line; G59.2 at X100 Y-150 Z-120.
    let mut at = Placement {
        zero: Some(dvec3(100.0, -150.0, -120.0)),
        tool_length: Some(60.0),
    };
    let check = profile.travel_check(low, high, &at);
    assert!(
        check.past.is_empty() && check.unchecked.is_empty(),
        "{check:?}"
    );
    // 40 mm higher, the gauge line goes past the top of Z.
    at.zero = Some(dvec3(100.0, -150.0, -80.0));
    let past = profile.travel_check(low, high, &at).past;
    assert!(
        past.len() == 1
            && past[0].starts_with("Z goes from ")
            && past[0].contains(" over LinuxCNC's travel of -150 to 0 mm"),
        "{past:?}"
    );
    check_against(&profile, &fixture(), &spec, "linuxcnc-g59-2");
}

/// A setup of three operations on two tools as one program: the header and
/// footer once, the tool changed once, the second tool kept turning into
/// the engraving at its own speed with the coolant off, on a machine that
/// changes tools itself and one where the operator does.
#[test]
fn a_setup_is_one_program() {
    let ops = setup();
    let sections: Vec<Section> = ops
        .iter()
        .map(|(path, spec)| Section { path, spec })
        .collect();
    for name in ["linuxcnc", "grbl"] {
        let p = Profile::builtin(name).unwrap().unwrap();
        check_setup_against(&p, &sections, &format!("{name}-setup"));
    }
}

/// The setup as one program for a Fanuc, numbered as the setup is, the
/// second tool called up after the first change, and for a Heidenhain in
/// Klartext: one BEGIN and END PGM, the stock once, a TOOL CALL a change
/// and the engraving's speed by a TOOL CALL of its own.
#[test]
fn a_setup_is_one_program_on_industrial_controls() {
    let mut ops = setup();
    for (_, spec) in &mut ops {
        spec.stock = Some((dvec3(-5.0, -5.0, -20.0), dvec3(60.0, 50.0, 0.0)));
    }
    let sections: Vec<Section> = ops
        .iter()
        .map(|(path, spec)| Section { path, spec })
        .collect();
    for name in ["fanuc", "heidenhain", "haas"] {
        let p = Profile::builtin(name).unwrap().unwrap();
        check_setup_against(&p, &sections, &format!("{name}-setup"));
    }
}

/// The setup with two tapping operations after it, the tap kept turning
/// from one to the other: on a Fanuc each takes rigid mode (`M29 S..`)
/// again, since it ends with its cycles at `G80`; on LinuxCNC each hole is
/// tapped by `G33.1`.
#[test]
fn a_setup_taps_in_each_posts_form() {
    use camcraft_post::profile::TapStyle;
    for (name, style) in [("fanuc", TapStyle::M29), ("linuxcnc", TapStyle::G331)] {
        let mut ops = setup();
        for op_name in ["threads", "more threads"] {
            let mut spec = common::tapping(ThreadHand::Right);
            spec.op_name = op_name.into();
            if let Some(d) = &mut spec.drill {
                d.tap_style = style;
            }
            ops.push((Toolpath::new(), spec));
        }
        let sections: Vec<Section> = ops
            .iter()
            .map(|(path, spec)| Section { path, spec })
            .collect();
        let p = Profile::builtin(name).unwrap().unwrap();
        check_setup_against(&p, &sections, &format!("{name}-setup-taps"));
    }
}

/// The same with optional stops between operations, which restart the same
/// tool, and custom G-code before the engraving and at the end.
#[test]
fn a_setup_stops_between_operations_where_asked() {
    use camcraft_post::op::CustomGcode;
    let mut ops = setup();
    ops[2].1.before = vec![CustomGcode::new(
        "Check depth",
        "(MSG, measure the pocket before @operation@)\nM0",
    )];
    ops[2].1.after = vec![CustomGcode::new("Air off", "M9")];
    let sections: Vec<Section> = ops
        .iter()
        .map(|(path, spec)| Section { path, spec })
        .collect();
    let mut p = Profile::builtin("masso").unwrap().unwrap();
    p.optional_stop = true;
    check_setup_against(&p, &sections, "masso-setup-stop");
}

/// A probe grid on LinuxCNC, its points logged to a file by `PROBEOPEN`, and
/// on GRBL, which reports them over the serial line: `G38.2` down from over
/// each point to 3 mm below where it should touch, at the probe's feed,
/// back up at rapid, the spindle never started. Then on LinuxCNC as the
/// first operation of a setup, the engraving after it, whose spindle starts
/// once the probe is put away.
#[test]
fn a_probe_grid_is_stable() {
    let (path, probe) = common::probing(Some("probe.txt"));
    let linuxcnc = Profile::builtin("linuxcnc").unwrap().unwrap();
    check_against(&linuxcnc, &path, &probe, "linuxcnc-probe");
    let (path, probe) = common::probing(None);
    let grbl = Profile::builtin("grbl").unwrap().unwrap();
    check_against(&grbl, &path, &probe, "grbl-probe");

    let (path, probe) = common::probing(Some("probe.txt"));
    let engraving = (
        engraving(),
        ProgramSpec {
            op_name: "engrave".into(),
            strategy: "engrave".into(),
            rpm: 12000,
            coolant: camcraft_core::job::Coolant::Off,
            ..spec()
        },
    );
    let sections = [
        Section {
            path: &path,
            spec: &probe,
        },
        Section {
            path: &engraving.0,
            spec: &engraving.1,
        },
    ];
    check_setup_against(&linuxcnc, &sections, "linuxcnc-setup-probe");
}

/// Every builtin profile must have a golden file. Adding a profile without one
/// would leave it untested by exactly the test meant to cover it.
#[test]
fn every_builtin_profile_has_a_golden_file() {
    for name in Profile::builtin_names() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(format!("{name}.nc"));
        assert!(
            path.exists() || std::env::var("UPDATE_GOLDEN").is_ok(),
            "profile {name} has no golden file at {}",
            path.display()
        );
    }
}

/// The same instruction stream drives every profile. If one needed a different
/// fixture, the IR would not be machine-independent.
#[test]
fn one_fixture_serves_every_profile() {
    let ops = lower(&fixture(), &spec()).unwrap();
    let mut seen = Vec::new();
    for name in Profile::builtin_names() {
        let p = Profile::builtin(name).unwrap().unwrap();
        let (text, _) = render(&ops, &p, "golden");
        assert!(!text.is_empty());
        seen.push(text);
    }
    let first = seen.first().unwrap();
    assert!(
        seen.iter().any(|t| t != first),
        "all profiles produced identical output, so none of them is doing anything"
    );
}
