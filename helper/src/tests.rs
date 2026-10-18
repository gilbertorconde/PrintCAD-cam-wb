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
        job: JOB.into(),
        shapes: BTreeMap::from([(
            "part".to_string(),
            Shape {
                text: native("hinge_jig.step"),
                placement: IDENTITY,
            },
        )]),
        max_points: 5000,
    };
    let run = answer(&request).unwrap();
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
        };
        answer(&request).unwrap().stock
    };
    let mut moved = IDENTITY;
    moved[3] = 100.0;
    let (here, there) = (at(IDENTITY), at(moved));
    assert!((there[0][0] - here[0][0] - 100.0).abs() < 1e-6);
    assert!((there[0][1] - here[0][1]).abs() < 1e-6);
}
