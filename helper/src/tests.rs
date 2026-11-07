use std::collections::BTreeMap;

use cam_protocol::{IDENTITY, Level, Request, Shape, VERSION};
use ogeom_io::native::{WriteOptions, write};

use super::answer;

/// A fixture's solids as printCAD keeps a body's shape: ogeom's native text,
/// without triangulations.
fn native(fixture: &str) -> String {
    let path = format!(
        "{}/../camcraft/fixtures/step/{fixture}",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(path).unwrap();
    let import = ogeom_io::read_step(&text, ogeom_core_tolerances()).unwrap();
    let model = import.document.model();
    let roots: Vec<_> = import.solids.clone();
    write(
        model,
        &roots,
        WriteOptions {
            triangulations: false,
        },
    )
    .unwrap()
}

fn ogeom_core_tolerances() -> ogeom_core::Tolerances {
    ogeom_core::Tolerances::millimetres()
}

const JOB: &str = r#"
version = 1
tolerance = 0.05

[tools.flat-6]
kind = "flat"
diameter = 6.0
number = 1
rpm = 16000

[[model]]
name = "part"
path = "part"

[[stock]]
name = "block"
kind = "around-models"
margin = [2.0, 2.0, 2.0]

[[setup]]
name = "top"
models = ["part"]
stocks = ["block"]
post = "grbl"
programs = "operation"

[[setup.operation]]
name = "rough"
strategy = "pocket-clearing"
tool = "flat-6"
passes = { stepdown = 3.0, stepover = 3.0 }
"#;

#[test]
fn a_native_solid_runs_and_posts() {
    let request = Request {
        version: VERSION,
        // Kept round the part's silhouette, so it has a boundary to draw.
        job: JOB.replace(
            "passes = { stepdown = 3.0, stepover = 3.0 }",
            "passes = { stepdown = 3.0, stepover = 3.0 }\n\
             boundary = { kind = \"silhouette\", containment = \"outside\", margin = 6.0 }",
        ),
        shapes: BTreeMap::from([(
            "part".to_string(),
            Shape {
                text: native("hinge_jig.step"),
                placement: IDENTITY,
            },
        )]),
        max_points: 5000,
        ..Request::default()
    };
    let mut steps = Vec::new();
    let run =
        crate::answer_reporting(&request, &mut |done, total| steps.push((done, total))).unwrap();
    // Every step told, in order, from none to all.
    let total = steps[0].1;
    assert_eq!(steps.first(), Some(&(0, total)));
    assert_eq!(steps.last(), Some(&(total, total)));
    assert!(steps.windows(2).all(|w| w[0].0 < w[1].0), "{steps:?}");
    for f in &run.findings {
        if f.level != Level::Pass {
            eprintln!("{f:?}");
        }
    }
    let op = &run.operations[0];
    assert_eq!(op.made, "generated");
    assert!(op.cut_mm > 0.0);
    let drawn: usize = op.strokes.iter().map(|s| s.points.len()).sum();
    assert!(drawn > 0 && drawn <= 5000 + op.strokes.len(), "{drawn}");
    let program = op.program.as_ref().unwrap();
    assert!(program.text.contains("G21"), "millimetres");
    // Each line says how many moves are made once it has run: none in the
    // header, every one by the end, and a cutting move where a move is.
    assert_eq!(program.moves.len(), program.text.lines().count());
    assert_eq!(program.moves.len(), program.lines);
    assert_eq!(program.moves.first(), Some(&0));
    let moves = run.playback.as_ref().unwrap().moments.last().unwrap().of;
    assert_eq!(program.moves.last().map(|m| *m as usize), Some(moves));
    assert!(program.moves.windows(2).all(|w| w[0] <= w[1]));
    let first_cut = program.text.lines().position(|l| l.starts_with("G1"));
    let first_cut = first_cut.expect("a cut");
    assert!(program.moves[first_cut] > program.moves[first_cut - 1]);
    let line = program.line_of(program.moves[first_cut] as usize);
    assert_eq!(line, Some(first_cut), "the line found again from its move");
    assert!(run.material.is_some());
    assert!(run.stock[7][2] > run.stock[0][2]);
    // Its boundary, the part's silhouette, drawn with where the tool may go
    // and what it cuts outside the part, at the stock's top.
    let drawn = op.outlines.as_ref().expect("a 3D operation's outlines");
    assert!(!drawn.boundary.is_empty() && !drawn.limit.is_empty());
    assert!(!drawn.cut.is_empty());
    let ring = &drawn.boundary[0];
    assert_eq!(ring.first(), ring.last(), "a ring closes");
    assert!((f64::from(ring[0][2]) - run.stock[7][2]).abs() < 1e-3);
}

/// A setup tilted 30° about X runs and verifies, its stock box turned with
/// it; posted for a machine with an A rotary, its program stands the part up
/// first, and one past the A's limits is a failed check that says why.
#[test]
fn a_tilted_setup_runs_and_is_posted_with_its_rotary() {
    let (s, c) = 30f64.to_radians().sin_cos();
    let job = JOB.replace(
        "post = \"grbl\"",
        &format!(
            "post = \"linuxcnc\"\norientation = {{ z = [0.0, {}, {c}], x = \"+x\" }}",
            -s
        ),
    );
    let profile = |rotary: &str| {
        let mut text =
            include_str!("../../camcraft/crates/camcraft-post/profiles/linuxcnc.toml").to_string();
        text.push_str(rotary);
        text
    };
    let request = |rotary: &str| Request {
        version: VERSION,
        job: job.clone(),
        shapes: BTreeMap::from([(
            "part".to_string(),
            Shape {
                text: native("hinge_jig.step"),
                placement: IDENTITY,
            },
        )]),
        post: Some(profile(rotary)),
        ..Request::default()
    };
    let run = answer(&request("\n[rotary]\naxes = [\"A\"]\n")).unwrap();
    for f in &run.findings {
        assert_ne!(f.level, Level::Fail, "{f:?}");
    }
    assert!(run.passed);
    // The box's X edge is the model's X, its Y edge leans with the tilt.
    let edge =
        |i: usize| -> [f64; 3] { std::array::from_fn(|k| run.stock[i][k] - run.stock[0][k]) };
    let (along_x, along_y) = (edge(1), edge(2));
    assert!(
        along_x[1].abs() < 1e-9 && along_x[2].abs() < 1e-9,
        "{along_x:?}"
    );
    let len = along_y.iter().map(|v| v * v).sum::<f64>().sqrt();
    assert!((along_y[1] / len - c).abs() < 1e-9 && (along_y[2] / len - s).abs() < 1e-9);
    let text = &run.operations[0].program.as_ref().unwrap().text;
    assert!(text.lines().any(|l| l == "G0 A-30"), "{text}");
    assert!(text.contains("tilted 30 deg"), "{text}");

    let run = answer(&request(
        "\n[rotary]\naxes = [\"A\"]\n[rotary.A]\nlimits = [0, 90]\n",
    ))
    .unwrap();
    assert!(!run.passed);
    let why = run
        .findings
        .iter()
        .find(|f| f.level == Level::Fail)
        .unwrap();
    assert!(why.text.contains("past the machine's limits"), "{why:?}");
}

#[test]
fn another_version_is_refused() {
    let request = Request {
        version: VERSION + 1,
        ..Request::default()
    };
    assert!(answer(&request).unwrap_err().contains("reinstall"));
}

#[test]
fn a_placed_body_is_machined_where_it_sits() {
    let at = |placement| {
        let request = Request {
            version: VERSION,
            job: JOB.into(),
            shapes: BTreeMap::from([(
                "part".to_string(),
                Shape {
                    text: native("hinge_jig.step"),
                    placement,
                },
            )]),
            max_points: 0,
            ..Request::default()
        };
        answer(&request).unwrap().stock
    };
    let mut moved = IDENTITY;
    moved[3] = 100.0;
    let (here, there) = (at(IDENTITY), at(moved));
    assert!((there[0][0] - here[0][0] - 100.0).abs() < 1e-6);
    assert!((there[0][1] - here[0][1]).abs() < 1e-6);
}

/// A bottom 2 mm under the stock's top keeps the cut above it, and the
/// heights come back as planes where they resolved.
#[test]
fn the_heights_hold_and_come_back_as_planes() {
    let job = JOB.replace(
        "passes = { stepdown = 3.0, stepover = 3.0 }",
        "passes = { stepdown = 3.0, stepover = 3.0 }\n\
         heights = { bottom = { ref = \"stock-top\", offset = -2.0 } }",
    );
    let request = Request {
        version: VERSION,
        job,
        shapes: BTreeMap::from([(
            "part".to_string(),
            Shape {
                text: native("hinge_jig.step"),
                placement: IDENTITY,
            },
        )]),
        max_points: 0,
        ..Request::default()
    };
    let run = answer(&request).unwrap();
    let top = run.stock[7][2];
    let op = &run.operations[0];
    assert_eq!(op.made, "generated");
    let plane = |name: &str| {
        op.heights
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("no {name} plane in {:?}", op.heights))
    };
    let clearance = plane("Clearance").corners[0][2];
    assert!(
        (f64::from(clearance) - (top + 10.0)).abs() < 1e-3,
        "{clearance}"
    );
    let bottom = plane("Bottom").corners[0][2];
    assert!((f64::from(bottom) - (top - 2.0)).abs() < 1e-3, "{bottom}");
    // Each corner a little outside the stock, across it.
    let corners = plane("Bottom").corners;
    assert!(f64::from(corners[0][0]) < run.stock[0][0]);
    assert!(f64::from(corners[2][0]) > run.stock[7][0]);

    let full = Request {
        max_points: 1_000_000,
        ..request
    };
    let run = answer(&full).unwrap();
    let lowest = run.operations[0]
        .strokes
        .iter()
        .filter(|s| !s.rapid)
        .flat_map(|s| s.points.iter())
        .map(|p| f64::from(p[2]))
        .fold(f64::INFINITY, f64::min);
    assert!(
        lowest >= top - 2.0 - 1e-3,
        "cut down to {lowest}, under {}",
        top - 2.0
    );
}

