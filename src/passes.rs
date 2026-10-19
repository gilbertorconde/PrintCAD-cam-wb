//! The rest of an operation's Passes tab: the numbers and choices its strategy
//! has of its own, the deepest step a cut takes, the tabs a profile leaves, and
//! a 2D cut's finishing passes. Their numbers, the panel's rows of them, and
//! what the rows' widgets do.

use camcraft_core::job::{
    ContourOrder, FaceDirection, Finishing, Operation, PassOrder, RadialDirection, StrategySpec,
    TabShape, Tabs, ToolSpec,
};
use printcad_bench_sdk::api::*;
use printcad_bench_sdk::serde_json::{self, Value};
use serde::{Deserialize, Serialize};

use crate::data::OperationData;
use crate::linking::{Opt, optional_row};
use crate::panel::{choice, group, number};

/// An operation's own numbers of its Passes tab, beyond the stepover, depths,
/// feeds and stock to leave. A number its strategy has not keeps camcraft's
/// default, which it starts from if the strategy changes to one that has.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PassesData {
    /// The deepest a cut steps down without counting as a plunge.
    pub max_cut_step: f64,
    /// The range of slopes a 3D finish keeps to, in degrees from level.
    pub slope_from: f64,
    pub slope_to: f64,
    /// A spiral's or a radial's centre, from the work zero; the boundary's
    /// middle unless given.
    pub centre: bool,
    pub centre_x: f64,
    pub centre_y: f64,
    pub inner_radius: f64,
    /// The outer radius; all of the boundary unless given.
    pub outer_radius: Opt,
    /// A radial's spokes, in degrees from X.
    pub angle_from: f64,
    pub angle_to: f64,
    /// Pocket clearing's shortest ring.
    pub min_ring_length: f64,
    /// How far facing passes run past the stock.
    pub pass_extension: f64,
    /// A drill's peck depth and its dwell at the bottom, in seconds.
    pub peck: Opt,
    pub dwell: Opt,
    pub tabs: bool,
    pub tab_shape: TabShape,
    pub tab_width: f64,
    pub tab_height: f64,
    pub tab_spacing: f64,
    pub finishing: bool,
    pub finishing_passes: f64,
    pub finishing_stepover: Opt,
    pub finishing_feed: Opt,
    pub finishing_repeat: bool,
}

impl Default for PassesData {
    /// camcraft's: each of the strategies that has a number gives its default.
    fn default() -> Self {
        let tabs = Tabs::default();
        let finishing = Finishing::default();
        let mut out = Self {
            max_cut_step: Operation::new("", named("adaptive")).passes.max_cut_step,
            slope_from: 0.0,
            slope_to: 90.0,
            centre: false,
            centre_x: 0.0,
            centre_y: 0.0,
            inner_radius: 0.0,
            outer_radius: Opt::default(),
            angle_from: 0.0,
            angle_to: 360.0,
            min_ring_length: 0.0,
            pass_extension: 0.0,
            peck: Opt::default(),
            dwell: Opt::default(),
            tabs: false,
            tab_shape: tabs.shape,
            tab_width: tabs.width,
            tab_height: tabs.height,
            tab_spacing: tabs.spacing,
            finishing: false,
            finishing_passes: f64::from(finishing.passes.unwrap_or(1)),
            finishing_stepover: Opt::of(finishing.stepover),
            finishing_feed: Opt::of(finishing.feed),
            finishing_repeat: finishing.repeat,
        };
        for name in ["radial", "pocket-clearing", "face", "drill"] {
            out.read_strategy(&named(name));
        }
        out
    }
}

fn named(name: &str) -> StrategySpec {
    StrategySpec::named(name).unwrap_or(StrategySpec::Parallel { angle: 0.0 })
}

impl PassesData {
    /// camcraft's operation as numbers: what its strategy has, and the rest
    /// camcraft's defaults.
    pub fn of(op: &Operation) -> Self {
        let mut out = Self::default();
        out.read_strategy(&op.strategy);
        let p = &op.passes;
        out.max_cut_step = p.max_cut_step;
        if let Some(t) = p.tabs {
            out.tabs = true;
            out.tab_shape = t.shape;
            out.tab_width = t.width;
            out.tab_height = t.height;
            out.tab_spacing = t.spacing;
        }
        if let Some(f) = p.finishing {
            out.finishing = true;
            out.finishing_passes = f64::from(f.passes.unwrap_or(1));
            out.finishing_stepover = Opt::of(f.stepover);
            out.finishing_feed = Opt::of(f.feed);
            out.finishing_repeat = f.repeat;
        }
        out
    }

