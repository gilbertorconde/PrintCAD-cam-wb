//! An operation's pattern: its toolpath repeated in rows, round a centre,
//! or mirrored. Its numbers, the panel's group of them, and what the
//! group's widgets do.

use camcraft_core::job::{Linear, MOST_COPIES, Mirror, MirrorAxis, Pattern, Polar};
use printcad_bench_sdk::api::*;
use printcad_bench_sdk::serde_json::{self, Value};
use serde::{Deserialize, Serialize};

use crate::data::OperationData;
use crate::panel::{about, choice, fold, number, toggle};

/// A pattern as numbers, each part on or off. A part turned off keeps its
/// numbers, so turning it on again finds them, and a formula always has
/// one to drive. Counts are numbers too, so formulas can drive them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PatternData {
    pub linear: bool,
    pub x_count: f64,
    pub x_spacing: f64,
    pub y_count: f64,
    pub y_spacing: f64,
    pub polar: bool,
    pub polar_count: f64,
    pub polar_angle: f64,
    pub centre_x: f64,
    pub centre_y: f64,
    pub mirror: bool,
    pub mirror_axis: MirrorAxis,
    pub through_x: f64,
    pub through_y: f64,
}

impl Default for PatternData {
    /// Nothing repeated; turned on, three in a row 20 mm apart, four round
    /// the work zero, a mirror about its X.
    fn default() -> Self {
        Self {
            linear: false,
            x_count: 3.0,
            x_spacing: 20.0,
            y_count: 1.0,
            y_spacing: 20.0,
            polar: false,
            polar_count: 4.0,
            polar_angle: 360.0,
            centre_x: 0.0,
            centre_y: 0.0,
            mirror: false,
            mirror_axis: MirrorAxis::X,
            through_x: 0.0,
            through_y: 0.0,
        }
    }
}

/// A count as camcraft takes it: whole, one at least.
fn count(v: f64) -> u32 {
    v.round().clamp(1.0, f64::from(MOST_COPIES)) as u32
}

impl PatternData {
    /// camcraft's pattern as numbers, the parts it has not as they start.
    pub fn of(p: &Pattern) -> Self {
        let mut out = Self::default();
        if let Some(l) = p.linear {
            out.linear = true;
            out.x_count = f64::from(l.x_count);
            out.x_spacing = l.x_spacing;
            out.y_count = f64::from(l.y_count);
            out.y_spacing = l.y_spacing;
        }
        if let Some(r) = p.polar {
            out.polar = true;
            out.polar_count = f64::from(r.count);
            out.polar_angle = r.angle;
            [out.centre_x, out.centre_y] = r.centre;
        }
        if let Some(m) = p.mirror {
            out.mirror = true;
            out.mirror_axis = m.axis;
            [out.through_x, out.through_y] = m.through;
        }
        out
    }

    /// The pattern camcraft cuts with: the parts turned on.
    pub fn pattern(&self) -> Pattern {
        Pattern {
            linear: self.linear.then_some(Linear {
                x_count: count(self.x_count),
                x_spacing: self.x_spacing,
                y_count: count(self.y_count),
                y_spacing: self.y_spacing,
            }),
            polar: self.polar.then_some(Polar {
                count: count(self.polar_count),
                angle: self.polar_angle,
                centre: [self.centre_x, self.centre_y],
            }),
            mirror: self.mirror.then_some(Mirror {
                axis: self.mirror_axis,
                through: [self.through_x, self.through_y],
            }),
        }
    }