/// A profile's leads, which the package turns off as `linking = { leads =
/// false }`, change its program.
#[test]
fn a_profile_posts_its_leads() {
    let program = |linking: &str| {
        let job = JOB.replace(
            "strategy = \"pocket-clearing\"\ntool = \"flat-6\"\npasses = { stepdown = 3.0, stepover = 3.0 }",
            &format!("strategy = \"profile\"\ntool = \"flat-6\"\npasses = {{ stepdown = 6.0 }}\n{linking}"),
        );
        let request = Request {
            version: VERSION,
            job,
            shapes: BTreeMap::from([(
                "part".to_string(),
                Shape {
                    text: native("hinge_jig.step"),
                    placement: IDENTITY,
                },
            )]),
            max_points: 0,
            ..Request::default()
        };
        let run = answer(&request).unwrap();
        let op = &run.operations[0];
        assert_eq!(op.made, "generated", "{:?}", op.warnings);
        op.program.as_ref().unwrap().text.clone()
    };
    assert_ne!(program(""), program("linking = { leads = false }"));
}

/// A profile with tabs, as the package writes them, rides over them at its
/// deepest level, so its program changes.
#[test]
fn a_profile_leaves_its_tabs() {
    let cut = |tabs: &str| {
        let job = JOB.replace(
            "strategy = \"pocket-clearing\"\ntool = \"flat-6\"\npasses = { stepdown = 3.0, stepover = 3.0 }",
            &format!("strategy = \"profile\"\ntool = \"flat-6\"\npasses = {{ stepdown = 6.0{tabs} }}"),
        );
        let request = Request {
            version: VERSION,
            job,
            shapes: BTreeMap::from([(
                "part".to_string(),
                Shape {
                    text: native("hinge_jig.step"),
                    placement: IDENTITY,
                },
            )]),
            max_points: 0,
            ..Request::default()
        };
        let run = answer(&request).unwrap();
        let op = &run.operations[0];
        assert_eq!(op.made, "generated", "{:?}", op.warnings);
        op.program.as_ref().unwrap().text.clone()
    };
    assert_ne!(
        cut(""),
        cut(", tabs = { width = 6.0, height = 3.0, spacing = 40.0 }")
    );
}

/// A run plays back in order, within budget, the material only ever coming
/// off, and the tool's way drawn no further than the operation goes.
#[test]
fn a_run_plays_back_as_it_cut() {
    let job = JOB.replace(
        "passes = { stepdown = 3.0, stepover = 3.0 }",
        "passes = { stepdown = 3.0, stepover = 3.0 }\n\n\
         [[setup.operation]]\nname = \"finish\"\nstrategy = \"profile\"\ntool = \"flat-6\"",
    );
    let request = Request {
        version: VERSION,
        job,
        shapes: BTreeMap::from([(
            "part".to_string(),
            Shape {
                text: native("hinge_jig.step"),
                placement: IDENTITY,
            },
        )]),
        max_points: 2000,
        ..Request::default()
    };
    let run = answer(&request).unwrap();
    let play = run.playback.as_ref().expect("a playback");
    assert!(play.moments.len() > 10 && play.moments.len() <= 60 + run.operations.len());
    assert_eq!(play.indices.len() % 3, 0);
    let cells = play.base.len();
    let mut last: Option<&cam_protocol::Moment> = None;
    for m in &play.moments {
        assert_eq!(m.heights.len(), cells);
        let drawn: usize = run.operations[m.op]
            .strokes
            .iter()
            .map(|s| s.points.len())
            .sum();
        assert!(m.drawn <= drawn, "{} of {drawn}", m.drawn);
        if let Some(l) = last {
            assert!((l.op, l.moves) < (m.op, m.moves), "out of order");
            let grew = l
                .heights
                .iter()
                .zip(&m.heights)
                .filter(|(a, b)| b > a)
                .count();
            assert_eq!(grew, 0, "material came back at {grew} cells");
        }
        last = Some(m);
    }
    let end = last.unwrap();
    assert_eq!((end.op, end.moves), (1, end.of));
    // The clock runs on with the moves, and ends with the operations' time.
    assert!(
        play.moments
            .windows(2)
            .all(|w| w[0].seconds <= w[1].seconds)
    );
    let total: f64 = run.operations.iter().map(|o| o.seconds).sum();
    assert!(run.operations.iter().all(|o| o.seconds > 0.0));
    assert!(
        (end.seconds - total).abs() < 1e-6,
        "{} of {total}",
        end.seconds
    );
    for o in &run.operations {
        assert!(o.marks.windows(2).all(|w| w[0].moves < w[1].moves));
    }
    let first = &play.moments[0];
    assert!(
        first.heights.iter().zip(&end.heights).any(|(a, b)| b < a),
        "nothing came off"
    );
    // The answer stays small enough to send every run.
    let bytes = serde_json::to_string(play).unwrap().len();
    assert!(bytes < 2_000_000, "{bytes} bytes");
}