    /// The numbers `s` has, over these.
    fn read_strategy(&mut self, s: &StrategySpec) {
        match *s {
            StrategySpec::Scallop {
                slope_from,
                slope_to,
                ..
            }
            | StrategySpec::Contour {
                slope_from,
                slope_to,
                ..
            }
            | StrategySpec::MorphedSpiral {
                slope_from,
                slope_to,
                ..
            } => {
                self.slope_from = slope_from;
                self.slope_to = slope_to;
            }
            StrategySpec::Spiral {
                centre,
                inner_radius,
                outer_radius,
                slope_from,
                slope_to,
                ..
            } => {
                self.read_centre(centre, inner_radius, outer_radius);
                self.slope_from = slope_from;
                self.slope_to = slope_to;
            }
            StrategySpec::Radial {
                centre,
                inner_radius,
                outer_radius,
                angle_from,
                angle_to,
                slope_from,
                slope_to,
                ..
            } => {
                self.read_centre(centre, inner_radius, outer_radius);
                self.angle_from = angle_from;
                self.angle_to = angle_to;
                self.slope_from = slope_from;
                self.slope_to = slope_to;
            }
            StrategySpec::PocketClearing {
                min_ring_length, ..
            } => self.min_ring_length = min_ring_length,
            StrategySpec::Face { pass_extension, .. } => self.pass_extension = pass_extension,
            StrategySpec::Drill { peck, dwell } => {
                self.peck = Opt::of(peck);
                self.dwell = Opt::of(dwell);
            }
            _ => {}
        }
        if let StrategySpec::MorphedSpiral { centre, .. } = *s {
            self.centre = centre.is_some();
            if let Some([x, y]) = centre {
                (self.centre_x, self.centre_y) = (x, y);
            }
        }
    }

    fn read_centre(&mut self, centre: Option<[f64; 2]>, inner: f64, outer: f64) {
        self.centre = centre.is_some();
        if let Some([x, y]) = centre {
            (self.centre_x, self.centre_y) = (x, y);
        }
        self.inner_radius = inner;
        // Zero is none given.
        self.outer_radius = Opt::of((outer > 0.0).then_some(outer));
    }

    fn centre(&self) -> Option<[f64; 2]> {
        self.centre.then_some([self.centre_x, self.centre_y])
    }

    /// The numbers written into camcraft's operation, where its strategy has
    /// them.
    pub fn apply(&self, op: &mut Operation) {
        let centre = self.centre();
        let outer = self.outer_radius.get().unwrap_or(0.0);
        match &mut op.strategy {
            StrategySpec::Scallop {
                slope_from,
                slope_to,
                ..
            }
            | StrategySpec::Contour {
                slope_from,
                slope_to,
                ..
            } => {
                (*slope_from, *slope_to) = (self.slope_from, self.slope_to);
            }
            StrategySpec::MorphedSpiral {
                centre: c,
                slope_from,
                slope_to,
                ..
            } => {
                *c = centre;
                (*slope_from, *slope_to) = (self.slope_from, self.slope_to);
            }
            StrategySpec::Spiral {
                centre: c,
                inner_radius,
                outer_radius,
                slope_from,
                slope_to,
                ..
            } => {
                *c = centre;
                (*inner_radius, *outer_radius) = (self.inner_radius, outer);
                (*slope_from, *slope_to) = (self.slope_from, self.slope_to);
            }
            StrategySpec::Radial {
                centre: c,
                inner_radius,
                outer_radius,
                angle_from,
                angle_to,
                slope_from,
                slope_to,
                ..
            } => {
                *c = centre;
                (*inner_radius, *outer_radius) = (self.inner_radius, outer);
                (*angle_from, *angle_to) = (self.angle_from, self.angle_to);
                (*slope_from, *slope_to) = (self.slope_from, self.slope_to);
            }
            StrategySpec::PocketClearing {
                min_ring_length, ..
            } => *min_ring_length = self.min_ring_length,
            StrategySpec::Face { pass_extension, .. } => *pass_extension = self.pass_extension,
            StrategySpec::Drill { peck, dwell } => {
                (*peck, *dwell) = (self.peck.get(), self.dwell.get());
            }
            _ => {}
        }
        let uses = op.strategy.uses();
        let p = &mut op.passes;
        p.max_cut_step = self.max_cut_step;
        p.tabs = (uses.tabs && self.tabs).then_some(Tabs {
            shape: self.tab_shape,
            width: self.tab_width,
            height: self.tab_height,
            spacing: self.tab_spacing,
        });
        let passes = self.finishing_passes.round().clamp(1.0, 20.0) as u32;
        p.finishing = (uses.finishing && self.finishing).then_some(Finishing {
            passes: (passes != 1).then_some(passes),
            stepover: self.finishing_stepover.get(),
            feed: self.finishing_feed.get(),
            repeat: self.finishing_repeat,
        });
    }