    /// The numbers formulas may drive, by pointer into the operation's
    /// data, name and label: those of the parts turned on.
    pub fn parameters(&self) -> Vec<(String, &'static str, &'static str, Dim)> {
        let mut out = Vec::new();
        let parts: [(bool, &[Field]); 3] = [
            (
                self.linear,
                &[
                    ("x_count", "Pattern copies along X", Dim::Number),
                    ("x_spacing", "Pattern spacing along X", Dim::Length),
                    ("y_count", "Pattern copies along Y", Dim::Number),
                    ("y_spacing", "Pattern spacing along Y", Dim::Length),
                ],
            ),
            (
                self.polar,
                &[
                    ("polar_count", "Pattern copies round", Dim::Number),
                    ("polar_angle", "Pattern angle", Dim::Angle),
                    ("centre_x", "Pattern centre X", Dim::Length),
                    ("centre_y", "Pattern centre Y", Dim::Length),
                ],
            ),
            (
                self.mirror,
                &[
                    ("through_x", "Mirror through X", Dim::Length),
                    ("through_y", "Mirror through Y", Dim::Length),
                ],
            ),
        ];
        for (on, fields) in parts {
            if on {
                for (field, label, dim) in fields {
                    out.push((format!("/transform/{field}"), *field, *label, *dim));
                }
            }
        }
        out
    }
}

/// A number of the pattern: its field, label and kind.
type Field = (&'static str, &'static str, Dim);

const AXES: [(MirrorAxis, &str); 2] = [
    (MirrorAxis::X, "About a line along X: Y turned over"),
    (MirrorAxis::Y, "About a line along Y: X turned over"),
];

/// The Pattern group.
pub fn pattern_group(data: &OperationData, feature: &str) -> Widget {
    let p = &data.transform;
    let length = |id: &str, label: &str, v: f64| number(id, label, v, Dim::Length, 2, feature);
    let whole = |id: &str, label: &str, v: f64| number(id, label, v, Dim::Number, 0, feature);
    let toggle = |id: &str, label: &str, on: bool| toggle(format!("transform/{id}"), label, on);
    let mut rows = vec![toggle("linear", "In rows along X and Y", p.linear)];
    if p.linear {
        rows.push(whole("transform/x_count", "Along X, copies", p.x_count));
        rows.push(length("transform/x_spacing", "Along X, apart", p.x_spacing));
        rows.push(whole("transform/y_count", "Along Y, copies", p.y_count));
        rows.push(length("transform/y_spacing", "Along Y, apart", p.y_spacing));
    }
    rows.push(toggle("polar", "Round a centre", p.polar));
    if p.polar {
        rows.push(whole("transform/polar_count", "Copies", p.polar_count));
        rows.push(number(
            "transform/polar_angle",
            "Over, degrees",
            p.polar_angle,
            Dim::Angle,
            1,
            feature,
        ));
        rows.push(length("transform/centre_x", "Centre X", p.centre_x));
        rows.push(length("transform/centre_y", "Centre Y", p.centre_y));
    }
    rows.push(toggle("mirror", "Mirrored", p.mirror));
    if p.mirror {
        rows.push(choice(
            "transform/mirror_axis",
            "Mirror",
            AXES.iter().map(|(_, l)| l.to_string()).collect(),
            AXES.iter()
                .position(|(a, _)| *a == p.mirror_axis)
                .unwrap_or(0),
        ));
        rows.push(length("transform/through_x", "Through X", p.through_x));
        rows.push(length("transform/through_y", "Through Y", p.through_y));
    }
    let copies = p.pattern().copies();
    rows.push(about(if copies > 1 {
        format!(
            "{copies} copies of the toolpath, the original among them, from the work \
                 zero in the setup's axes: the rows, each round the centre, then all \
                 mirrored. Each copy is linked, simulated and checked; a mirrored one's \
                 rings are turned round so it still climbs."
        )
    } else {
        "The toolpath once, where it is.".into()
    }));
    let summary = if copies > 1 {
        format!("{copies} copies")
    } else {
        "Once".into()
    };
    fold("Pattern", &summary, rows)
}