/// A file the package names is read for it; one too big, or not there, is
/// refused with a reason.
#[test]
fn a_file_is_read_for_the_package() {
    let request = Request {
        version: VERSION,
        ..Request::default()
    };
    let path = format!("{}/Cargo.toml", env!("CARGO_MANIFEST_DIR"));
    assert!(
        crate::read(&request, &path)
            .unwrap()
            .contains("camcraft-helper")
    );
    assert!(
        crate::read(&request, "/no/such/file")
            .unwrap_err()
            .contains("cannot read")
    );
    let old = Request {
        version: VERSION + 1,
        ..Request::default()
    };
    assert!(crate::read(&old, &path).unwrap_err().contains("reinstall"));
}

/// A setup posts with a profile of the user's that the request carries, and
/// an operation whose tool turns outside that machine's spindle range is
/// warned about.
#[test]
fn a_post_of_ones_own_writes_the_program() {
    let grbl = include_str!("../../camcraft/crates/camcraft-post/profiles/grbl.toml");
    let mine = grbl.replacen("\"GRBL\"", "\"My mill\"", 1).replacen(
        "extension = \"nc\"",
        "extension = \"mine\"\nspindle = [18000, 24000]",
        1,
    );
    let request = Request {
        version: VERSION,
        job: JOB.replace("post = \"grbl\"", "post = \"My mill\""),
        shapes: BTreeMap::from([(
            "part".to_string(),
            Shape {
                text: native("hinge_jig.step"),
                placement: IDENTITY,
            },
        )]),
        max_points: 0,
        post: Some(mine),
        read: None,
    };
    let run = answer(&request).unwrap();
    let program = run.operations[0].program.as_ref().unwrap();
    assert_eq!(program.post, "My mill");
    assert_eq!(program.extension, "mine");
    let warned = &run.operations[0].warnings;
    assert!(
        warned.iter().any(|w| w.contains("16000 rev/min")),
        "{warned:?}"
    );
}