    /// The numbers formulas may drive, by pointer into the operation's data,
    /// name and label: those its strategy has, an optional one only while it is
    /// given.
    pub fn parameters(&self, op: &Operation) -> Vec<(String, &'static str, &'static str, Dim)> {
        let uses = op.strategy.uses();
        let s = &op.strategy;
        let slopes = matches!(
            s,
            StrategySpec::Scallop { .. }
                | StrategySpec::Contour { .. }
                | StrategySpec::Spiral { .. }
                | StrategySpec::Radial { .. }
                | StrategySpec::MorphedSpiral { .. }
        );
        let centred = matches!(s, StrategySpec::Spiral { .. } | StrategySpec::Radial { .. });
        let centre = centred || matches!(s, StrategySpec::MorphedSpiral { .. });
        let radial = matches!(s, StrategySpec::Radial { .. });
        let drill = matches!(s, StrategySpec::Drill { .. });
        let mut out = Vec::new();
        let mut add = |on: bool, field: &'static str, label, dim| {
            if on {
                out.push((format!("/passes/{field}"), field, label, dim));
            }
        };
        add(true, "max_cut_step", "Deepest step", Dim::Length);
        add(slopes, "slope_from", "Slopes from", Dim::Angle);
        add(slopes, "slope_to", "Slopes to", Dim::Angle);
        add(centre && self.centre, "centre_x", "Centre X", Dim::Length);
        add(centre && self.centre, "centre_y", "Centre Y", Dim::Length);
        add(centred, "inner_radius", "Inner radius", Dim::Length);
        add(radial, "angle_from", "Spokes from", Dim::Angle);
        add(radial, "angle_to", "Spokes to", Dim::Angle);
        add(
            matches!(s, StrategySpec::PocketClearing { .. }),
            "min_ring_length",
            "Shortest ring",
            Dim::Length,
        );
        add(
            matches!(s, StrategySpec::Face { .. }),
            "pass_extension",
            "Pass extension",
            Dim::Length,
        );
        let tabs = uses.tabs && self.tabs;
        add(tabs, "tab_width", "Tab width", Dim::Length);
        add(tabs, "tab_height", "Tab height", Dim::Length);
        add(tabs, "tab_spacing", "Tab spacing", Dim::Length);
        let finishing = uses.finishing && self.finishing;
        add(
            finishing,
            "finishing_passes",
            "Finishing passes",
            Dim::Number,
        );
        for (on, field, label, dim) in [
            (
                centred && self.outer_radius.given,
                "outer_radius",
                "Outer radius",
                Dim::Length,
            ),
            (drill && self.peck.given, "peck", "Peck", Dim::Length),
            (drill && self.dwell.given, "dwell", "Dwell, s", Dim::Number),
            (
                finishing && self.finishing_stepover.given,
                "finishing_stepover",
                "Finishing stepover",
                Dim::Length,
            ),
            (
                finishing && self.finishing_feed.given,
                "finishing_feed",
                "Finishing feed",
                Dim::Number,
            ),
        ] {
            if on {
                out.push((format!("/passes/{field}/value"), field, label, dim));
            }
        }
        out
    }
}

