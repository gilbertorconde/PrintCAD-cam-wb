//! An operation's linking, Fusion's Linking tab: how it moves between cuts, and
//! into and out of them. Its numbers, the panel's group of them, and what the
//! group's widgets do.

use camcraft_core::job::{
    Entry, HelixSize, Leads, Linking, Operation, Reach, Retraction, ToolSpec,
};
use printcad_bench_sdk::api::*;
use printcad_bench_sdk::serde_json::{self, Value};
use serde::{Deserialize, Serialize};

use crate::data::OperationData;
use crate::panel::{choice, group, number};

/// A number that need not be given: the strategy's own when it is not. It keeps
/// its value either way, so a formula always has one to drive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Opt {
    pub value: f64,
    pub given: bool,
}

impl Opt {
    fn of(v: Option<f64>) -> Self {
        Self {
            value: v.unwrap_or(0.0),
            given: v.is_some(),
        }
    }

    fn get(self) -> Option<f64> {
        self.given.then_some(self.value)
    }
}

/// An operation's linking as numbers; the plunge feed is the operation's own
/// (`/plunge`), with its other feeds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LinkingData {
    pub retract_feed: f64,
    pub retraction: Retraction,
    pub safe_distance: f64,
    pub stay_down: Opt,
    pub lead_radius: Opt,
    pub ramp_angle: f64,
    pub ramp_feed: Opt,
    pub ramp_stepdown: Opt,
    pub entry: Entry,
    pub helix_diameter: Opt,
    pub leads: bool,
    pub lead_vertical: Opt,
    pub lead_straight: Opt,
    pub lead_arc: Opt,
    pub lead_sweep: f64,
    pub reorder: bool,
    pub allow_reverse: bool,
}

impl Default for LinkingData {
    fn default() -> Self {
        Self::of(&Linking::default())
    }
}

impl LinkingData {
    /// camcraft's linking as numbers.
    pub fn of(l: &Linking) -> Self {
        let leads = l.leads.unwrap_or_default();
        Self {
            retract_feed: l.retract_feed,
            retraction: l.retraction,
            safe_distance: l.safe_distance,
            stay_down: Opt::of(l.stay_down),
            lead_radius: Opt::of(l.lead_radius),
            ramp_angle: l.ramp_angle,
            ramp_feed: Opt::of(l.ramp_feed),
            ramp_stepdown: Opt::of(l.ramp_stepdown),
            entry: l.entry.unwrap_or_default(),
            helix_diameter: Opt::of(l.helix_diameter),
            leads: l.leads.is_some(),
            lead_vertical: Opt::of(leads.vertical),
            lead_straight: Opt::of(leads.straight),
            lead_arc: Opt::of(leads.radius),
            lead_sweep: leads.sweep,
            reorder: l.reorder,
            allow_reverse: l.allow_reverse,
        }
    }

    /// The numbers written into camcraft's linking, whose plunge feed stays.
    pub fn apply(&self, l: &mut Linking) {
        l.retract_feed = self.retract_feed;
        l.retraction = self.retraction;
        l.safe_distance = self.safe_distance;
        l.stay_down = self.stay_down.get();
        l.lead_radius = self.lead_radius.get();
        l.ramp_angle = self.ramp_angle;
        l.ramp_feed = self.ramp_feed.get();
        l.ramp_stepdown = self.ramp_stepdown.get();
        // The default is written only when changed, as camcraft's app does.
        l.entry = (self.entry != Entry::default()).then_some(self.entry);
        l.helix_diameter = self.helix_diameter.get();
        l.leads = self.leads.then_some(Leads {
            radius: self.lead_arc.get(),
            sweep: self.lead_sweep,
            straight: self.lead_straight.get(),
            vertical: self.lead_vertical.get(),
        });
        l.reorder = self.reorder;
        l.allow_reverse = self.allow_reverse;
    }

