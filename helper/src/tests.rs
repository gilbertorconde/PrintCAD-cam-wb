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
    assert!(run.material.is_some());
    assert!(run.stock[1][2] > run.stock[0][2]);
    // Its boundary, the part's silhouette, drawn with where the tool may go
    // and what it cuts outside the part, at the stock's top.
    let drawn = op.outlines.as_ref().expect("a 3D operation's outlines");
    assert!(!drawn.boundary.is_empty() && !drawn.limit.is_empty());
    assert!(!drawn.cut.is_empty());
    let ring = &drawn.boundary[0];
    assert_eq!(ring.first(), ring.last(), "a ring closes");
    assert!((f64::from(ring[0][2]) - run.stock[1][2]).abs() < 1e-3);
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
    let top = run.stock[1][2];
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
    assert!(f64::from(corners[2][0]) > run.stock[1][0]);

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

/// A setup posts with a profile of the user's that the request carries.
#[test]
fn a_post_of_ones_own_writes_the_program() {
    let grbl = include_str!("../../camcraft/crates/camcraft-post/profiles/grbl.toml");
    let mine = grbl.replacen("\"GRBL\"", "\"My mill\"", 1).replacen(
        "extension = \"nc\"",
        "extension = \"mine\"",
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