const ORDERS: [(PassOrder, &str); 2] = [
    (PassOrder::OutsideIn, "Outside in"),
    (PassOrder::InsideOut, "Inside out"),
];
const CONTOUR_ORDERS: [(ContourOrder, &str); 2] = [
    (ContourOrder::ByDepth, "By depth"),
    (ContourOrder::ByArea, "By area"),
];
const RADIAL_DIRECTIONS: [(RadialDirection, &str); 3] = [
    (RadialDirection::BothWays, "Both ways"),
    (RadialDirection::Outward, "Outward"),
    (RadialDirection::Inward, "Inward"),
];
const FACE_DIRECTIONS: [(FaceDirection, &str); 3] = [
    (FaceDirection::BothWays, "Both ways"),
    (FaceDirection::Climb, "Climb"),
    (FaceDirection::Conventional, "Conventional"),
];
const TAB_SHAPES: [(TabShape, &str); 2] = [
    (TabShape::Rectangular, "Rectangular"),
    (TabShape::Triangular, "Triangular: ramped, no plunge"),
];

fn pick<T: PartialEq + Copy>(id: &str, label: &str, options: &[(T, &str)], now: T) -> Widget {
    choice(
        id,
        label,
        options.iter().map(|(_, l)| l.to_string()).collect(),
        options.iter().position(|(x, _)| *x == now).unwrap_or(0),
    )
}

fn order_mut(s: &mut StrategySpec) -> Option<&mut PassOrder> {
    match s {
        StrategySpec::Scallop { order, .. }
        | StrategySpec::Flat { order, .. }
        | StrategySpec::Spiral { order, .. }
        | StrategySpec::MorphedSpiral { order, .. } => Some(order),
        _ => None,
    }
}

fn flat_areas_mut(s: &mut StrategySpec) -> Option<&mut bool> {
    match s {
        StrategySpec::PocketClearing { flat_areas, .. }
        | StrategySpec::Adaptive { flat_areas, .. } => Some(flat_areas),
        _ => None,
    }
}

/// The strategy's own rows of the Passes group, and the deepest step.
pub fn strategy_rows(data: &OperationData, feature: &str) -> Vec<Widget> {
    let p = &data.passes;
    let mut s = data.operation.strategy;
    let mut rows = Vec::new();
    let length = |id: &str, label: &str, v: f64| number(id, label, v, Dim::Length, 2, feature);
    let angle = |id: &str, label: &str, v: f64| number(id, label, v, Dim::Angle, 1, feature);
    if let Some(o) = order_mut(&mut s) {
        rows.push(pick("passes/order", "Order", &ORDERS, *o));
    }
    if let StrategySpec::Contour { order, .. } = s {
        rows.push(pick(
            "passes/contour_order",
            "Order",
            &CONTOUR_ORDERS,
            order,
        ));
    }
    if let StrategySpec::Radial { direction, .. } = s {
        rows.push(pick(
            "passes/radial_direction",
            "Direction",
            &RADIAL_DIRECTIONS,
            direction,
        ));
    }
    if let StrategySpec::Face { direction, .. } = s {
        rows.push(pick(
            "passes/face_direction",
            "Direction",
            &FACE_DIRECTIONS,
            direction,
        ));
        rows.push(length(
            "passes/pass_extension",
            "Pass extension",
            p.pass_extension,
        ));
    }
    if let Some(f) = flat_areas_mut(&mut s) {
        rows.push(Widget::Toggle {
            id: "passes/flat_areas".into(),
            label: "A level on each flat face".into(),
            on: *f,
        });
    }
    if let StrategySpec::PocketClearing { .. } = s {
        rows.push(length(
            "passes/min_ring_length",
            "Shortest ring",
            p.min_ring_length,
        ));
    }
    let centred = matches!(s, StrategySpec::Spiral { .. } | StrategySpec::Radial { .. });
    if centred || matches!(s, StrategySpec::MorphedSpiral { .. }) {
        let absent = if centred {
            "the boundary's middle"
        } else {
            "the point furthest inside"
        };
        rows.push(Widget::Toggle {
            id: "passes/centre".into(),
            label: if p.centre {
                "Set the centre".into()
            } else {
                format!("Centre: {absent}")
            },
            on: p.centre,
        });
        if p.centre {
            rows.push(length("passes/centre_x", "Centre X", p.centre_x));
            rows.push(length("passes/centre_y", "Centre Y", p.centre_y));
        }
    }
    if centred {
        rows.push(length(
            "passes/inner_radius",
            "Inner radius",
            p.inner_radius,
        ));
        optional_row(
            &mut rows,
            "passes",
            "outer_radius",
            "Outer radius",
            "all of the boundary",
            p.outer_radius,
            Dim::Length,
            feature,
        );
    }
    if let StrategySpec::Radial { .. } = s {
        rows.push(angle("passes/angle_from", "Spokes from", p.angle_from));
        rows.push(angle("passes/angle_to", "Spokes to", p.angle_to));
    }
    if matches!(
        s,
        StrategySpec::Scallop { .. }
            | StrategySpec::Contour { .. }
            | StrategySpec::Spiral { .. }
            | StrategySpec::Radial { .. }
            | StrategySpec::MorphedSpiral { .. }
    ) {
        rows.push(angle("passes/slope_from", "Slopes from", p.slope_from));
        rows.push(angle("passes/slope_to", "Slopes to", p.slope_to));
    }
    if let StrategySpec::Drill { .. } = s {
        optional_row(
            &mut rows,
            "passes",
            "peck",
            "Peck",
            "all the way at once",
            p.peck,
            Dim::Length,
            feature,
        );
        optional_row(
            &mut rows,
            "passes",
            "dwell",
            "Dwell at the bottom, s",
            "none",
            p.dwell,
            Dim::Number,
            feature,
        );
    }
    rows.push(length(
        "passes/max_cut_step",
        "Deepest step",
        p.max_cut_step,
    ));
    rows
}