    /// The numbers formulas may drive, by pointer into the operation's data,
    /// name and label: an optional one only while it is given.
    pub fn parameters(&self) -> Vec<(String, &'static str, &'static str, Dim)> {
        let mut out = Vec::new();
        for (field, label, dim) in [
            ("retract_feed", "Retract feed", Dim::Number),
            ("safe_distance", "Safe distance", Dim::Length),
            ("ramp_angle", "Ramp angle", Dim::Angle),
            ("lead_sweep", "Lead sweep", Dim::Angle),
        ] {
            out.push((format!("/linking/{field}"), field, label, dim));
        }
        for (field, label, dim, opt) in [
            (
                "stay_down",
                "Stay-down distance",
                Dim::Length,
                self.stay_down,
            ),
            ("lead_radius", "Lead radius", Dim::Length, self.lead_radius),
            ("ramp_feed", "Ramp feed", Dim::Number, self.ramp_feed),
            (
                "ramp_stepdown",
                "Ramp stepdown",
                Dim::Length,
                self.ramp_stepdown,
            ),
            (
                "helix_diameter",
                "Helix diameter",
                Dim::Length,
                self.helix_diameter,
            ),
            (
                "lead_vertical",
                "Vertical lead radius",
                Dim::Length,
                self.lead_vertical,
            ),
            (
                "lead_straight",
                "Lead straight",
                Dim::Length,
                self.lead_straight,
            ),
            ("lead_arc", "Lead arc radius", Dim::Length, self.lead_arc),
        ] {
            if opt.given {
                out.push((format!("/linking/{field}/value"), field, label, dim));
            }
        }
        out
    }
}

/// The tool's diameter and corner, or a 6 mm flat's before a tool is chosen.
fn sizes(tool: Option<&ToolSpec>) -> (f64, f64) {
    tool.map_or((6.0, 0.0), |t| (t.diameter, t.corner))
}

fn mm(v: f64) -> String {
    format!("{v:.2} mm")
}

/// An optional number of the group: unticked, what it is without one; ticked,
/// its field.
#[allow(clippy::too_many_arguments)]
fn optional(
    rows: &mut Vec<Widget>,
    field: &str,
    label: &str,
    absent: &str,
    opt: Opt,
    dim: Dim,
    feature: &str,
) {
    rows.push(Widget::Toggle {
        id: format!("linking/{field}/given"),
        label: if opt.given {
            format!("Set the {}", label.to_lowercase())
        } else {
            format!("{label}: {absent}")
        },
        on: opt.given,
    });
    if opt.given {
        rows.push(number(
            &format!("linking/{field}/value"),
            label,
            opt.value,
            dim,
            2,
            feature,
        ));
    }
}

const RETRACTIONS: [(Retraction, &str); 2] = [
    (Retraction::Minimum, "Minimum, clear of the material"),
    (Retraction::Full, "Full, to the retract height"),
];

/// The entries a strategy offers, as the panel words them.
fn entry_label(e: Entry, beside_lead: bool) -> &'static str {
    match (e, beside_lead) {
        (Entry::Helix, true) => "Helix beside the lead",
        (Entry::Plunge, true) => "Plunge beside the lead",
        (Entry::Helix, false) => "Helix",
        (Entry::Plunge, false) => "Plunge",
        (Entry::Ramp, _) => "Ramp along the ring",
    }
}

