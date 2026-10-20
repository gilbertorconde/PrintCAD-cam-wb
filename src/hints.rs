//! Help on hover, as camcraft's app gives it: for the fields a drawing says
//! best, a sentence and a small sketch with the measure the field sets
//! picked out (printCAD's figure hints).

use printcad_bench_sdk::api::*;

/// The sketches' size, in their own units, y up.
const W: f32 = 220.0;
const H: f32 = 120.0;

fn path(points: &[[f32; 2]], closed: bool, stroke: DiagramStroke) -> DiagramShape {
    DiagramShape::Path {
        points: points.to_vec(),
        closed,
        stroke,
        fill: false,
    }
}

fn text(at: [f32; 2], text: &str) -> DiagramShape {
    DiagramShape::Text {
        at,
        text: text.into(),
        mono: false,
    }
}

fn measure(from: [f32; 2], to: [f32; 2], offset: f32, text: &str) -> Dimension {
    Dimension {
        from,
        to,
        offset,
        text: text.into(),
        emphasis: true,
    }
}

fn figure(shapes: Vec<DiagramShape>, dimensions: Vec<Dimension>) -> Widget {
    Widget::Diagram {
        id: String::new(),
        width: W,
        height: H,
        shapes,
        dimensions,
        callouts: Vec::new(),
    }
}

/// A flat end mill seen from the side, its tip's centre at `x`, its tip at
/// `y`, `r` across half.
fn cutter(x: f32, y: f32, r: f32) -> DiagramShape {
    path(
        &[[x - r, y], [x + r, y], [x + r, y + 60.0], [x - r, y + 60.0]],
        true,
        DiagramStroke::Outline,
    )
}

/// Two passes side by side, their stepover between their centres.
fn stepover() -> Widget {
    let (y, r) = (40.0, 18.0);
    figure(
        vec![
            path(&[[10.0, y], [210.0, y]], false, DiagramStroke::Thin),
            cutter(80.0, y, r),
            cutter(140.0, y, r),
            path(&[[80.0, 10.0], [80.0, 115.0]], false, DiagramStroke::Axis),
            path(&[[140.0, 10.0], [140.0, 115.0]], false, DiagramStroke::Axis),
        ],
        vec![measure([80.0, 20.0], [140.0, 20.0], 0.0, "stepover")],
    )
}

/// Levels cut down a block, the stepdown between two.
fn stepdown() -> Widget {
    figure(
        vec![
            path(
                &[[30.0, 10.0], [190.0, 10.0], [190.0, 100.0], [30.0, 100.0]],
                true,
                DiagramStroke::Outline,
            ),
            path(&[[30.0, 75.0], [190.0, 75.0]], false, DiagramStroke::Hidden),
            path(&[[30.0, 50.0], [190.0, 50.0]], false, DiagramStroke::Hidden),
            path(&[[30.0, 25.0], [190.0, 25.0]], false, DiagramStroke::Hidden),
        ],
        vec![measure([190.0, 100.0], [190.0, 75.0], -14.0, "stepdown")],
    )
}

/// Material left on a wall and a floor, the one asked about picked out.
fn stock_to_leave(walls: bool) -> Widget {
    let part = [[40.0, 15.0], [120.0, 15.0], [120.0, 70.0], [190.0, 70.0]];
    let left = [[40.0, 27.0], [108.0, 27.0], [108.0, 82.0], [190.0, 82.0]];
    let dimension = if walls {
        measure([108.0, 50.0], [120.0, 50.0], 0.0, "walls")
    } else {
        measure([70.0, 15.0], [70.0, 27.0], 0.0, "floors")
    };
    figure(
        vec![
            path(&part, false, DiagramStroke::Outline),
            path(&left, false, DiagramStroke::Hidden),
            text([160.0, 100.0], "material left"),
        ],
        vec![dimension],
    )
}

/// The heights through the stock, the one asked about picked out.
fn heights(which: &str) -> Widget {
    let stock = [[50.0, 10.0], [190.0, 10.0], [190.0, 60.0], [50.0, 60.0]];
    let part = [[70.0, 10.0], [170.0, 10.0], [170.0, 45.0], [70.0, 45.0]];
    let level = |y: f32, name: &str| {
        let stroke = if name == which {
            DiagramStroke::Accent
        } else {
            DiagramStroke::Thin
        };
        vec![
            path(&[[40.0, y], [200.0, y]], false, stroke),
            text([22.0, y], name),
        ]
    };
    let mut shapes = vec![
        path(&stock, true, DiagramStroke::Hidden),
        path(&part, true, DiagramStroke::Outline),
    ];
    for (y, name) in [
        (110.0, "clearance"),
        (85.0, "retract"),
        (60.0, "top"),
        (20.0, "bottom"),
    ] {
        shapes.extend(level(y, name));
    }
    figure(shapes, Vec::new())
}

/// What a field's hover says and shows, by its id; `None` for one with no
/// sketch.
fn hint(id: &str) -> Option<(&'static str, Widget)> {
    Some(match id {
        "stepover" => (
            "How far apart side by side the passes are, centre to centre.",
            stepover(),
        ),
        "stepdown" => (
            "The deepest each level cuts below the one before.",
            stepdown(),
        ),
        "radial" => (
            "Material left on the walls, for a finishing pass to take.",
            stock_to_leave(true),
        ),
        "axial" => (
            "Material left on the floors, for a finishing pass to take.",
            stock_to_leave(false),
        ),
        "heights/clearance" => (
            "Where the tool arrives at the operation and leaves it.",
            heights("clearance"),
        ),
        "heights/retract" => (
            "How high the tool lifts between cuts with a full retraction.",
            heights("retract"),
        ),
        "heights/top" => ("The highest level it cuts at.", heights("top")),
        "heights/bottom" => ("The lowest level it cuts down to.", heights("bottom")),
        _ => return None,
    })
}

/// `widget` with its hover help and sketch, where its field has one.
pub fn with_hint(widget: Widget) -> Widget {
    let id = match &widget {
        Widget::Number { id, .. } => id.clone(),
        _ => return widget,
    };
    match hint(&id) {
        Some((words, figure)) => Widget::HintedFigure {
            hint: words.into(),
            figure: Box::new(figure),
            widget: Box::new(widget),
        },
        None => widget,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fields_a_sketch_says_best_have_one() {
        for id in [
            "stepover",
            "stepdown",
            "radial",
            "axial",
            "heights/clearance",
            "heights/retract",
            "heights/top",
            "heights/bottom",
        ] {
            let (words, figure) = hint(id).unwrap_or_else(|| panic!("{id}"));
            assert!(words.len() <= 160, "{id}: camcraft keeps a hint short");
            let Widget::Diagram { shapes, .. } = figure else {
                panic!("{id}");
            };
            // Every point inside the sketch.
            for shape in shapes {
                if let DiagramShape::Path { points, .. } = shape {
                    assert!(
                        points
                            .iter()
                            .all(|p| (0.0..=W).contains(&p[0]) && (0.0..=H).contains(&p[1])),
                        "{id}"
                    );
                }
            }
        }
        let plain = Widget::Separator;
        assert_eq!(with_hint(plain.clone()), plain);
    }
}