/// The Tabs group, for a strategy that leaves them.
pub fn tabs_group(data: &OperationData, feature: &str) -> Option<Widget> {
    if !data.strategy().uses().tabs {
        return None;
    }
    let p = &data.passes;
    let mut rows = vec![Widget::Toggle {
        id: "passes/tabs".into(),
        label: "Leave tabs to hold the part".into(),
        on: p.tabs,
    }];
    if p.tabs {
        rows.push(pick("passes/tab_shape", "Shape", &TAB_SHAPES, p.tab_shape));
        for (field, label, v) in [
            ("tab_width", "Width", p.tab_width),
            ("tab_height", "Height", p.tab_height),
            ("tab_spacing", "About this far apart", p.tab_spacing),
        ] {
            rows.push(number(
                &format!("passes/{field}"),
                label,
                v,
                Dim::Length,
                2,
                feature,
            ));
        }
        rows.push(Widget::Text {
            text: "Tabs go evenly round each loop cut at the deepest level, and every cut \
                   over one rides over its top."
                .into(),
            mono: false,
        });
    }
    Some(group("Tabs", rows))
}

/// The Finishing passes group, for a strategy that has them.
pub fn finishing_group(
    data: &OperationData,
    tool: Option<&ToolSpec>,
    feature: &str,
) -> Option<Widget> {
    if !data.strategy().uses().finishing {
        return None;
    }
    let p = &data.passes;
    let mut rows = vec![Widget::Toggle {
        id: "passes/finishing".into(),
        label: "Finish the walls with the side of the tool".into(),
        on: p.finishing,
    }];
    if p.finishing {
        rows.push(number(
            "passes/finishing_passes",
            "Passes",
            p.finishing_passes,
            Dim::Number,
            0,
            feature,
        ));
        let tenth = match tool {
            Some(t) => format!("a tenth of the tool, {:.2} mm", 0.1 * t.diameter),
            None => "a tenth of the tool".into(),
        };
        optional_row(
            &mut rows,
            "passes",
            "finishing_stepover",
            "Stepover",
            &tenth,
            p.finishing_stepover,
            Dim::Length,
            feature,
        );
        optional_row(
            &mut rows,
            "passes",
            "finishing_feed",
            "Feed",
            "the cutting feed",
            p.finishing_feed,
            Dim::Number,
            feature,
        );
        rows.push(Widget::Toggle {
            id: "passes/finishing_repeat".into(),
            label: "Cut the last one twice".into(),
            on: p.finishing_repeat,
        });
    }
    Some(group("Finishing passes", rows))
}