/// The Linking group, after camcraft's Linking tab: only what the strategy
/// uses, and for what is not given, what that comes to with `tool`.
pub fn linking_group(data: &OperationData, tool: Option<&ToolSpec>, feature: &str) -> Widget {
    let op = data.operation("");
    let uses = op.strategy.uses();
    let l = &data.linking;
    let (d, corner) = sizes(tool);
    let of_tool = |part: f64, what: &str| match tool {
        Some(_) => format!("{what}, {}", mm(part * d)),
        None => what.to_string(),
    };
    let mut rows = vec![number(
        "linking/retract_feed",
        "Retract feed, mm/min",
        l.retract_feed,
        Dim::Number,
        0,
        feature,
    )];
    if uses.stay_down {
        rows.push(choice(
            "linking/retraction",
            "Retraction",
            RETRACTIONS.iter().map(|(_, s)| s.to_string()).collect(),
            RETRACTIONS
                .iter()
                .position(|(r, _)| *r == l.retraction)
                .unwrap_or(0),
        ));
    }
    rows.push(number(
        "linking/safe_distance",
        "Safe distance",
        l.safe_distance,
        Dim::Length,
        2,
        feature,
    ));
    if uses.stay_down {
        let absent = match uses.reach {
            Reach::Mm(v) => format!("up to {}", mm(f64::from(v))),
            Reach::Tools(k) => of_tool(f64::from(k), &format!("up to {k} × the tool")),
        };
        optional(
            &mut rows,
            "stay_down",
            "Stay-down distance",
            &absent,
            l.stay_down,
            Dim::Length,
            feature,
        );
    }
    if uses.lead_radius {
        optional(
            &mut rows,
            "lead_radius",
            "Lead radius",
            &of_tool(0.1, "a tenth of the tool"),
            l.lead_radius,
            Dim::Length,
            feature,
        );
    }
    if uses.ramp {
        rows.push(number(
            "linking/ramp_angle",
            "Ramp angle",
            l.ramp_angle,
            Dim::Angle,
            1,
            feature,
        ));
        optional(
            &mut rows,
            "ramp_feed",
            "Ramp feed",
            &format!("60% of cutting, {:.0} mm/min", op.ramp_feed()),
            l.ramp_feed,
            Dim::Number,
            feature,
        );
        let absent = match op.stepdown() {
            Some(s) => format!("the stepdown, {}", mm(s)),
            None => of_tool(1.0, "the tool's diameter"),
        };
        optional(
            &mut rows,
            "ramp_stepdown",
            "Most down per lap",
            &absent,
            l.ramp_stepdown,
            Dim::Length,
            feature,
        );
    }
    if !uses.entries.is_empty() {
        rows.push(choice(
            "linking/entry",
            "Entry",
            uses.entries
                .iter()
                .map(|e| entry_label(*e, uses.leads).to_string())
                .collect(),
            uses.entries.iter().position(|e| *e == l.entry).unwrap_or(0),
        ));
        if l.entry == Entry::Helix {
            let absent = match (uses.helix, tool) {
                (HelixSize::Tool, _) => of_tool(0.9, "0.9 × the tool"),
                (HelixSize::Flat, Some(_)) => format!(
                    "0.95 × the tool less its corners, {}",
                    mm(op.helix_diameter(d, corner))
                ),
                (HelixSize::Flat, None) => "0.95 × the tool less its corners".into(),
            };
            optional(
                &mut rows,
                "helix_diameter",
                "Helix diameter",
                &absent,
                l.helix_diameter,
                Dim::Length,
                feature,
            );
        }
    }
    // A pocket's leads take the tool onto its finishing passes and off.
    let pocket_leads = uses.finishing && op.passes.finishing.is_some();
    if uses.leads || pocket_leads {
        rows.push(Widget::Toggle {
            id: "linking/leads".into(),
            label: if uses.leads {
                "Leads on and off each ring".into()
            } else {
                "Leads onto the finishing passes and off".into()
            },
            on: l.leads,
        });
        if l.leads {
            let tenth = of_tool(0.1, "a tenth of the tool");
            // A pocket's finishing passes are entered level, beside the wall:
            // their leads have no vertical arc.
            if uses.leads {
                optional(
                    &mut rows,
                    "lead_vertical",
                    "Vertical lead radius",
                    &tenth,
                    l.lead_vertical,
                    Dim::Length,
                    feature,
                );
            }
            optional(
                &mut rows,
                "lead_straight",
                "Lead straight",
                &tenth,
                l.lead_straight,
                Dim::Length,
                feature,
            );
            optional(
                &mut rows,
                "lead_arc",
                "Lead arc radius",
                &tenth,
                l.lead_arc,
                Dim::Length,
                feature,
            );
            rows.push(number(
                "linking/lead_sweep",
                "Lead arc sweep",
                l.lead_sweep,
                Dim::Angle,
                0,
                feature,
            ));
        }
    }
    if uses.reorder {
        rows.push(Widget::Toggle {
            id: "linking/reorder".into(),
            label: "Reorder to shorten travel".into(),
            on: l.reorder,
        });
        rows.push(Widget::Toggle {
            id: "linking/allow_reverse".into(),
            label: "Reverse passes while reordering (changes climb)".into(),
            on: l.allow_reverse,
        });
    }
    group("Linking", rows)
}

/// Where an optional number starts when it is first given: what it was without
/// one, so ticking it changes nothing until the number does.
fn start(field: &str, op: &Operation, tool: Option<&ToolSpec>) -> Option<f64> {
    let (d, corner) = sizes(tool);
    Some(match field {
        "stay_down" => op.stay_down(d),
        "lead_radius" => op.lead_radius(d),
        "ramp_feed" => op.ramp_feed(),
        "ramp_stepdown" => op.ramp_stepdown(d),
        "helix_diameter" => op.helix_diameter(d, corner),
        "lead_vertical" | "lead_straight" | "lead_arc" => 0.1 * d,
        _ => return None,
    })
}