/// A widget of the Pattern group changed: `id` without its `transform/`.
/// Whether it was one of its.
pub fn pattern_event(data: &mut OperationData, id: &str, event: &PanelEvent) -> bool {
    if let ("mirror_axis", PanelEvent::Choice { index, .. }) = (id, event) {
        return match AXES.get(*index) {
            Some((a, _)) => {
                data.transform.mirror_axis = *a;
                true
            }
            None => false,
        };
    }
    let mut value = serde_json::to_value(data.transform).unwrap_or(Value::Null);
    let Some(slot) = value.get_mut(id) else {
        return false;
    };
    match event {
        PanelEvent::Number { value: v, .. } if slot.is_number() => *slot = serde_json::json!(v),
        PanelEvent::Toggle { on, .. } if slot.is_boolean() => *slot = Value::Bool(*on),
        _ => return false,
    }
    match serde_json::from_value::<PatternData>(value) {
        Ok(mut p) => {
            // Counts whole, from one; a spread of more than nothing, a turn
            // at most.
            for c in [&mut p.x_count, &mut p.y_count, &mut p.polar_count] {
                *c = c.round().clamp(1.0, f64::from(MOST_COPIES));
            }
            p.polar_angle = p.polar_angle.clamp(1.0, 360.0);
            data.transform = p;
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use camcraft_core::job::StrategySpec;

    #[test]
    fn a_pattern_reaches_the_operation_and_comes_back() {
        let mut op = OperationData::new(None, StrategySpec::named("pocket").unwrap());
        assert!(!op.operation("").transform.repeats(), "none unless asked");
        let on = |id: &str| PanelEvent::Toggle {
            id: format!("transform/{id}"),
            on: true,
        };
        assert!(pattern_event(&mut op, "linear", &on("linear")));
        assert!(pattern_event(&mut op, "mirror", &on("mirror")));
        let set = |id: &str, value| PanelEvent::Number {
            id: format!("transform/{id}"),
            value,
        };
        assert!(pattern_event(&mut op, "x_spacing", &set("x_spacing", 25.0)));
        assert!(pattern_event(&mut op, "x_count", &set("x_count", 2.6)));
        let y = PanelEvent::Choice {
            id: "transform/mirror_axis".into(),
            index: 1,
        };
        assert!(pattern_event(&mut op, "mirror_axis", &y));
        assert!(!pattern_event(&mut op, "nothing", &set("nothing", 1.0)));
        let cut = op.operation("");
        assert_eq!(
            cut.transform,
            Pattern {
                linear: Some(Linear {
                    x_count: 3,
                    x_spacing: 25.0,
                    y_count: 1,
                    y_spacing: 20.0,
                }),
                polar: None,
                mirror: Some(Mirror {
                    axis: MirrorAxis::Y,
                    through: [0.0, 0.0],
                }),
            }
        );
        assert_eq!(cut.transform.copies(), 6);
        assert_eq!(PatternData::of(&cut.transform), op.transform);
        let names: Vec<_> = op.transform.parameters().into_iter().map(|p| p.1).collect();
        assert!(names.contains(&"x_spacing") && names.contains(&"through_x"));
        assert!(!names.contains(&"polar_count"), "only the parts turned on");
        // The job file keeps it.
        let setup = crate::data::SetupData::new(vec!["b1".into()], "grbl", 0.05);
        let models = crate::data::models_of(&setup, |_| Some("Part".into()));
        let job = crate::data::job("S", &setup, &models, &[("P".to_string(), &op)]);
        let text = camcraft_session::job_text(&job).unwrap();
        let back = camcraft_session::parse_job(&text, "t").unwrap();
        assert_eq!(back.setups[0].operations[0].transform, cut.transform);
        // The document keeps it, and a new strategy keeps it too.
        let back = OperationData::from(&op.to_value()).unwrap();
        assert_eq!(back, op);
        op.set_strategy(StrategySpec::named("profile").unwrap(), None, 0.05);
        assert_eq!(op.operation("").transform, cut.transform);
        // A document from before patterns has none.
        let mut old = OperationData::new(None, StrategySpec::named("pocket").unwrap()).to_value();
        old.as_object_mut().unwrap().remove("transform");
        let old = OperationData::from(&old).unwrap();
        assert!(!old.operation("").transform.repeats());
    }
}