/// Where an optional number starts when it is first given.
fn start(field: &str, data: &OperationData, tool: Option<&ToolSpec>) -> Option<f64> {
    let d = tool.map_or(6.0, |t| t.diameter);
    Some(match field {
        "outer_radius" => data.passes.inner_radius + 10.0,
        "peck" => 2.0,
        "dwell" => 0.5,
        "finishing_stepover" => 0.1 * d,
        "finishing_feed" => data.feed,
        _ => return None,
    })
}

/// A range camcraft would refuse put right, as camcraft's app does as it is
/// set: slopes from below to, spokes no more than a turn, the inner radius
/// inside the outer.
fn put_right(p: &mut PassesData) {
    p.slope_from = p.slope_from.clamp(0.0, 89.0);
    p.slope_to = p.slope_to.clamp(1.0, 90.0);
    if p.slope_from >= p.slope_to {
        p.slope_to = (p.slope_from + 1.0).min(90.0);
        p.slope_from = p.slope_from.min(p.slope_to - 1.0);
    }
    if p.angle_to <= p.angle_from {
        p.angle_to = p.angle_from + 1.0;
    }
    if p.angle_to > p.angle_from + 360.0 {
        p.angle_to = p.angle_from + 360.0;
    }
    p.inner_radius = p.inner_radius.max(0.0);
    if p.outer_radius.given && p.inner_radius >= p.outer_radius.value {
        p.outer_radius.value = p.inner_radius + 1.0;
    }
    p.finishing_passes = p.finishing_passes.round().clamp(1.0, 20.0);
}

/// A widget of the passes' rows changed: `id` without its `passes/`. Whether it
/// was one of theirs.
pub fn passes_event(
    data: &mut OperationData,
    id: &str,
    event: &PanelEvent,
    tool: Option<&ToolSpec>,
) -> bool {
    let s = &mut data.operation.strategy;
    match (id, event) {
        ("order", PanelEvent::Choice { index, .. }) => {
            return set(order_mut(s), ORDERS.get(*index).map(|(x, _)| *x));
        }
        ("contour_order", PanelEvent::Choice { index, .. }) => {
            let StrategySpec::Contour { order, .. } = s else {
                return false;
            };
            return set(Some(order), CONTOUR_ORDERS.get(*index).map(|(x, _)| *x));
        }
        ("radial_direction", PanelEvent::Choice { index, .. }) => {
            let StrategySpec::Radial { direction, .. } = s else {
                return false;
            };
            return set(
                Some(direction),
                RADIAL_DIRECTIONS.get(*index).map(|(x, _)| *x),
            );
        }
        ("face_direction", PanelEvent::Choice { index, .. }) => {
            let StrategySpec::Face { direction, .. } = s else {
                return false;
            };
            return set(
                Some(direction),
                FACE_DIRECTIONS.get(*index).map(|(x, _)| *x),
            );
        }
        ("flat_areas", PanelEvent::Toggle { on, .. }) => return set(flat_areas_mut(s), Some(*on)),
        ("tab_shape", PanelEvent::Choice { index, .. }) => {
            return set(
                Some(&mut data.passes.tab_shape),
                TAB_SHAPES.get(*index).map(|(x, _)| *x),
            );
        }
        _ => {}
    }
    let mut value = serde_json::to_value(data.passes).unwrap_or(Value::Null);
    let pointer = format!("/{id}");
    match event {
        PanelEvent::Number { value: v, .. } => {
            let Some(slot) = value.pointer_mut(&pointer).filter(|s| s.is_number()) else {
                return false;
            };
            *slot = serde_json::json!(v);
        }
        PanelEvent::Toggle { on, .. } => {
            if let Some(field) = id.strip_suffix("/given")
                && *on
                && let Some(s) = start(field, data, tool)
            {
                value[field]["value"] = serde_json::json!(s);
            }
            let Some(slot) = value.pointer_mut(&pointer).filter(|s| s.is_boolean()) else {
                return false;
            };
            *slot = Value::Bool(*on);
        }
        _ => return false,
    }
    match serde_json::from_value(value) {
        Ok(mut p) => {
            put_right(&mut p);
            data.passes = p;
            true
        }
        Err(_) => false,
    }
}

