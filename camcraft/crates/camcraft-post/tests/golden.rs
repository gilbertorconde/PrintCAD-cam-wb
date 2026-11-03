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
use camcraft_post::render::{render, render_dated};
use common::{chip_breaking, drilling, engraving, fixture, slot, spec, tapping};

fn check(profile_name: &str) {
    let profile = Profile::builtin(profile_name)
        .unwrap_or_else(|| panic!("no builtin profile {profile_name}"))
        .unwrap();
    // Written in the profile's units, as a session posts it.
    let spec = ProgramSpec {
        units: profile.units,
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

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{file}.nc"));

    if std::env::var("UPDATE_GOLDEN").is_ok() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(&path, &got).unwrap();
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

fn masso_with_cycles() -> Profile {
    let mut p = Profile::builtin("masso").unwrap().unwrap();
    p.drill_cycles = true;
    p
}

#[test]
fn drilling_cycles_are_stable() {
    let p = masso_with_cycles();
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
    check_against(
        &p,
        &Toolpath::new(),
        &tapping(ThreadHand::Right),
        "masso-tap",
    );
    check_against(
        &p,
        &Toolpath::new(),
        &tapping(ThreadHand::Left),
        "masso-tap-left",
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
    assert!(profile.travel_problems(low, high).is_empty());
    check_dated(
        &profile,
        &fixture(),
        &spec,
        "linuxcnc-custom",
        Some("2026-11-03"),
    );
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