/// A playback session runs the setup once, then answers each seek with the
/// material that changed: all of it first, finished against the part at
/// the end, and as it was at the start again going back.
#[test]
fn a_session_plays_back_at_full_resolution() {
    use cam_protocol::{SessionIn, SessionOut, class, unbase64};
    let request = Request {
        version: VERSION,
        job: JOB.into(),
        shapes: BTreeMap::from([(
            "part".to_string(),
            Shape {
                text: native("hinge_jig.step"),
                placement: IDENTITY,
            },
        )]),
        max_points: 2000,
        ..Request::default()
    };
    let seek = |s: f64| serde_json::to_string(&SessionIn::Seek { seconds: s }).unwrap();
    let lines = format!("{}\n{}\n{}\n", seek(0.0), seek(1e9), seek(0.0));
    let mut out = Vec::new();
    crate::session::serve(
        SessionIn::Open(Box::new(request)),
        std::io::Cursor::new(lines),
        &mut out,
    )
    .unwrap();
    let said: Vec<SessionOut> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let SessionOut::Ready(ready) = &said[0] else {
        panic!("{:?}", said[0]);
    };
    assert!(ready.cols > 100 && ready.cols <= 600, "{}", ready.cols);
    assert_eq!(ready.times.len(), 1);
    assert!(ready.bare.is_empty(), "a box has stock all over");
    let patch = |i: usize| match &said[i] {
        SessionOut::Stock(p) => p.clone(),
        other => panic!("{other:?}"),
    };
    let (start, end, back) = (patch(1), patch(2), patch(3));
    assert_eq!(
        (start.width, start.height),
        (ready.cols, ready.rows),
        "all of it first"
    );
    let heights = |p: &cam_protocol::StockPatch| -> Vec<u16> {
        unbase64(&p.heights)
            .unwrap()
            .chunks(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect()
    };
    assert_eq!(heights(&start).len(), (start.width * start.height) as usize);
    assert_eq!((end.moves, end.of), (end.of, ready.times[0].len()));
    assert!(end.width > 0, "the run changed the material");
    let finished = unbase64(&end.classes).unwrap();
    assert!(
        finished.contains(&class::FINISHED),
        "cut down to the part somewhere"
    );
    assert!(!finished.contains(&class::GOUGE), "and nowhere below it");
    // Back at the start the block is whole again where it changed.
    let top = heights(&start).into_iter().max().unwrap();
    assert!(heights(&back).iter().all(|h| *h == top));
}

/// A session opened with a run's own request times its moves as the run
/// did, and says so with the run's timing; a job changed since (here a
/// finer stepover) is told apart by it, so the package does not step and
/// seek by a session that ran something else.
#[test]
fn a_session_times_its_moves_as_the_run_did() {
    use cam_protocol::{SessionIn, SessionOut};
    let request = |job: String| Request {
        version: VERSION,
        job,
        shapes: BTreeMap::from([(
            "part".to_string(),
            Shape {
                text: native("hinge_jig.step"),
                placement: IDENTITY,
            },
        )]),
        max_points: 2000,
        ..Request::default()
    };
    let asked = request(JOB.into());
    let run = answer(&asked).unwrap();
    assert_ne!(run.timing, 0);
    let mut out = Vec::new();
    crate::session::serve(
        SessionIn::Open(Box::new(asked.clone())),
        std::io::Cursor::new(""),
        &mut out,
    )
    .unwrap();
    let said: SessionOut = serde_json::from_slice(&out).unwrap();
    let SessionOut::Ready(ready) = said else {
        panic!("{said:?}");
    };
    assert_eq!(ready.timing, run.timing);
    let mut start = 0.0;
    for (op, times) in run.operations.iter().zip(&ready.times) {
        start += op.seconds;
        let end = f64::from(times.last().copied().unwrap_or(0.0));
        assert!((end - start).abs() < 1e-3 * start.max(1.0), "{end} {start}");
    }
    // The run again, asked as it was: the same timing.
    assert_eq!(answer(&asked).unwrap().timing, run.timing);
    let finer = request(JOB.replace("stepover = 3.0", "stepover = 2.5"));
    assert_ne!(answer(&finer).unwrap().timing, run.timing);
}

#[test]
fn each_line_counts_the_moves_of_the_instruction_that_wrote_it() {
    // Two header lines; an instruction of two lines making none, one of a
    // line making a move, one writing nothing, one of two lines making two
    // more; a footer line.
    let made = [0, 1, 1, 3];
    let op_lines = [4, 5, 5, 7];
    assert_eq!(
        crate::line_moves(&made, &op_lines, 8),
        vec![0, 0, 0, 0, 1, 3, 3, 3]
    );
    assert_eq!(
        crate::line_moves(&[], &[], 2),
        vec![0, 0],
        "no instructions"
    );
}

#[test]
fn each_line_of_a_setups_program_says_whose_move_it_is() {
    // A header line; operation 1's two lines making none, then a move;
    // operation 3's tool change making none, then two moves; a footer line.
    let made = [(1, 0), (1, 1), (3, 0), (3, 2)];
    let op_lines = [3, 4, 5, 6];
    assert_eq!(
        crate::line_moments(&made, &op_lines, 7),
        (vec![1, 1, 1, 1, 3, 3, 3], vec![0, 0, 0, 1, 0, 2, 2])
    );
}

/// A job tapping one of the sled base's holes, picked by its top and bottom
/// rims, left-hand, posted for `post`.
fn tapping_job(post: &str) -> Request {
    let job = r#"
version = 1
tolerance = 0.1

[tools.tap]
kind = "flat"
diameter = 5.08
number = 7
rpm = 600
comment = "M5 tap"

[[model]]
name = "sled"
path = "sled"

[[stock]]
name = "block"
kind = "around-models"
margin = [2.0, 2.0, 2.0]

[[setup]]
name = "top"
models = ["sled"]
stocks = ["block"]
post = "grbl"
programs = "operation"

[[setup.operation]]
name = "threads"
strategy = "drill"
pitch = 0.8
hand = "left"
tool = "tap"
selection = { edges = [
    { model = "sled", index = 86, at = [228.11, 195.25, 40.894] },
    { model = "sled", index = 100, at = [228.11, 195.25, 1.016] },
] }
"#;
    Request {
        version: VERSION,
        job: job.replace("post = \"grbl\"", &format!("post = \"{post}\"")),
        shapes: BTreeMap::from([(
            "sled".to_string(),
            Shape {
                text: native("sled_base.step"),
                placement: IDENTITY,
            },
        )]),
        max_points: 0,
        post: None,
        read: None,
    }
}

/// The tap, for a post without a tapping cycle: the run is shown but fails,
/// saying the machine cannot tap and which can, and has no program; for one
/// of the post's own with the cycle, it passes, a G74 a hole.
#[test]
fn a_tap_posts_only_where_the_post_has_the_cycle() {
    let refused = answer(&tapping_job("grbl")).unwrap();
    assert!(!refused.passed);
    let said = &refused.findings[0];
    assert_eq!((said.level, said.operation), (Level::Fail, Some(0)));
    assert!(said.text.contains("GRBL cannot tap"), "{said:?}");
    assert!(said.text.contains("LinuxCNC"), "{said:?}");
    let op = &refused.operations[0];
    assert!(op.program.is_none());
    assert!(op.cut_mm > 0.0, "its path is still shown");

    let grbl = include_str!("../../camcraft/crates/camcraft-post/profiles/grbl.toml");
    let cycles = grbl.replacen("tap = \"none\"", "tap = \"g84\"", 1);
    let run = answer(&Request {
        post: Some(cycles),
        ..tapping_job("grbl")
    })
    .unwrap();
    for f in run.findings.iter().filter(|f| f.level == Level::Fail) {
        eprintln!("{f:?}");
    }
    assert!(run.passed);
    let program = run.operations[0].program.as_ref().unwrap();
    assert!(program.text.contains("S600 M4"), "{}", program.text);
    let taps: Vec<&str> = program
        .text
        .lines()
        .filter(|l| l.starts_with("G74"))
        .collect();
    assert_eq!(taps.len(), 1, "{}", program.text);
    assert!(
        taps[0].ends_with("F480"),
        "0.8 mm a turn at 600: {}",
        taps[0]
    );
}

/// The tap on each built-in post: one whose controller taps writes a
/// program and passes, LinuxCNC tapping with G33.1, CNC12 and Haas with
/// their cycle, Fanuc with it after M29, Heidenhain with cycle 207; the
/// others fail, saying so.
#[test]
fn a_tap_on_a_builtin_post_writes_a_program_where_it_can() {
    for (post, word) in [
        ("linuxcnc", Some("G33.1 Z")),
        ("linuxcnc-inch", Some("G33.1 Z")),
        ("centroid-avid", Some("G74 X")),
        ("centroid-avid-inch", Some("G74 X")),
        ("fanuc", Some("G74 X")),
        ("haas-inch", Some("G74 X")),
        ("heidenhain", Some("CYCL DEF 207")),
        ("mach", None),
        ("masso", None),
        ("masso-inch", None),
        ("grbl", None),
    ] {
        let run = answer(&tapping_job(post)).unwrap();
        let program = run.operations[0].program.as_ref();
        match word {
            Some(word) => {
                for f in run.findings.iter().filter(|f| f.level == Level::Fail) {
                    eprintln!("{post}: {f:?}");
                }
                assert!(run.passed, "{post}");
                let text = &program.unwrap().text;
                assert!(text.contains("M4"), "{post}: {text}");
                let taps = text.lines().filter(|l| l.contains(word)).count();
                assert_eq!(taps, 1, "{post}: {text}");
            }
            None => {
                assert!(!run.passed, "{post}");
                assert!(program.is_none(), "{post}");
                assert!(
                    run.findings.iter().any(|f| f.text.contains("cannot tap")),
                    "{post}: {:?}",
                    run.findings
                );
            }
        }
    }
}

/// Round stock comes back as a cylinder, in the model's coordinates, inside
/// the box it gives; and the material, played back or swept again, is drawn
/// over the disc only, not its box's corners.
#[test]
fn round_stock_is_drawn_round() {
    use cam_protocol::{SessionIn, SessionOut, is_bare, unbase64};
    let request = Request {
        version: VERSION,
        job: JOB.replace(
            "kind = \"around-models\"\nmargin = [2.0, 2.0, 2.0]",
            "kind = \"cylinder-around-models\"\naxis = \"z\"\nmargin = 3.0\nends = 1.0",
        ),
        shapes: BTreeMap::from([(
            "part".to_string(),
            Shape {
                text: native("hinge_jig.step"),
                placement: IDENTITY,
            },
        )]),
        max_points: 2000,
        ..Request::default()
    };
    let run = answer(&request).unwrap();
    let c = run.cylinder.expect("a cylinder");
    let (lo, hi) = (run.stock[0], run.stock[7]);
    let [a, b] = c.ends;
    // Up the tool, its ends' middles over each other, at the box's foot
    // and top, and the box as wide as it.
    assert_eq!((a[0], a[1]), (b[0], b[1]));
    assert!((a[2] - lo[2]).abs() < 1e-9 && (b[2] - hi[2]).abs() < 1e-9);
    assert!((hi[0] - lo[0] - 2.0 * c.radius).abs() < 1e-9);
    assert!((hi[1] - lo[1] - 2.0 * c.radius).abs() < 1e-9);
    let middle = [a[0], a[1]];
    // No triangle of the material over a corner of the box.
    let outside = |p: [f32; 3]| {
        let (dx, dy) = (f64::from(p[0]) - middle[0], f64::from(p[1]) - middle[1]);
        (dx * dx + dy * dy).sqrt()
    };
    let check = |positions: &[[f32; 3]], indices: &[u32], what: &str| {
        let squares = indices.len() / 6;
        assert!(squares > 0, "{what}: none drawn");
        for t in indices.chunks(3) {
            let centre = std::array::from_fn(|i| {
                t.iter().map(|&k| positions[k as usize][i]).sum::<f32>() / 3.0
            });
            assert!(
                outside(centre) < c.radius * 1.05,
                "{what}: a triangle {} out",
                outside(centre)
            );
        }
    };
    let material = run.material.as_ref().unwrap();
    check(&material.positions, &material.indices, "material");
    let play = run.playback.as_ref().unwrap();
    let base: Vec<[f32; 3]> = play.base.clone();
    check(&base, &play.indices, "playback");

    // The session says which squares are bare: the corners, not the middle.
    let mut out = Vec::new();
    crate::session::serve(
        SessionIn::Open(Box::new(request)),
        std::io::Cursor::new(String::new()),
        &mut out,
    )
    .unwrap();
    let said: SessionOut = serde_json::from_str(String::from_utf8(out).unwrap().trim()).unwrap();
    let SessionOut::Ready(ready) = said else {
        panic!("{said:?}");
    };
    let bare = unbase64(&ready.bare).unwrap();
    let (w, h) = ((ready.cols - 1) as usize, (ready.rows - 1) as usize);
    assert!(
        is_bare(&bare, 0) && is_bare(&bare, w * h - 1),
        "the corners"
    );
    assert!(!is_bare(&bare, (h / 2) * w + w / 2), "the middle");
    let count = (0..w * h).filter(|&i| is_bare(&bare, i)).count() as f64;
    // A square less its circle: 1 − π/4 of it.
    let share = count / (w * h) as f64;
    assert!(
        (share - (1.0 - std::f64::consts::FRAC_PI_4)).abs() < 0.05,
        "{share}"
    );
}

/// An engraving of a circle and a line drawn over the part: its arcs are
/// posted as arcs, what it follows is drawn, the line left open, and what it
/// cuts into the part is meant, so nothing gouges.
#[test]
fn an_engraving_posts_its_arcs_and_draws_its_curves() {
    let part = BTreeMap::from([(
        "part".to_string(),
        Shape {
            text: native("hinge_jig.step"),
            placement: IDENTITY,
        },
    )]);
    let first = Request {
        version: VERSION,
        job: JOB.to_string(),
        shapes: part.clone(),
        ..Request::default()
    };
    let stock = answer(&first).unwrap().stock;
    let (lo, hi) = (stock[0], stock[7]);
    let (x, y) = (
        f64::from(lo[0] + hi[0]) / 2.0,
        f64::from(lo[1] + hi[1]) / 2.0,
    );
    let r = 3.0;
    let curves = format!(
        "selection = {{ curves = [\n\
         {{ start = [{a}, {y}, 0.0], segments = [{{ via = [{x}, {b}, 0.0], to = [{c}, {y}, 0.0] }}, \
         {{ via = [{x}, {d}, 0.0], to = [{a}, {y}, 0.0] }}] }},\n\
         {{ start = [{c}, {e}, 0.0], segments = [{{ to = [{a}, {e}, 0.0] }}] }},\n] }}",
        a = x + r,
        b = y + r,
        c = x - r,
        d = y - r,
        e = y + 2.0 * r,
    );
    let job = JOB.replace(
        "strategy = \"pocket-clearing\"\ntool = \"flat-6\"\npasses = { stepdown = 3.0, stepover = 3.0 }",
        &format!(
            "strategy = \"engrave\"\ntool = \"engraver\"\ndepth = 0.3\npasses = {{ feed = 600.0 }}\n{curves}"
        ),
    )
    .replace(
        "[[model]]",
        "[tools.engraver]\nkind = \"ball\"\ndiameter = 1.0\nnumber = 2\nrpm = 18000\n\n[[model]]",
    );
    let request = Request {
        version: VERSION,
        job,
        shapes: part,
        max_points: 5000,
        ..Request::default()
    };
    let run = answer(&request).unwrap();
    let op = &run.operations[0];
    assert_eq!(op.made, "generated", "{:?}", op.warnings);
    for f in run.findings.iter().filter(|f| f.level == Level::Fail) {
        panic!("{f:?}");
    }
    let program = &op.program.as_ref().unwrap().text;
    let arcs = program
        .lines()
        .filter(|l| l.split_whitespace().any(|w| w == "G2" || w == "G3"))
        .count();
    assert!(arcs >= 4, "{program}");
    let drawn = op.outlines.as_ref().expect("what it follows, drawn");
    assert_eq!(drawn.boundary.len(), 2);
    let open = drawn
        .boundary
        .iter()
        .filter(|l| l.first() != l.last())
        .count();
    assert_eq!(open, 1, "the line is left open");
    assert!(!drawn.cut.is_empty());

    // Twice, 10 mm apart: both cut and posted, and both drawn.
    let request = Request {
        job: request.job.replace(
            "depth = 0.3\n",
            "depth = 0.3\ntransform = { linear = { x_count = 2, x_spacing = 10.0 } }\n",
        ),
        ..request
    };
    let run = answer(&request).unwrap();
    let twice = &run.operations[0];
    assert_eq!(twice.made, "generated", "{:?}", twice.warnings);
    for f in run.findings.iter().filter(|f| f.level == Level::Fail) {
        panic!("{f:?}");
    }
    let program = &twice.program.as_ref().unwrap().text;
    let arcs2 = program
        .lines()
        .filter(|l| l.split_whitespace().any(|w| w == "G2" || w == "G3"))
        .count();
    assert_eq!(arcs2, 2 * arcs, "{program}");
    let drawn = twice.outlines.as_ref().unwrap();
    assert_eq!(drawn.boundary.len(), 4);
    let shift = drawn.boundary[2][0][0] - drawn.boundary[0][0][0];
    assert!((shift - 10.0).abs() < 1e-3, "{shift}");
}

/// The day a program is posted on, counted from 1970 as the clock counts.
#[test]
fn days_are_dates() {
    assert_eq!(crate::civil(0), (1970, 1, 1));
    assert_eq!(crate::civil(11_016), (2000, 2, 29));
    assert_eq!(crate::civil(20_760), (2026, 11, 3));
}

/// The job, its rough between two Custom G-code operations, posted with a
/// profile of one's own: `gcode` is the first one's text, and `travel` the
/// profile's travel table.
fn custom_request(gcode: &str, travel: &str) -> Request {
    let grbl = include_str!("../../camcraft/crates/camcraft-post/profiles/grbl.toml");
    let mine = grbl
        .replacen("\"GRBL\"", "\"My mill\"", 1)
        .replacen(
            "header = [",
            "tool_change_lines = [\"(fit tool @tool@)\", \"T@tool@\", \"M0\"]\n\
             header = [\n  \"(posted @date@)\",",
            1,
        )
        .replacen("[format]", &format!("{travel}\n[format]"), 1);
    let job = JOB.replacen(
        "[[setup.operation]]\nname = \"rough\"",
        &format!(
            "[[setup.operation]]\nname = \"clamps\"\nstrategy = \"custom-gcode\"\n\
             gcode = \"\"\"{gcode}\"\"\"\n\n[[setup.operation]]\nname = \"rough\""
        ),
        1,
    ) + "\n[[setup.operation]]\nname = \"done\"\nstrategy = \"custom-gcode\"\n\
         gcode = \"M64 P1\"\n";
    Request {
        version: VERSION,
        job: job.replace("post = \"grbl\"", "post = \"My mill\""),
        shapes: BTreeMap::from([(
            "part".to_string(),
            Shape {
                text: native("hinge_jig.step"),
                placement: IDENTITY,
            },
        )]),
        max_points: 0,
        post: Some(mine),
        read: None,
    }
}

/// A setup posted as one program, as it is unless it says otherwise: the
/// run has it, no operation one of its own; one header and footer, custom
/// G-code where it stands; and each line says whose move it is, which the
/// operations' own lines and moves find again.
#[test]
fn a_setup_is_one_program() {
    let job = JOB.replace("programs = \"operation\"\n", "number = 42\n")
        + r#"
[[setup.operation]]
name = "between"
strategy = "custom-gcode"
gcode = "M0 (look)"

[[setup.operation]]
name = "again"
strategy = "pocket-clearing"
tool = "flat-6"
passes = { stepdown = 3.0, stepover = 2.5, feed = 900.0 }
"#;
    let request = Request {
        version: VERSION,
        job,
        shapes: BTreeMap::from([(
            "part".to_string(),
            Shape {
                text: native("hinge_jig.step"),
                placement: IDENTITY,
            },
        )]),
        max_points: 0,
        ..Request::default()
    };
    let run = answer(&request).unwrap();
    assert!(run.operations.iter().all(|o| o.program.is_none()));
    let program = run.program.as_ref().expect("the setup's program");
    assert_eq!(program.name, "top");
    let text = &program.text;
    let at = |w: &str| {
        text.find(w)
            .unwrap_or_else(|| panic!("no {w:?} in\n{text}"))
    };
    assert!(at("(rough - ") < at("(custom gcode: between)"));
    assert!(at("(custom gcode: between)") < at("(again - "));
    assert_eq!(text.matches("M30").count(), 1);
    assert_eq!(program.moves.len(), program.lines);
    assert_eq!(program.operations.len(), program.lines);
    assert_eq!(program.lines, text.lines().count());
    let moments: Vec<(usize, usize)> = (0..program.lines)
        .map(|l| program.moment(l).unwrap())
        .collect();
    assert!(moments.windows(2).all(|w| w[0] <= w[1]));
    assert_eq!(moments.first(), Some(&(0, 0)));
    let ends = |k: usize| {
        let moments = &run.playback.as_ref().unwrap().moments;
        moments.iter().rfind(|m| m.op == k).unwrap().of
    };
    assert_eq!(moments.last(), Some(&(2, ends(2))));
    assert!(moments.contains(&(0, ends(0))));
    // The second operation's first cut, found again from its move.
    let lines: Vec<&str> = text.lines().collect();
    let again = lines
        .iter()
        .position(|l| l.starts_with("(again - "))
        .unwrap();
    let first_cut = again
        + lines[again..]
            .iter()
            .position(|l| l.starts_with("G1"))
            .unwrap();
    let (op, made) = program.moment(first_cut).unwrap();
    assert_eq!(op, 2);
    assert_eq!(program.line_at(op, made), Some(first_cut));
}

/// Custom G-code is written into the program beside it, where it stands,
/// its placeholders filled from that program, with the profile's own tool
/// change and the day it was posted; it has no program of its own, and
/// cuts nothing.
#[test]
fn custom_gcode_goes_into_the_program_beside_it() {
    let request = custom_request("M0 (check the clamps for @operation@)\nG4 P1", "");
    let run = answer(&request).unwrap();
    let failed: Vec<_> = run
        .findings
        .iter()
        .filter(|f| f.level == Level::Fail)
        .collect();
    assert!(run.passed, "{failed:?}");
    let names: Vec<&str> = run.operations.iter().map(|o| o.made.as_str()).collect();
    assert_eq!(names, ["custom-gcode", "generated", "custom-gcode"]);
    assert!(run.operations[0].program.is_none());
    assert_eq!(run.operations[0].summary, "2 line(s) of custom G-code");
    assert!(run.operations[0].strokes.is_empty());
    let text = &run.operations[1].program.as_ref().unwrap().text;
    let at = |line: &str| {
        text.lines()
            .position(|l| l == line)
            .unwrap_or_else(|| panic!("no {line:?} in\n{text}"))
    };
    assert!(at("(custom gcode: clamps)") < at("M0 (check the clamps for rough)"));
    assert!(at("G4 P1") < at("(rough - pocket-clearing - T1 G54)"));
    assert!(at("(fit tool 1)") < at("T1"));
    assert!(at("(rough - pocket-clearing - T1 G54)") < at("(custom gcode: done)"));
    assert!(at("M64 P1") < at("M30"));
    assert!(
        !text.split_whitespace().any(|w| w == "M6"),
        "its own tool change: {text}"
    );
    let posted = text.lines().next().unwrap();
    assert!(
        posted.len() == "(posted 2026-11-03)".len() && posted.starts_with("(posted 20"),
        "{posted}"
    );
}

/// What moves the machine in custom G-code is a failed check on its own
/// operation; a call of a subprogram a warning.
#[test]
fn custom_gcode_that_moves_is_a_failed_check() {
    let run = answer(&custom_request("M0\nG0 Z5", "")).unwrap();
    assert!(!run.passed);
    let f = run
        .findings
        .iter()
        .find(|f| f.level == Level::Fail)
        .unwrap();
    assert_eq!((f.code.as_str(), f.operation), ("CC108", Some(0)), "{f:?}");
    assert!(f.text.contains("line 2: G0 is refused"), "{f:?}");

    let run = answer(&custom_request("M98 P100", "")).unwrap();
    assert!(run.passed);
    assert!(
        run.findings
            .iter()
            .any(|f| f.code == "CC216" && f.level == Level::Warn && f.operation == Some(0)),
        "{:?}",
        run.findings
    );
}

/// Inside the machine's travel, a program passes; past it, it is a failed
/// check on the operation that goes there, saying which axis and how far.
#[test]
fn a_program_past_the_travel_is_a_failed_check() {
    let inside = answer(&custom_request("M0", "[travel]\nz = [-100, 100]")).unwrap();
    assert!(inside.passed, "{:?}", inside.findings);
    let past = answer(&custom_request("M0", "[travel]\nz = [-1, 100]")).unwrap();
    assert!(!past.passed);
    let f = past.findings.iter().find(|f| f.code == "travel").unwrap();
    assert_eq!((f.level, f.operation), (Level::Fail, Some(1)), "{f:?}");
    assert!(
        f.text.starts_with("travel rough: Z goes from -") && f.text.contains("of -1 to 100 mm"),
        "{}",
        f.text
    );
}

/// Travel in machine coordinates: a setup that does not say where its work
/// offset is is not checked, and a note says so; one whose offset puts the
/// cut past the machine's travel is a failed check naming the axis, how far
/// and the operation. With length offsets, a tool of no known length leaves
/// Z unchecked, and says so.
#[test]
fn machine_travel_is_checked_where_the_work_offset_is() {
    let travel = "[travel]\ncoordinates = \"machine\"\nx = [0, 10000]\nz = [-10000, 0]";
    let unknown = answer(&custom_request("M0", travel)).unwrap();
    assert!(unknown.passed, "{:?}", unknown.findings);
    let note = unknown
        .findings
        .iter()
        .find(|f| f.code == "travel")
        .unwrap();
    assert_eq!(
        (note.level, note.operation),
        (Level::Note, None),
        "{note:?}"
    );
    assert!(
        note.text
            .starts_with("travel: My mill's travel is in machine coordinates"),
        "{}",
        note.text
    );

    let placed = |at: &str, travel: &str| {
        let mut request = custom_request("M0", travel);
        request.job = request.job.replacen(
            "post = \"My mill\"",
            &format!("post = \"My mill\"\nwork_offset = {at}"),
            1,
        );
        answer(&request).unwrap()
    };
    let inside = placed("[5000.0, 0.0, -5000.0]", travel);
    assert!(inside.passed, "{:?}", inside.findings);
    assert!(!inside.findings.iter().any(|f| f.code == "travel"));
    let past = placed("[-5000.0, 0.0, -5000.0]", travel);
    assert!(!past.passed);
    let f = past.findings.iter().find(|f| f.code == "travel").unwrap();
    assert_eq!((f.level, f.operation), (Level::Fail, Some(1)), "{f:?}");
    assert!(
        f.text.starts_with("travel rough: X goes from -")
            && f.text.contains("mm in machine coordinates, ")
            && f.text
                .contains(" mm under My mill's travel of 0 to 10000 mm"),
        "{}",
        f.text
    );

    // The job's tool has no length: with G43, Z cannot be placed.
    let g43 = format!("length_offset = true\n{travel}");
    let run = placed("[5000.0, 0.0, -5000.0]", &g43);
    assert!(run.passed, "{:?}", run.findings);
    let note = run.findings.iter().find(|f| f.code == "travel").unwrap();
    assert_eq!(note.level, Level::Note);
    assert!(
        note.text.starts_with("travel: Z is not checked"),
        "{}",
        note.text
    );
}

/// The sled base's M5 hole, picked by its rims, threaded with a thread mill:
/// it passes, its thread a run of helixes, `G3` with a `Z`, climbing up the
/// hole, in the setup's one program; and with the same diameter, the
/// sled's other holes as wide are threaded too.
#[test]
fn a_thread_is_milled_in_every_hole_as_wide() {
    let job = |same: bool| {
        format!(
            r#"
version = 1
tolerance = 0.1

[tools.mill]
kind = "thread-mill"
diameter = 3.5
number = 8
rpm = 12000
tip_length = 0.3
pitch_range = [0.5, 1.0]

[[model]]
name = "sled"
path = "sled"

[[stock]]
name = "block"
kind = "around-models"
margin = [2.0, 2.0, 2.0]

[[setup]]
name = "top"
models = ["sled"]
stocks = ["block"]
post = "grbl"

[[setup.operation]]
name = "threads"
strategy = "thread"
pitch = 0.8
diameter = 5.08
tool = "mill"
passes = {{ feed = 300.0 }}
holes = {{ same_diameter = {same} }}
selection = {{ edges = [
    {{ model = "sled", index = 86, at = [228.11, 195.25, 40.894] }},
    {{ model = "sled", index = 100, at = [228.11, 195.25, 1.016] }},
] }}
"#
        )
    };
    let request = |job: String| Request {
        version: VERSION,
        job,
        shapes: BTreeMap::from([(
            "sled".to_string(),
            Shape {
                text: native("sled_base.step"),
                placement: IDENTITY,
            },
        )]),
        max_points: 0,
        post: None,
        read: None,
    };
    let helixes = |same: bool| {
        let run = answer(&request(job(same))).unwrap();
        for f in run.findings.iter().filter(|f| f.level == Level::Fail) {
            eprintln!("{f:?}");
        }
        assert!(run.passed, "same diameter {same}");
        // Posted as the setup's one program, as a setup is unless asked.
        assert!(run.operations[0].program.is_none());
        let program = run.program.as_ref().unwrap();
        let up: Vec<&str> = program
            .text
            .lines()
            .filter(|l| l.split_whitespace().any(|w| w == "G3") && l.contains('Z'))
            .collect();
        assert!(!up.is_empty(), "{}", program.text);
        assert!(
            !program
                .text
                .lines()
                .any(|l| l.split_whitespace().any(|w| w == "G2")),
            "climbing up, right-hand: {}",
            program.text
        );
        up.len()
    };
    let one = helixes(false);
    let all = helixes(true);
    assert!(all > one, "{one} then {all}");
}

/// The hinge jig roughed then profiled, as a request.
fn rough_then_profile(rough: &str) -> Request {
    let job = format!(
        "{JOB}{rough}\n[[setup.operation]]\nname = \"outline\"\nstrategy = \"profile\"\n\
         tool = \"flat-6\"\npasses = {{ stepdown = 6.0 }}\n"
    );
    Request {
        version: VERSION,
        job,
        shapes: BTreeMap::from([(
            "part".to_string(),
            Shape {
                text: native("hinge_jig.step"),
                placement: IDENTITY,
            },
        )]),
        max_points: 2000,
        ..Request::default()
    }
}

/// An operation turned off is in the run but made nothing: no toolpath, no
/// program, no checks, no time in playback. The one after it cuts the
/// material as if it were not there: the same program as with it gone.
#[test]
fn an_operation_turned_off_is_left_out() {
    let off = answer(&rough_then_profile("enabled = false\n")).unwrap();
    let rough = &off.operations[0];
    assert_eq!(rough.made, "disabled");
    assert!(rough.program.is_none());
    assert!(rough.strokes.is_empty() && rough.points == 0);
    assert_eq!(rough.cut_mm, 0.0);
    assert!(off.findings.iter().all(|f| f.operation != Some(0)));
    assert_eq!(off.operations[1].made, "generated");

    // With it gone, the outline is the only operation.
    let mut gone = rough_then_profile("");
    gone.job = gone.job.replace(
        "[[setup.operation]]\nname = \"rough\"\nstrategy = \"pocket-clearing\"\ntool = \"flat-6\"\npasses = { stepdown = 3.0, stepover = 3.0 }\n",
        "",
    );
    assert!(!gone.job.contains("rough"), "{}", gone.job);
    let gone = answer(&gone).unwrap();
    let text = |r: &cam_protocol::Run, i: usize| {
        r.operations[i]
            .program
            .as_ref()
            .unwrap()
            .text
            .lines()
            .filter(|l| !l.contains("(") && !l.starts_with(';'))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(text(&off, 1), text(&gone, 0));
    // On again, it cuts first, and the outline after it has less to take.
    let on = answer(&rough_then_profile("")).unwrap();
    assert_eq!(on.operations[0].made, "generated");
    assert_ne!(text(&on, 1), text(&gone, 0));
}

/// A profile asked for corner relief posts longer: out into each inside
/// corner and back. The verifier takes the notches as meant, not gouges.
#[test]
fn a_profile_posts_its_dogbones() {
    let cut = |relief: &str| {
        let mut request = rough_then_profile("enabled = false\n");
        request.job = request.job.replace(
            "passes = { stepdown = 6.0 }",
            &format!("passes = {{ stepdown = 6.0{relief} }}"),
        );
        let run = answer(&request).unwrap();
        let op = &run.operations[1];
        assert_eq!(op.made, "generated", "{:?}", op.warnings);
        let gouges = run
            .findings
            .iter()
            .filter(|f| f.level == Level::Fail)
            .count();
        (op.program.as_ref().unwrap().text.lines().count(), gouges)
    };
    let (plain, failed) = cut("");
    let (relieved, failed_relieved) = cut(", relief = { style = \"dogbone\" }");
    assert!(relieved > plain, "{relieved} lines against {plain}");
    assert_eq!(failed_relieved, failed);
}

/// A known path takes the seconds its moves take: each rapid as long as its
/// slowest axis at the machine's rates, each feed at its feed, but never
/// quicker than its axes can go.
#[test]
fn a_path_is_timed_at_its_feeds_and_the_machines_rapids() {
    use camcraft_core::path::toolpath::{MoveRole, Toolpath};
    use camcraft_core::setup::frame::Frame;
    use camcraft_post::profile::Rapid;
    use glam::DVec3;

    let rapid = Rapid {
        x: 5000.0,
        y: 5000.0,
        z: 2500.0,
    };
    let mut path = Toolpath::new();
    // Where it starts: no time.
    path.rapid_to(DVec3::new(0.0, 0.0, 10.0)).unwrap();
    // 100 mm along X at 5000 mm/min: 1.2 s.
    path.rapid_to(DVec3::new(100.0, 0.0, 10.0)).unwrap();
    // 8 mm down at Z's 2500 mm/min: 0.192 s.
    path.rapid_to(DVec3::new(100.0, 0.0, 2.0)).unwrap();
    // 5 mm plunged at 300 mm/min: 1 s.
    path.feed_to(DVec3::new(100.0, 0.0, -3.0), MoveRole::Plunge, 300.0)
        .unwrap();
    // 60 mm cut at 1200 mm/min: 3 s.
    path.feed_to(DVec3::new(100.0, 60.0, -3.0), MoveRole::Cut, 1200.0)
        .unwrap();
    // 13 mm retracted at 6000 mm/min, which Z cannot: 0.312 s, as at rapid.
    path.feed_to(DVec3::new(100.0, 60.0, 10.0), MoveRole::Retract, 6000.0)
        .unwrap();
    let drawn = crate::strokes(&path, &Frame::default(), 0.01, 0, &rapid);
    let want = [0.0, 1.2, 1.392, 2.392, 5.392, 5.704];
    assert_eq!(drawn.times.len(), want.len());
    for (got, want) in drawn.times.iter().zip(want) {
        assert!((got - want).abs() < 1e-9, "{:?}", drawn.times);
    }
    assert_eq!(drawn.z, Some([-3.0, 10.0]));
    assert_eq!(drawn.fastest, 6000.0);
    assert_eq!((drawn.cut, drawn.rapid), (78.0, 108.0));
}

/// A setup's machining time is its operations' at its machine's rapid
/// rates, and its tool changes at the machine's time for one; its tools are
/// listed with what each does, and each operation with how it cuts.
#[test]
fn a_setup_is_timed_and_its_tools_listed() {
    let job = JOB.replace("programs = \"operation\"\n", "")
        + r#"
[tools.ball-3]
kind = "ball"
diameter = 3.0
number = 2
rpm = 18000
flutes = 2
stickout = 20.0
holder = { name = "ER11 collet", segments = [{ diameter = 16.0, length = 20.0 }] }

[[setup.operation]]
name = "finish"
strategy = "parallel"
tool = "ball-3"
passes = { stepover = 1.0, feed = 900.0 }
"#;
    let request = |post: Option<String>| Request {
        version: VERSION,
        job: match &post {
            Some(_) => job.replace("post = \"grbl\"", "post = \"Quick\""),
            None => job.clone(),
        },
        shapes: BTreeMap::from([(
            "part".to_string(),
            Shape {
                text: native("hinge_jig.step"),
                placement: IDENTITY,
            },
        )]),
        max_points: 0,
        post,
        read: None,
    };
    let run = answer(&request(None)).unwrap();
    let m = &run.machining;
    // GRBL's own rates, and a minute a change by hand.
    assert_eq!((m.rapid, m.rapid_given), ([5000.0, 5000.0, 2500.0], true));
    assert_eq!(m.tool_change, Some(60.0));
    assert_eq!(m.tool_changes, 2, "{}", run.program.as_ref().unwrap().text);
    let moves: f64 = run.operations.iter().map(|o| o.seconds).sum();
    assert!((m.moves - moves).abs() < 1e-9);
    assert!((m.total() - (moves + 120.0)).abs() < 1e-9);

    // Each tool, in the order it first cuts, and what it does.
    let names: Vec<&str> = run.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["flat-6", "ball-3"]);
    let ball = &run.tools[1];
    assert_eq!((ball.number, ball.kind.as_str()), (2, "ball"));
    assert_eq!((ball.flutes, ball.stickout), (Some(2), Some(20.0)));
    assert_eq!(ball.holder.as_deref(), Some("ER11 collet"));
    assert_eq!(ball.operations, [1]);
    assert_eq!(ball.fastest_rpm, 18000);
    assert!((ball.seconds - run.operations[1].seconds).abs() < 1e-9);

    // Each operation, as it cuts.
    let rough = run.operations[0].cutting.as_ref().unwrap();
    assert_eq!((rough.tool.as_str(), rough.tool_number), ("flat-6", 1));
    assert_eq!((rough.stepdown, rough.stepover), (Some(3.0), Some(3.0)));
    assert_eq!(rough.rpm, 16000);
    let [low, high] = rough.z.unwrap();
    assert!(low < 0.0 && high > 0.0, "{low} {high}");
    assert_eq!(run.tools[0].lowest, Some(low));
    let finish = run.operations[1].cutting.as_ref().unwrap();
    assert_eq!((finish.feed, finish.stepdown), (900.0, None));

    // The part inside its stock, its top at the work zero's height less
    // the stock's margin.
    let layout = run.layout.as_ref().unwrap();
    assert_eq!(layout.post, "GRBL");
    assert!(layout.tilt.is_empty());
    assert_eq!(layout.tool_axis, [0.0, 0.0, 1.0]);
    for i in 0..3 {
        assert!(layout.stock[0][i] <= layout.part[0][i] + 1e-9);
        assert!(layout.part[1][i] <= layout.stock[1][i] + 1e-9);
    }
    assert!((layout.stock[1][2] - layout.part[1][2] - 2.0).abs() < 1e-6);

    // The same on a machine whose rapids are quicker: the cutting takes as
    // long, the rest less, and a machine that does not say how long a
    // change takes leaves the changes out.
    let grbl = include_str!("../../camcraft/crates/camcraft-post/profiles/grbl.toml");
    let quick = grbl
        .replacen("\"GRBL\"", "\"Quick\"", 1)
        .replace("tool_change_seconds = 60\n", "")
        .replace("x = 5000", "x = 50000")
        .replace("y = 5000", "y = 50000")
        .replace("z = 2500", "z = 25000");
    let fast = answer(&request(Some(quick))).unwrap();
    assert_eq!(fast.machining.tool_change, None);
    assert!((fast.machining.total() - fast.machining.moves).abs() < 1e-9);
    assert!(fast.machining.moves < m.moves);
    for (a, b) in run.operations.iter().zip(&fast.operations) {
        assert!(b.seconds < a.seconds, "{}", a.name);
        assert_eq!(a.cut_mm, b.cut_mm);
    }
}

/// The sheet's notes on an operation say its strategy's own settings: a
/// thread's pitch and hand, a bore's helix, rest machining, corner relief
/// and a pattern.
#[test]
fn an_operations_own_settings_are_noted() {
    use camcraft_core::job::{
        Linear, Operation, Relief, ReliefStyle, Rest, StrategySpec, ThreadHand,
    };

    let mut thread = Operation::new("thread", StrategySpec::named("thread").unwrap());
    if let StrategySpec::Thread { pitch, hand, .. } = &mut thread.strategy {
        (*pitch, *hand) = (0.8, ThreadHand::Left);
    }
    assert_eq!(
        crate::strategy_notes(&thread),
        ["thread 0.8 mm pitch, left-hand"]
    );
    let bore = Operation::new("bore", StrategySpec::named("bore").unwrap());
    assert_eq!(crate::strategy_notes(&bore), ["helix 1 mm a turn"]);
    let mut rest = Operation::new("rest", StrategySpec::named("adaptive").unwrap());
    if let StrategySpec::Adaptive { rest: r, .. } = &mut rest.strategy {
        *r = Some(Rest {
            adjustment: 0.2,
            ..Rest::default()
        });
    }
    rest.transform.linear = Some(Linear {
        x_count: 3,
        x_spacing: 20.0,
        y_count: 1,
        y_spacing: 0.0,
    });
    assert_eq!(
        crate::strategy_notes(&rest),
        [
            "rest machining: what the operations before left, past 0.2 mm",
            "3 copies, by its pattern"
        ]
    );
    let mut profile = Operation::new("profile", StrategySpec::named("profile").unwrap());
    profile.passes.relief = Some(Relief {
        style: ReliefStyle::TBoneLong,
        sharper_than: Some(100.0),
    });
    assert_eq!(
        crate::strategy_notes(&profile),
        ["corners relieved with T-bones along the longer wall, sharper than 100°"]
    );
    let plain = Operation::new("plain", StrategySpec::named("parallel").unwrap());
    assert!(crate::strategy_notes(&plain).is_empty());
}