fn set<T>(slot: Option<&mut T>, to: Option<T>) -> bool {
    match (slot, to) {
        (Some(slot), Some(to)) => {
            *slot = to;
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(strategy: &str) -> OperationData {
        OperationData::new(None, named(strategy))
    }

    #[test]
    fn a_strategys_numbers_reach_its_operation() {
        let mut radial = op("radial");
        let p = &mut radial.passes;
        p.centre = true;
        (p.centre_x, p.centre_y) = (5.0, -3.0);
        p.inner_radius = 2.0;
        p.outer_radius = Opt::of(Some(20.0));
        (p.angle_from, p.angle_to) = (10.0, 190.0);
        (p.slope_from, p.slope_to) = (5.0, 60.0);
        let cut = radial.operation("");
        let StrategySpec::Radial {
            centre,
            inner_radius,
            outer_radius,
            angle_from,
            angle_to,
            slope_from,
            slope_to,
            ..
        } = cut.strategy
        else {
            panic!("{:?}", cut.strategy);
        };
        assert_eq!(centre, Some([5.0, -3.0]));
        assert_eq!((inner_radius, outer_radius), (2.0, 20.0));
        assert_eq!((angle_from, angle_to), (10.0, 190.0));
        assert_eq!((slope_from, slope_to), (5.0, 60.0));
        assert_eq!(PassesData::of(&cut), radial.passes);

        let mut drill = op("drill");
        drill.passes.peck = Opt::of(Some(1.5));
        assert!(matches!(
            drill.operation("").strategy,
            StrategySpec::Drill {
                peck: Some(1.5),
                dwell: None
            }
        ));
    }

    #[test]
    fn tabs_and_finishing_only_where_the_strategy_has_them() {
        let mut profile = op("profile");
        profile.passes.tabs = true;
        profile.passes.tab_width = 8.0;
        profile.passes.finishing = true;
        let cut = profile.operation("");
        assert_eq!(cut.passes.tabs.unwrap().width, 8.0);
        assert_eq!(
            cut.passes.finishing.is_some(),
            cut.strategy.uses().finishing
        );
        let mut pocket = op("pocket");
        pocket.passes.finishing = true;
        pocket.passes.finishing_passes = 3.0;
        pocket.passes.finishing_repeat = true;
        pocket.passes.tabs = true;
        let cut = pocket.operation("");
        assert_eq!(cut.passes.finishing.unwrap().passes, Some(3));
        assert!(cut.passes.finishing.unwrap().repeat);
        assert_eq!(cut.passes.tabs, None, "a pocket leaves no tabs");
        assert_eq!(OperationData::from(&pocket.to_value()).unwrap(), pocket);
    }

    #[test]
    fn an_operation_from_before_passes_cuts_with_camcrafts() {
        for strategy in [
            "radial",
            "pocket-clearing",
            "face",
            "drill",
            "profile",
            "scallop",
        ] {
            let op = op(strategy);
            let mut value = op.to_value();
            value.as_object_mut().unwrap().remove("passes");
            let old = OperationData::from(&value).unwrap();
            assert_eq!(old.operation(""), op.operation(""), "{strategy}");
        }
    }

    #[test]
    fn a_range_camcraft_would_refuse_is_put_right() {
        let mut scallop = op("scallop");
        let set = |id: &str, value: f64| PanelEvent::Number {
            id: format!("passes/{id}"),
            value,
        };
        assert!(passes_event(
            &mut scallop,
            "slope_from",
            &set("slope_from", 95.0),
            None
        ));
        assert_eq!(
            (scallop.passes.slope_from, scallop.passes.slope_to),
            (89.0, 90.0)
        );
        let mut spiral = op("spiral");
        spiral.passes.inner_radius = 4.0;
        let tick = PanelEvent::Toggle {
            id: "passes/outer_radius/given".into(),
            on: true,
        };
        assert!(passes_event(&mut spiral, "outer_radius/given", &tick, None));
        assert_eq!(spiral.passes.outer_radius, Opt::of(Some(14.0)));
        let order = PanelEvent::Choice {
            id: "passes/order".into(),
            index: 1,
        };
        assert!(passes_event(&mut spiral, "order", &order, None));
        assert!(matches!(
            spiral.operation.strategy,
            StrategySpec::Spiral {
                order: PassOrder::InsideOut,
                ..
            }
        ));
        // A choice the strategy has not.
        assert!(!passes_event(&mut op("parallel"), "order", &order, None));
    }
}
