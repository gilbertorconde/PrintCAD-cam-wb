use super::*;
use crate::test_support::run_small;
use crate::{load_models, parse_job, run_setup};
use camcraft_core::job::{Coolant, SpindleDirection};
use camcraft_post::setup::Section;

#[test]
fn program_names_are_numbered_and_file_safe() {
    assert_eq!(program_name("top", 0, "rough"), "top-01-rough");
    assert_eq!(
        program_name("side 2", 11, "finish / walls"),
        "side-2-12-finish---walls"
    );
}

/// The example job on the small fixture, run.
fn example() -> (camcraft_core::job::Job, PathBuf, Ran) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let text = std::fs::read_to_string(dir.join("rail-jig.camcraft")).unwrap();
    // The small fixture, so the test runs quickly.
    let text = text.replace("sled_base.step", "hinge_jig.step");
    let job = parse_job(&text, "rail-jig").unwrap();
    let loaded = load_models(&job, &dir, &[0]).unwrap();
    let ran = run_setup(&job, 0, &crate::models_of(&loaded)).unwrap();
    (job, dir, ran)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("camcraft-posting-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The same names and programs as the command line writes, into the folder.
#[test]
fn a_posting_writes_a_program_per_operation() {
    let (job, job_dir, ran) = example();
    let dir = scratch("writes");
    let p = plan_posting(
        &[(&job.setups[0], &ran)],
        &job_dir,
        &dir,
        None,
        Some(Programs::Operation),
    )
    .unwrap();
    assert!(!p.needs_confirming(), "{:?}", p.refused);
    let names: Vec<String> = p
        .programs
        .iter()
        .map(|g| g.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["top-01-rough.nc", "top-02-finish.nc"]);
    assert_eq!(write_posting(&p).unwrap(), 2);
    let text = std::fs::read_to_string(&p.programs[0].path).unwrap();
    assert!(text.contains("G54"));

    // Posting again would replace them, and says so first.
    let again = plan_posting(
        &[(&job.setups[0], &ran)],
        &job_dir,
        &dir,
        None,
        Some(Programs::Operation),
    )
    .unwrap();
    assert_eq!(again.overwrites.len(), 2);
    assert!(again.needs_confirming());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A setup posted as one program, as it is unless it says otherwise: one
/// file named after it, every operation in it in order, numbered as the
/// setup says; one an operation numbered on from it.
#[test]
fn a_setup_is_posted_as_one_program() {
    let (mut job, job_dir, ran) = example();
    let out = scratch("one");
    std::fs::create_dir_all(&out).unwrap();
    let base = include_str!("../../../camcraft-post/profiles/grbl.toml");
    let numbered = base.replacen("\"({name})\",", "\"(O@number@ {name})\",", 1);
    assert_ne!(numbered, base);
    let file = out.join("numbered.toml");
    std::fs::write(&file, numbered).unwrap();
    let profile = Some(file.to_str().unwrap());
    let dir = out.join("programs");
    job.setups[0].number = 2000;
    let p = plan_posting(&[(&job.setups[0], &ran)], &job_dir, &dir, profile, None).unwrap();
    assert_eq!(p.programs.len(), 1);
    let one = &p.programs[0];
    assert_eq!(one.path, dir.join("top.nc"));
    assert_eq!(one.op, "top");
    assert!(one.text.starts_with("(O2000 top)\n"), "{}", one.text);
    let rough = one.text.find("(rough - ").unwrap();
    let finish = one.text.find("(finish - ").unwrap();
    assert!(rough < finish);
    assert_eq!(one.text.matches("M30").count(), 1);

    let each = plan_posting(
        &[(&job.setups[0], &ran)],
        &job_dir,
        &dir,
        profile,
        Some(Programs::Operation),
    )
    .unwrap();
    let firsts: Vec<&str> = each
        .programs
        .iter()
        .map(|g| g.text.lines().next().unwrap())
        .collect();
    assert_eq!(firsts, ["(O2000 top-01-rough)", "(O2001 top-02-finish)"]);
    let _ = std::fs::remove_dir_all(&out);
}

/// Each line of a setup's program is of an operation and a move of it, in
/// order: the header of the first, and each operation's own moves counted
/// as the operation's.
#[test]
fn a_setup_program_says_whose_each_instruction_is() {
    let (job, _, ran) = example();
    let profile = load_profile("masso", None).unwrap();
    let setup = &job.setups[0];
    let title = Title {
        name: "top",
        number: 1,
        date: None,
    };
    let p = setup_program(setup, &ran.result, &profile, &title).unwrap();
    assert_eq!(p.made.len(), p.report.op_lines.len());
    assert!(p.made.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(p.made.first(), Some(&(0, 0)));
    let last = ran.result.operations.len() - 1;
    assert_eq!(
        p.made.last(),
        Some(&(last, ran.result.operations[last].path.len()))
    );
    for (k, op) in ran.result.operations.iter().enumerate() {
        assert!(p.made.contains(&(k, op.path.len())), "{}", op.name);
    }
    // Each change it counts is one it writes.
    let written = p.text.lines().filter(|l| l.contains(" M6")).count();
    assert!(p.tool_changes >= 1);
    assert_eq!(p.tool_changes, written, "{}", p.text);
}

/// Two setups whose names come to the same file name, as "flip 1" and
/// "Flip_1" do, are refused before either is written, rather than the second
/// silently replacing the first.
#[test]
fn two_programs_for_one_file_are_refused() {
    let (job, job_dir, ran) = example();
    let mut other = job.setups[0].clone();
    other.name = "TOP".into();
    let dir = scratch("same");
    let err = plan_posting(
        &[(&job.setups[0], &ran), (&other, &ran)],
        &job_dir,
        &dir,
        None,
        None,
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("setup 'top'") && err.contains("setup 'TOP'"),
        "{err}"
    );
}

/// A setup that did not pass is named, with what failed, before anything is
/// written.
#[test]
fn a_refused_setup_is_named_with_its_failures() {
    let (job, job_dir, mut ran) = example();
    ran.result.operations[0].sim.rapid_contacts = 1;
    ran.checks.groups[1].findings[0].level = crate::checks::Level::Fail;
    let dir = scratch("refused");
    let p = plan_posting(
        &[(&job.setups[0], &ran)],
        &job_dir,
        &dir,
        None,
        Some(Programs::Operation),
    )
    .unwrap();
    assert_eq!(p.refused.len(), 1);
    assert_eq!(p.refused[0].0, "top");
    assert!(!p.refused[0].1.is_empty());
    assert!(!dir.exists(), "planning writes nothing");
}

/// A profile that cannot be loaded stops the posting before anything is
/// rendered or written, and says which setup.
#[test]
fn a_bad_profile_writes_nothing() {
    let (job, job_dir, ran) = example();
    let dir = scratch("bad-profile");
    let err = plan_posting(
        &[(&job.setups[0], &ran)],
        &job_dir,
        &dir,
        Some("no-such"),
        None,
    )
    .unwrap_err();
    assert!(err.to_string().contains("setup 'top'"), "{err}");
    assert!(!dir.exists());
}

#[test]
fn a_program_renders_from_a_run() {
    let ran = run_small();
    let profile = load_profile("masso", None).unwrap();
    let op = ran.result.operations.first().unwrap();
    let (text, report) = program(op, 54, &ran.result.frame, &profile, "01-finish").unwrap();
    assert!(text.contains("G54"));
    assert!(report.motion_blocks > 10);
    let word = |w: &str| text.split_whitespace().any(|x| x == w);
    assert!(word("M3") && !word("M8"), "{text}");
}

/// A drilling operation is posted as canned cycles only for a profile that
/// takes them; otherwise as its moves.
#[test]
fn holes_are_cycles_only_where_the_profile_takes_them() {
    use camcraft_core::strategy::drilling::{DrillReport, Hole};
    let ran = run_small();
    let mut op = ran.result.operations.first().unwrap().clone();
    op.detail = StrategyDetail::Drill(DrillReport {
        holes: vec![Hole {
            at: glam::dvec2(3.0, 4.0),
            top: 5.0,
            bottom: 0.0,
            radius: 2.0,
        }],
        peck: Some(1.5),
        chip_break: None,
        dwell: None,
        tap: None,
        feed: 200.0,
    });
    let mut profile = load_profile("masso", None).unwrap();
    profile.drill_cycles = false;
    let (moves, _) = program(&op, 54, &ran.result.frame, &profile, "01-holes").unwrap();
    assert!(!moves.contains("G83"), "{moves}");
    profile.drill_cycles = true;
    let (cycles, _) = program(&op, 54, &ran.result.frame, &profile, "01-holes").unwrap();
    assert!(cycles.contains("G83 X3. Y4. Z0. R7."), "{cycles}");
    assert!(cycles.contains("G80"), "{cycles}");
}

/// A tapping operation is posted as the machine taps: its cycle for one
/// that takes cycles, rigid tapping for LinuxCNC; and refused for one that
/// cannot tap, saying which machines can.
#[test]
fn a_tap_is_posted_only_as_the_machine_taps() {
    use camcraft_core::job::ThreadHand;
    use camcraft_core::strategy::drilling::{DrillReport, Hole, Tap};
    let ran = run_small();
    let mut op = ran.result.operations.first().unwrap().clone();
    let tap = Tap {
        pitch: 1.5,
        hand: ThreadHand::Left,
    };
    op.rpm = 400;
    op.spindle = SpindleDirection::CounterClockwise;
    op.detail = StrategyDetail::Drill(DrillReport {
        holes: vec![Hole {
            at: glam::dvec2(3.0, 4.0),
            top: 5.0,
            bottom: 0.0,
            radius: 5.0,
        }],
        peck: None,
        chip_break: None,
        dwell: None,
        tap: Some(tap),
        feed: tap.feed(400.0),
    });
    let mut profile = load_profile("masso", None).unwrap();
    let refused = program(&op, 54, &ran.result.frame, &profile, "01-threads").unwrap_err();
    let said = refused.to_string();
    assert!(said.starts_with("Masso cannot tap"), "{said}");
    assert!(
        said.contains(
            "(Centroid CNC12 (Avid CNC), LinuxCNC, Fanuc, Haas (NGC), Heidenhain TNC (Klartext))"
        ),
        "{said}"
    );
    profile.drill_cycles = true;
    profile.tap = None;
    let (cycles, _) = program(&op, 54, &ran.result.frame, &profile, "01-threads").unwrap();
    assert!(cycles.contains("S400 M4"), "{cycles}");
    assert!(cycles.contains("G74 X3. Y4. Z0. R7. F600."), "{cycles}");

    let linuxcnc = load_profile("linuxcnc", None).unwrap();
    let (rigid, _) = program(&op, 54, &ran.result.frame, &linuxcnc, "01-threads").unwrap();
    assert!(rigid.contains("S400 M4"), "{rigid}");
    assert!(rigid.contains("\nZ7\nG33.1 Z0 K1.5\n"), "{rigid}");
    assert!(!rigid.contains("G74"), "{rigid}");
    let avid = load_profile("centroid-avid", None).unwrap();
    let (cycle, _) = program(&op, 54, &ran.result.frame, &avid, "01-threads").unwrap();
    assert!(cycle.contains("G74 X3 Y4 Z0 R7 F600"), "{cycle}");
    let fanuc = load_profile("fanuc", None).unwrap();
    let (m29, _) = program(&op, 54, &ran.result.frame, &fanuc, "01-threads").unwrap();
    assert!(
        m29.contains("\nM29 S400\nG74 X3. Y4. Z0. R7. F600.\n"),
        "{m29}"
    );
}

/// A tool's spindle direction and coolant reach the program.
#[test]
fn a_tools_direction_and_coolant_are_posted() {
    let ran = run_small();
    let profile = load_profile("masso", None).unwrap();
    let mut op = ran.result.operations.first().unwrap().clone();
    op.spindle = SpindleDirection::CounterClockwise;
    op.coolant = Coolant::Mist;
    let (text, _) = program(&op, 54, &ran.result.frame, &profile, "01-finish").unwrap();
    let word = |w: &str| text.split_whitespace().any(|x| x == w);
    assert!(word("M4") && !word("M3"), "{text}");
    assert!(word("M7") && word("M9"), "{text}");
}

/// A tilted setup's program says so in its header; with an A rotary it
/// stands the part up first, at rapid, alone, and reads back; past the A's
/// limits, it is refused, saying why.
#[test]
fn a_tilted_setup_is_posted_with_its_rotary() {
    use camcraft_core::setup::frame::{Axis, Direction, Frame, Orientation};
    let ran = run_small();
    let op = ran.result.operations.first().unwrap();
    let (s, c) = 30f64.to_radians().sin_cos();
    let turn = Orientation {
        z: Direction::Vector([0.0, -s, c]),
        x: Direction::Axis(Axis::PosX),
    }
    .turn()
    .unwrap()
    .unwrap();
    let frame = Frame {
        turn,
        origin: glam::DVec3::ZERO,
    };
    let mut profile = load_profile("linuxcnc", None).unwrap();
    let (by_hand, _) = program(op, 54, &frame, &profile, "01-tilted").unwrap();
    assert!(by_hand.contains("tilted 30 deg"), "{by_hand}");
    assert!(by_hand.contains("fixture the part tilted"), "{by_hand}");
    assert!(!by_hand.contains(" A"), "{by_hand}");

    profile.rotary =
        Some(toml::from_str("axes = [\"A\"]\n[A]\nsign = -1\nlimits = [-10, 110]\n").unwrap());
    let (text, _) = program(op, 54, &frame, &profile, "01-tilted").unwrap();
    assert!(text.lines().any(|l| l == "G0 A30"), "{text}");
    assert!(text.contains("set the work offset in it"), "{text}");
    let spec = program_spec(op, 54, &frame, &profile).unwrap();
    if let Err(p) = camcraft_post::readback::check(&text, &op.path, &spec, &profile) {
        panic!("does not read back: {p}\n{text}");
    }

    profile.rotary = Some(toml::from_str("axes = [\"A\"]\n[A]\nlimits = [-20, 20]\n").unwrap());
    let e = program(op, 54, &frame, &profile, "01-tilted").unwrap_err();
    assert!(e.to_string().contains("past the machine's limits"), "{e}");
}

/// Every operation of setup `index` of `job`, as run, posted for every
/// built-in machine and read back as a controller would, cuts the path it was
/// posted from. Says how many moves were read back, so a run with nothing in
/// it does not pass for one that checked something.
fn reads_back_for_every_post(job: &camcraft_core::job::Job, index: usize, ran: &Ran) -> usize {
    let setup = &job.setups[index];
    let mut moves = 0;
    for name in builtin_posts() {
        let profile = load_profile(name, None).unwrap();
        for (i, op) in ran.result.operations.iter().enumerate() {
            let (text, _) = program(
                op,
                setup.wcs,
                &ran.result.frame,
                &profile,
                &program_name(&setup.name, i, &op.name),
            )
            .unwrap();
            let spec = program_spec(op, setup.wcs, &ran.result.frame, &profile).unwrap();
            if let Err(p) = camcraft_post::readback::check(&text, &op.path, &spec, &profile) {
                panic!("{} posted for {name} does not read back: {p}", op.name);
            }
            moves += op.path.len();
        }
        // And all of them as one program.
        let title = Title {
            name: &setup.name,
            number: setup.number,
            date: None,
        };
        let joined = setup_program(setup, &ran.result, &profile, &title).unwrap();
        let specs = setup_specs(setup, &ran.result, &profile).unwrap();
        let sections: Vec<Section> = specs
            .iter()
            .map(|(k, spec)| Section {
                path: &ran.result.operations[*k].path,
                spec,
            })
            .collect();
        if let Err(p) = camcraft_post::readback::check_setup(&joined.text, &sections, &profile) {
            panic!(
                "{} posted as one for {name} does not read back: {p}",
                setup.name
            );
        }
    }
    moves
}

/// The example job's pocket clearing and parallel finish.
#[test]
fn every_program_reads_back_to_its_path() {
    let (job, _, ran) = example();
    assert!(reads_back_for_every_post(&job, 0, &ran) > 1000);
}

/// The lego brick's contour, as the command line's comparison with Fusion
/// runs it: ramped down round the ring at three depths, with Fusion's leads,
/// upright arcs among them.
#[test]
fn the_lego_contour_reads_back_for_every_post() {
    let text = format!(
        r#"
version = 1
tolerance = 0.05

[tools.quarter]
kind = "flat"
diameter = 6.35
number = 2
rpm = 16000

[[model]]
name = "lego"
path = "{}"

[[stock]]
name = "block"
kind = "around-models"
margin = [50.8, 12.7, 5.08]

[[setup]]
name = "top"
models = ["lego"]
stocks = ["block"]

[setup.origin]
kind = "box-point"
of = "stock"
x = "min"
y = "min"
z = "min"

[[setup.operation]]
name = "contour"
strategy = "profile"
tool = "quarter"
heights = {{ bottom = {{ ref = "model-bottom", offset = 1.0 }} }}
passes = {{ feed = 3810.0, stepdown = 15.0 }}
linking = {{ entry = "ramp" }}
"#,
        crate::test_support::fixture("lego_4x2_01.step")
            .display()
            .to_string()
            .replace('\\', "/")
    );
    let job = parse_job(&text, "lego").unwrap();
    let loaded = load_models(&job, Path::new("."), &[0]).unwrap();
    let ran = run_setup(&job, 0, &crate::models_of(&loaded)).unwrap();
    assert!(reads_back_for_every_post(&job, 0, &ran) > 50);
}

/// A job of your own, each setup run and read back for every built-in post:
/// `CAMCRAFT_READBACK_JOB=path/to/job.camcraft cargo test -p camcraft-session
/// readback_job`. Nothing is checked when it is not set.
#[test]
fn readback_job_from_the_environment() {
    let Ok(path) = std::env::var("CAMCRAFT_READBACK_JOB") else {
        return;
    };
    let path = PathBuf::from(path);
    let job = crate::read_job(&path).unwrap();
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut moves = 0;
    for index in 0..job.setups.len() {
        let loaded = load_models(&job, dir, &[index]).unwrap();
        let ran = run_setup(&job, index, &crate::models_of(&loaded)).unwrap();
        moves += reads_back_for_every_post(&job, index, &ran);
    }
    println!("{moves} moves read back from {}", path.display());
    assert!(moves > 0, "nothing in {} to read back", path.display());
}

/// Custom G-code between the example's two operations, and after the last:
/// each written into the program beside it, which reads back; no program of
/// its own; and a profile whose travel the programs go past refuses them,
/// saying which and why.
#[test]
fn custom_gcode_is_posted_beside_its_neighbours_and_travel_is_kept() {
    use camcraft_core::job::{Operation, StrategySpec};
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let text = std::fs::read_to_string(dir.join("rail-jig.camcraft")).unwrap();
    let text = text.replace("sled_base.step", "hinge_jig.step");
    let mut job = parse_job(&text, "rail-jig").unwrap();
    let custom = |name: &str, gcode: &str| Operation {
        gcode: gcode.into(),
        ..Operation::new(name, StrategySpec::CustomGcode {})
    };
    let ops = &mut job.setups[0].operations;
    ops.insert(1, custom("flip", "M0 (turn T@tool@ round)"));
    ops.push(custom("signal", "M64 P1"));
    // As a job file writes it and reads it back.
    let job = parse_job(&crate::job_text(&job).unwrap(), "again").unwrap();
    let loaded = load_models(&job, &dir, &[0]).unwrap();
    let ran = run_setup(&job, 0, &crate::models_of(&loaded)).unwrap();
    assert!(ran.passed(), "{:?}", ran.checks.failure_lines());
    let made: Vec<Made> = ran.result.operations.iter().map(|o| o.made).collect();
    assert_eq!(
        made,
        [Made::Generated, Made::Text, Made::Generated, Made::Text]
    );
    let blocks = custom_gcode(&job.setups[0], &ran.result.operations);
    assert!(blocks[0].0.is_empty() && blocks[0].1.is_empty());
    assert_eq!(
        blocks[2].0,
        [CustomGcode::new("flip", "M0 (turn T@tool@ round)")]
    );
    assert_eq!(blocks[2].1, [CustomGcode::new("signal", "M64 P1")]);

    let out = scratch("custom");
    let p = plan_posting(
        &[(&job.setups[0], &ran)],
        &dir,
        &out,
        None,
        Some(Programs::Operation),
    )
    .unwrap();
    assert!(p.refused.is_empty(), "{:?}", p.refused);
    assert_eq!(p.programs.len(), 2, "none of its own");
    let finish = &p.programs[1].text;
    // And as one program, the custom G-code between the two operations.
    let one = plan_posting(&[(&job.setups[0], &ran)], &dir, &out, None, None).unwrap();
    let text = &one.programs[0].text;
    let at = |w: &str| text.find(w).unwrap_or_else(|| panic!("{w} in {text}"));
    assert!(at("(rough - ") < at("(custom gcode: flip)"));
    assert!(at("(custom gcode: flip)") < at("(finish - "));
    assert!(at("(finish - ") < at("(custom gcode: signal)"));
    // Masso numbers its blocks, the custom ones among them.
    assert!(
        finish.contains("(custom gcode: flip)\nN20 M0 (turn T2 round)\n"),
        "{finish}"
    );
    assert!(
        finish.contains("(custom gcode: signal)\n") && finish.contains(" M64 P1\n"),
        "{finish}"
    );
    assert!(!p.programs[0].text.contains("custom gcode"));
    let setup = &job.setups[0];
    let profile = load_profile(&setup.post, None).unwrap();
    for (k, op) in ran.result.operations.iter().enumerate() {
        if !op.made.has_paths() {
            continue;
        }
        let mut spec = program_spec(op, setup.wcs, &ran.result.frame, &profile).unwrap();
        (spec.before, spec.after) = blocks[k].clone();
        let name = program_name(&setup.name, k, &op.name);
        let (text, _) = program_with(op, &spec, &profile, &name).unwrap();
        if let Err(e) = camcraft_post::readback::check(&text, &op.path, &spec, &profile) {
            panic!("{} does not read back: {e}\n{text}", op.name);
        }
    }

    // A profile whose Z travel stops short of the depth the part is cut to.
    let base = include_str!("../../../camcraft-post/profiles/grbl.toml");
    let tight = base.replacen("\"GRBL\"", "\"Short Z\"", 1).replacen(
        "[format]",
        "[travel]\nz = [-1, 100]\n\n[format]",
        1,
    );
    std::fs::create_dir_all(&out).unwrap();
    let file = out.join("short.toml");
    std::fs::write(&file, tight).unwrap();
    let p = plan_posting(
        &[(&job.setups[0], &ran)],
        &dir,
        &out.join("programs"),
        Some(file.to_str().unwrap()),
        None,
    )
    .unwrap();
    let (_, why) = &p.refused[0];
    assert!(
        why.iter()
            .any(|w| w.starts_with("travel rough: Z goes from -")),
        "{why:?}"
    );

    // Its Z travel in machine coordinates, homed at the top: where the
    // setup's work offset is decides. Not given, nothing is refused.
    let machine = base.replacen("\"GRBL\"", "\"Homed\"", 1).replacen(
        "[format]",
        "[travel]\ncoordinates = \"machine\"\nz = [-100, 0]\n\n[format]",
        1,
    );
    let file = out.join("homed.toml");
    std::fs::write(&file, machine).unwrap();
    let post = |setup: &camcraft_core::job::Setup| {
        plan_posting(
            &[(setup, &ran)],
            &dir,
            &out.join("programs"),
            Some(file.to_str().unwrap()),
            None,
        )
        .unwrap()
    };
    assert!(post(&job.setups[0]).refused.is_empty());
    let mut low = job.setups[0].clone();
    low.work_offset = Some([0.0, 0.0, -50.0]);
    assert!(post(&low).refused.is_empty());
    // Its work zero 5 mm under the top of Z: the clearance goes past it.
    let mut high = job.setups[0].clone();
    high.work_offset = Some([0.0, 0.0, -5.0]);
    let p = post(&high);
    let (_, why) = &p.refused[0];
    assert!(
        why.iter()
            .any(|w| w.starts_with("travel rough: Z goes from -")
                && w.contains("in machine coordinates")
                && w.contains("over Homed's travel of -100 to 0 mm")),
        "{why:?}"
    );
    let _ = std::fs::remove_dir_all(&out);
}