/// A widget of the Linking group changed: `id` without its `linking/`. Whether
/// it was one of the group's.
pub fn linking_event(
    data: &mut OperationData,
    id: &str,
    event: &PanelEvent,
    tool: Option<&ToolSpec>,
) -> bool {
    let mut value = serde_json::to_value(data.linking).unwrap_or(Value::Null);
    let pointer = format!("/{id}");
    match event {
        PanelEvent::Number { value: v, .. } => {
            let Some(slot) = value.pointer_mut(&pointer).filter(|s| s.is_number()) else {
                return false;
            };
            *slot = serde_json::json!(v);
        }
        PanelEvent::Toggle { on, .. } => {
            if let Some(field) = id.strip_suffix("/given") {
                let op = data.operation("");
                if *on && let Some(s) = start(field, &op, tool) {
                    value[field]["value"] = serde_json::json!(s);
                }
            }
            let Some(slot) = value.pointer_mut(&pointer).filter(|s| s.is_boolean()) else {
                return false;
            };
            *slot = Value::Bool(*on);
        }
        PanelEvent::Choice { index, .. } => match id {
            "retraction" => {
                let Some((r, _)) = RETRACTIONS.get(*index) else {
                    return false;
                };
                value["retraction"] = serde_json::json!(r);
            }
            "entry" => {
                let entries = data.strategy().uses().entries;
                let Some(e) = entries.get(*index) else {
                    return false;
                };
                value["entry"] = serde_json::json!(e);
            }
            _ => return false,
        },
        _ => return false,
    }
    match serde_json::from_value(value) {
        Ok(l) => {
            data.linking = l;
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use camcraft_core::job::StrategySpec;

    use super::*;

    fn flat(diameter: f64) -> ToolSpec {
        ToolSpec {
            diameter,
            ..ToolSpec::default()
        }
    }

    #[test]
    fn camcrafts_linking_reads_back_from_the_numbers() {
        let linking = Linking {
            retract_feed: 3000.0,
            retraction: Retraction::Full,
            safe_distance: 2.5,
            stay_down: Some(12.0),
            ramp_angle: 3.0,
            ramp_feed: None,
            entry: Some(Entry::Plunge),
            leads: Some(Leads {
                radius: Some(1.5),
                sweep: 45.0,
                straight: None,
                vertical: Some(0.0),
            }),
            reorder: false,
            allow_reverse: true,
            ..Linking::default()
        };
        let mut back = Linking::default();
        LinkingData::of(&linking).apply(&mut back);
        assert_eq!(back, linking);
        LinkingData::default().apply(&mut back);
        assert_eq!(back, Linking::default());
    }

    #[test]
    fn a_number_ticked_starts_as_what_it_was() {
        let tool = flat(6.0);
        let mut op = OperationData::new(None, StrategySpec::named("adaptive").unwrap());
        op.use_tool("Flat 6", &tool, None, 0.05);
        let before = op.operation("").stay_down(6.0);
        let tick = PanelEvent::Toggle {
            id: "linking/stay_down/given".into(),
            on: true,
        };
        assert!(linking_event(
            &mut op,
            "stay_down/given",
            &tick,
            Some(&tool)
        ));
        assert!(op.linking.stay_down.given);
        assert_eq!(op.operation("").linking.stay_down, Some(before));
        let set = PanelEvent::Number {
            id: "linking/stay_down/value".into(),
            value: 4.0,
        };
        assert!(linking_event(&mut op, "stay_down/value", &set, Some(&tool)));
        assert_eq!(op.operation("").linking.stay_down, Some(4.0));
        // Not the group's, or not a number where it points.
        assert!(!linking_event(&mut op, "nothing", &set, Some(&tool)));
        assert!(!linking_event(&mut op, "stay_down", &set, Some(&tool)));
    }

    #[test]
    fn the_linking_reaches_the_job_file() {
        let mut op = OperationData::new(None, StrategySpec::named("profile").unwrap());
        op.linking.leads = false;
        op.linking.entry = Entry::Plunge;
        op.linking.retraction = Retraction::Full;
        let choose = PanelEvent::Choice {
            id: "linking/retraction".into(),
            index: 0,
        };
        assert!(linking_event(&mut op, "retraction", &choose, None));
        assert_eq!(op.linking.retraction, Retraction::Minimum);
        let cut = op.operation("Profile");
        assert_eq!(cut.linking.leads, None);
        assert_eq!(cut.entry(), Some(Entry::Plunge));
        let back = OperationData::from(&op.to_value()).unwrap();
        assert_eq!(back, op);
    }

    #[test]
    fn an_operation_from_before_linking_cuts_with_camcrafts() {
        let op = OperationData::new(None, StrategySpec::named("pocket").unwrap());
        let mut value = op.to_value();
        value.as_object_mut().unwrap().remove("linking");
        let old = OperationData::from(&value).unwrap();
        let mut expected = Operation::new("", StrategySpec::named("pocket").unwrap()).linking;
        expected.plunge_feed = op.plunge;
        assert_eq!(old.operation("").linking, expected);
    }
}
