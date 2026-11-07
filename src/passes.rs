//! The rest of an operation's Passes tab: the numbers and choices its strategy
//! has of its own, the deepest step a cut takes, the tabs a profile leaves, and
//! a 2D cut's finishing passes. Their numbers, the panel's rows of them, and
//! what the rows' widgets do.

use camcraft_core::job::{
    ChamferSide, ContourOrder, FaceDirection, Finishing, Operation, PassOrder, RadialDirection,
    Relief, ReliefStyle, Rest, RestSource, SlotPattern, StrategySpec, TabShape, Tabs, ThreadHand,
    ToolKind, ToolSpec, TraceDirection,
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
    /// How far a peck backs off to break the chip, rather than out of the
    /// hole: Fusion's chip break distance.
    #[serde(default)]
    pub chip_break: Opt,
    /// A tap's thread: its pitch, mm a turn, and its hand. Given, each hole is
    /// tapped rather than drilled, and the peck, chip break and dwell wait.
    #[serde(default)]
    pub pitch: Opt,
    #[serde(default)]
    pub hand: ThreadHand,
    /// How deep an engraving or a slot cuts below its top.
    #[serde(default = "half")]
    pub depth: f64,
    /// How wide a chamfer is, seen from above, and how far past its foot the
    /// tip goes.
    #[serde(default = "half")]
    pub chamfer_width: f64,
    #[serde(default = "fifth")]
    pub tip_offset: f64,
    /// The deepest a V-carving goes; the V's own full depth unless given.
    #[serde(default)]
    pub max_depth: Opt,
    /// How far down a bore's helix goes each turn.
    #[serde(default = "one_mm")]
    pub bore_pitch: f64,
    /// How far out from a boss circular milling clears in from.
    #[serde(default)]
    pub boss_width: f64,
    /// A milled thread's pitch, mm a turn, its major diameter (its hole's
    /// or boss's own unless given), and the radial passes it is cut in.
    #[serde(default = "one_mm")]
    pub thread_pitch: f64,
    #[serde(default)]
    pub thread_diameter: Opt,
    #[serde(default = "one_mm")]
    pub thread_passes: f64,
    /// Adaptive clearing's rest machining: only what the operations before
    /// it left, rest thinner than the adjustment taken as cleared.
    #[serde(default)]
    pub rest: bool,
    #[serde(default)]
    pub rest_adjustment: f64,
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
    /// Inside corners relieved, dogbones or T-bones, and only those sharper
    /// than an angle, in degrees, when one is given.
    #[serde(default)]
    pub relief: bool,
    #[serde(default)]
    pub relief_style: ReliefStyle,
    #[serde(default = "obtuse")]
    pub relief_angle: Opt,
    /// Whether a 3D finish leaves out the cuts where the tool's shank or
    /// holder would meet the part: Fusion's Use Shank and Holder.
    #[serde(default)]
    pub use_holder: bool,
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
            chip_break: Opt::default(),
            pitch: Opt::default(),
            hand: ThreadHand::default(),
            depth: half(),
            chamfer_width: half(),
            tip_offset: fifth(),
            max_depth: Opt::default(),
            bore_pitch: one_mm(),
            boss_width: 0.0,
            thread_pitch: one_mm(),
            thread_diameter: Opt::default(),
            thread_passes: 1.0,
            rest: false,
            rest_adjustment: 0.0,
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
            relief: false,
            relief_style: ReliefStyle::default(),
            relief_angle: obtuse(),
            use_holder: false,
        };
        for name in [
            "radial",
            "pocket-clearing",
            "face",
            "drill",
            "engrave",
            "chamfer",
            "bore",
            "circular",
            "thread",
        ] {
            out.read_strategy(&named(name));
        }
        out
    }
}

/// An engraving's depth in a document from before it had one: camcraft's.
fn half() -> f64 {
    0.5
}

/// The angle corner relief starts from when one is first given: every
/// corner sharper than the walls of a 45° chamfer's.
fn obtuse() -> Opt {
    Opt {
        value: 120.0,
        given: false,
    }
}

/// A chamfer's tip offset in a document from before it had one: camcraft's.
fn fifth() -> f64 {
    0.2
}

/// A bore's and a thread's pitch, and a thread's passes, in a document from
/// before they had them: camcraft's.
fn one_mm() -> f64 {
    1.0
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
        out.use_holder = p.use_holder;
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
        if let Some(r) = p.relief {
            out.relief = true;
            out.relief_style = r.style;
            if let Some(a) = r.sharper_than {
                out.relief_angle = Opt::of(Some(a));
            }
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
            StrategySpec::Drill {
                peck,
                dwell,
                chip_break,
                pitch,
                hand,
            } => {
                // Tapping, the others are not given, and keep what they were.
                if pitch.is_none() {
                    self.peck = Opt::of(peck);
                    self.dwell = Opt::of(dwell);
                    self.chip_break = Opt::of(chip_break);
                }
                self.pitch = Opt::of(pitch);
                self.hand = hand;
            }
            StrategySpec::Engrave { depth, .. } | StrategySpec::Slot { depth, .. } => {
                self.depth = depth
            }
            StrategySpec::Chamfer {
                width, tip_offset, ..
            } => (self.chamfer_width, self.tip_offset) = (width, tip_offset),
            StrategySpec::VCarve { max_depth } => self.max_depth = Opt::of(max_depth),
            StrategySpec::Bore { pitch, .. } => self.bore_pitch = pitch,
            StrategySpec::Circular { boss_width, .. } => self.boss_width = boss_width,
            StrategySpec::Thread {
                pitch,
                hand,
                diameter,
                radial_passes,
                ..
            } => {
                self.thread_pitch = pitch;
                self.hand = hand;
                self.thread_diameter = Opt::of(diameter);
                self.thread_passes = f64::from(radial_passes);
            }
            StrategySpec::Adaptive { rest, .. } => {
                // Off, the adjustment keeps what it was, for when it is on.
                self.rest = rest.is_some();
                if let Some(r) = rest {
                    self.rest_adjustment = r.adjustment;
                }
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
            StrategySpec::Drill {
                peck,
                dwell,
                chip_break,
                pitch,
                hand,
            } => {
                // A tap neither pecks nor pauses; a chip is broken between pecks.
                let tapping = self.pitch.given;
                let drilling = |v: Opt| v.get().filter(|_| !tapping);
                *peck = drilling(self.peck);
                *dwell = drilling(self.dwell);
                *chip_break = drilling(self.chip_break).filter(|_| self.peck.given);
                *pitch = self.pitch.get();
                *hand = self.hand;
            }
            StrategySpec::Engrave { depth, .. } | StrategySpec::Slot { depth, .. } => {
                *depth = self.depth
            }
            StrategySpec::Chamfer {
                width, tip_offset, ..
            } => (*width, *tip_offset) = (self.chamfer_width, self.tip_offset),
            StrategySpec::VCarve { max_depth } => *max_depth = self.max_depth.get(),
            StrategySpec::Bore { pitch, .. } => *pitch = self.bore_pitch,
            StrategySpec::Circular { boss_width, .. } => *boss_width = self.boss_width,
            StrategySpec::Thread {
                pitch,
                hand,
                diameter,
                radial_passes,
                ..
            } => {
                *pitch = self.thread_pitch;
                *hand = self.hand;
                *diameter = self.thread_diameter.get();
                *radial_passes = self.thread_passes.round().clamp(1.0, 20.0) as u32;
            }
            StrategySpec::Adaptive { rest, .. } => {
                *rest = self.rest.then_some(Rest {
                    source: RestSource::FromPreviousOperations,
                    adjustment: self.rest_adjustment,
                });
            }
            _ => {}
        }
        let uses = op.strategy.uses();
        let finish = follows_part(&op.strategy);
        let p = &mut op.passes;
        p.max_cut_step = self.max_cut_step;
        p.use_holder = finish && self.use_holder;
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
        p.relief = (uses.relief && self.relief).then_some(Relief {
            style: self.relief_style,
            sharper_than: self.relief_angle.get(),
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
        let tap = self.pitch.given;
        let drill = matches!(s, StrategySpec::Drill { .. }) && !tap;
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
        add(
            matches!(s, StrategySpec::Engrave { .. } | StrategySpec::Slot { .. }),
            "depth",
            "Depth",
            Dim::Length,
        );
        add(
            matches!(s, StrategySpec::Adaptive { .. }) && self.rest,
            "rest_adjustment",
            "Rest adjustment",
            Dim::Length,
        );
        let chamfer = matches!(s, StrategySpec::Chamfer { .. });
        add(chamfer, "chamfer_width", "Chamfer width", Dim::Length);
        add(
            matches!(s, StrategySpec::Bore { .. }),
            "bore_pitch",
            "Helix pitch",
            Dim::Length,
        );
        add(
            matches!(s, StrategySpec::Circular { .. }),
            "boss_width",
            "Boss stock width",
            Dim::Length,
        );
        let thread = matches!(s, StrategySpec::Thread { .. });
        add(thread, "thread_pitch", "Thread pitch", Dim::Length);
        add(thread, "thread_passes", "Radial passes", Dim::Number);
        add(chamfer, "tip_offset", "Tip offset", Dim::Length);
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
            (
                drill && self.peck.given && self.chip_break.given,
                "chip_break",
                "Chip break distance",
                Dim::Length,
            ),
            (drill && self.dwell.given, "dwell", "Dwell, s", Dim::Number),
            (
                matches!(s, StrategySpec::VCarve { .. }) && self.max_depth.given,
                "max_depth",
                "Deepest",
                Dim::Length,
            ),
            (
                matches!(s, StrategySpec::Drill { .. }) && tap,
                "pitch",
                "Thread pitch",
                Dim::Length,
            ),
            (
                thread && self.thread_diameter.given,
                "thread_diameter",
                "Thread diameter",
                Dim::Length,
            ),
            (
                uses.relief && self.relief && self.relief_angle.given,
                "relief_angle",
                "Relieve corners sharper than",
                Dim::Angle,
            ),
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
const HANDS: [(ThreadHand, &str); 2] = [
    (ThreadHand::Right, "Right-hand"),
    (ThreadHand::Left, "Left-hand"),
];
const THREAD_HANDS: [(ThreadHand, &str); 2] = [
    (ThreadHand::Right, "Right-hand"),
    (ThreadHand::Left, "Left-hand"),
];
const TRACE_DIRECTIONS: [(TraceDirection, &str); 3] = [
    (TraceDirection::Climb, "Climb"),
    (TraceDirection::Conventional, "Conventional"),
    (TraceDirection::AsDrawn, "As drawn"),
];
const SLOT_PATTERNS: [(SlotPattern, &str); 2] = [
    (
        SlotPattern::Bidirectional,
        "Both ways: back and forth, ramping down",
    ),
    (
        SlotPattern::Directional,
        "One way: a level each time, lifting back",
    ),
];
const CHAMFER_SIDES: [(ChamferSide, &str); 2] = [
    (ChamferSide::Auto, "Away from the material"),
    (ChamferSide::Flipped, "Flipped: the other side"),
];
const RELIEF_STYLES: [(ReliefStyle, &str); 5] = [
    (ReliefStyle::Dogbone, "Dogbone: into the corner"),
    (ReliefStyle::TBoneX, "T-bone along X"),
    (ReliefStyle::TBoneY, "T-bone along Y"),
    (ReliefStyle::TBoneLong, "T-bone along the longer wall"),
    (ReliefStyle::TBoneShort, "T-bone along the shorter wall"),
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

/// The strategy's own rows of the Passes group, and the deepest step, for an
/// operation cutting with `tool`.
pub fn strategy_rows(data: &OperationData, tool: Option<&ToolSpec>, feature: &str) -> Vec<Widget> {
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
    if let StrategySpec::Adaptive { .. } = s {
        rows.push(Widget::Toggle {
            id: "passes/rest".into(),
            label: "Rest machining, from previous operations".into(),
            on: p.rest,
        });
        if p.rest {
            rows.push(length(
                "passes/rest_adjustment",
                "Rest adjustment",
                p.rest_adjustment,
            ));
            rows.push(Widget::Text {
                text: "Clears only what the operations before it in the setup left. \
                       Rest thinner than the adjustment is left alone."
                    .into(),
                mono: false,
            });
        }
    }
    if let StrategySpec::PocketClearing { .. } = s {
        rows.push(Widget::Text {
            text: "Pocket clearing always clears what the operations before it left.".into(),
            mono: false,
        });
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
        drill_rows(&mut rows, data, tool, feature);
    }
    if let StrategySpec::Bore { .. } = s {
        rows.push(length("passes/bore_pitch", "Helix pitch", p.bore_pitch));
        rows.push(Widget::Text {
            text: "Down a helix from just above each hole to its bottom, the pitch a turn, \
                   round once there, then out to the wall in the finishing passes, each in \
                   and out along an arc. A hole wider than twice the tool leaves a core in \
                   its middle, and says so."
                .into(),
            mono: false,
        });
    }
    if let StrategySpec::Circular { .. } = s {
        rows.push(length(
            "passes/boss_width",
            "Boss stock width",
            p.boss_width,
        ));
        rows.push(Widget::Text {
            text: "A hole is cleared in circles from its middle out to its wall; a boss \
                   from the boss stock width out from it in to it (one circle round it \
                   when 0). Each level is entered down a helix at the ramp angle, and \
                   its last circle left along an arc."
                .into(),
            mono: false,
        });
    }
    if let StrategySpec::Thread { spring_pass, .. } = s {
        thread_rows(&mut rows, data, spring_pass, tool, feature);
    }
    if let StrategySpec::Engrave {
        direction, reverse, ..
    } = s
    {
        rows.push(length("passes/depth", "Depth", p.depth));
        rows.push(pick(
            "passes/trace_direction",
            "Direction",
            &TRACE_DIRECTIONS,
            direction,
        ));
        rows.push(Widget::Toggle {
            id: "passes/reverse".into(),
            label: "Reverse".into(),
            on: reverse,
        });
        rows.push(Widget::Text {
            text: "Climbing goes round each closed curve as cutting what it encloses \
                   climbs; an open curve runs as drawn. Each curve ramps down along \
                   itself at the ramp angle, round a closed one and back and forth \
                   along an open one, a stepdown at most each time, then once at \
                   the depth."
                .into(),
            mono: false,
        });
    }
    if let StrategySpec::Slot {
        pattern, reverse, ..
    } = s
    {
        rows.push(length("passes/depth", "Depth", p.depth));
        rows.push(pick(
            "passes/slot_pattern",
            "Pattern",
            &SLOT_PATTERNS,
            pattern,
        ));
        rows.push(Widget::Toggle {
            id: "passes/reverse".into(),
            label: "Reverse".into(),
            on: reverse,
        });
        let width = match tool {
            Some(t) => format!("{:.2} mm, the tool's", t.diameter),
            None => "the tool's".into(),
        };
        rows.push(Widget::Text {
            text: format!(
                "The slot is {width} wide, along each curve as drawn. Both ways, it goes \
                 a stepdown deeper each time along it, ramping as it goes, then once at \
                 the depth; one way, it cuts each level from the start, lifting at the \
                 end, and goes into each as the linking's entry says."
            ),
            mono: false,
        });
    }
    if let StrategySpec::Chamfer { side, .. } = s {
        rows.push(length(
            "passes/chamfer_width",
            "Chamfer width",
            p.chamfer_width,
        ));
        rows.push(length("passes/tip_offset", "Tip offset", p.tip_offset));
        rows.push(pick("passes/chamfer_side", "Side", &CHAMFER_SIDES, side));
        rows.push(Widget::Text {
            text: chamfer_note(p, tool),
            mono: false,
        });
    }
    if let StrategySpec::VCarve { .. } = s {
        let full = match tool.and_then(v_depth) {
            Some(d) => format!("the V's full depth, {d:.2} mm"),
            None => "the V's full depth".into(),
        };
        optional_row(
            &mut rows,
            "passes",
            "max_depth",
            "Deepest",
            &full,
            p.max_depth,
            Dim::Length,
            feature,
        );
        rows.push(Widget::Text {
            text: "The V goes down the middle of each area, as deep as it has to be to \
                   reach the walls. Where the area is wider than that at the deepest, it \
                   cuts the walls that deep and leaves the floor between them flat, \
                   and says so: clear it with a flat end mill."
                .into(),
            mono: false,
        });
    }
    if follows_part(&s) {
        rows.push(Widget::Toggle {
            id: "passes/use_holder".into(),
            label: "Use shank and holder: leave out cuts where they meet the part".into(),
            on: p.use_holder,
        });
    }
    rows.push(length(
        "passes/max_cut_step",
        "Deepest step",
        p.max_cut_step,
    ));
    rows
}

/// Whether `s` is a 3D finish, which follows the part and so can leave out
/// the cuts where the tool's shank or holder would meet it.
fn follows_part(s: &StrategySpec) -> bool {
    matches!(
        s,
        StrategySpec::Parallel { .. }
            | StrategySpec::Scallop { .. }
            | StrategySpec::Contour { .. }
            | StrategySpec::Flat { .. }
            | StrategySpec::Spiral { .. }
            | StrategySpec::Radial { .. }
            | StrategySpec::MorphedSpiral { .. }
    )
}

/// Where a chamfer puts `tool`, or why it cannot.
fn chamfer_note(p: &PassesData, tool: Option<&ToolSpec>) -> String {
    let what = "Each edge is chamfered from its own height, the tool's centre beside it \
                away from the material, as the part either side says, and its tip below \
                the chamfer by the tip offset, so the flank cuts it.";
    let Some(t) = tool.filter(|t| t.kind == ToolKind::Cone) else {
        return format!("{what} It cuts with a chamfer mill or a V-bit.");
    };
    let half = (t.angle.unwrap_or(f64::NAN) / 2.0).to_radians();
    match camcraft_core::strategy::chamfer::Place::new(
        p.chamfer_width,
        p.tip_offset,
        half,
        t.tip_diameter / 2.0,
        t.diameter / 2.0,
    ) {
        Ok(at) => format!(
            "{what} With this tool: its tip {:.2} mm down, its centre {:.2} mm off the edge.",
            at.depth, at.offset
        ),
        Err(e) => format!("{what} This tool is too small: {e}."),
    }
}

/// How deep `tool`'s V goes before it is as wide as the tool, if it is a V.
pub fn v_depth(tool: &ToolSpec) -> Option<f64> {
    let half = (tool.angle? / 2.0).to_radians();
    (tool.kind == ToolKind::Cone && half > 0.0)
        .then(|| (tool.diameter - tool.tip_diameter).max(0.0) / 2.0 / half.tan())
}

/// A thread milling operation's rows: its pitch, hand, diameter and passes,
/// and what that comes to with `tool`.
fn thread_rows(
    rows: &mut Vec<Widget>,
    data: &OperationData,
    spring_pass: bool,
    tool: Option<&ToolSpec>,
    feature: &str,
) {
    let p = &data.passes;
    rows.push(number(
        "passes/thread_pitch",
        "Thread pitch",
        p.thread_pitch,
        Dim::Length,
        3,
        feature,
    ));
    rows.push(pick("passes/hand", "Thread", &THREAD_HANDS, p.hand));
    optional_row(
        rows,
        "passes",
        "thread_diameter",
        "Major diameter",
        "a hole's plus the thread's depth, a boss's own",
        p.thread_diameter,
        Dim::Length,
        feature,
    );
    rows.push(number(
        "passes/thread_passes",
        "Radial passes",
        p.thread_passes,
        Dim::Number,
        0,
        feature,
    ));
    rows.push(Widget::Toggle {
        id: "passes/spring_pass".into(),
        label: "Cut the last pass twice (spring pass)".into(),
        on: spring_pass,
    });
    let pitch = p.thread_pitch;
    let depth = |boss| camcraft_core::strategy::holes::thread_depth(pitch, boss);
    let mut text = format!(
        "Round a helix at the pitch a turn, in and out along arcs, the thread's depth \
         ({:.3} mm in a hole, {:.3} mm on a boss) in equal radial passes. Climbing, a \
         right-hand thread goes up a hole and down a boss; the tool's end goes no lower \
         than the bottom.",
        depth(false),
        depth(true)
    );
    if let Some(t) = tool.filter(|t| t.kind == ToolKind::ThreadMill) {
        let teeth = t.threads_per_turn();
        if teeth > 1 {
            text.push_str(&format!(
                " This mill cuts {teeth} turns of the thread each time round."
            ));
        }
        if let Some(why) = t.pitch_problem(pitch) {
            text.push_str(&format!(" It cannot: {why}."));
        }
    } else {
        text.push_str(" It cuts with a thread mill.");
    }
    rows.push(Widget::Text { text, mono: false });
}

/// A drilling operation's rows: its pecks, how they break the chip and the
/// pause at the bottom; or, tapping, the thread and the feed it makes.
fn drill_rows(
    rows: &mut Vec<Widget>,
    data: &OperationData,
    tool: Option<&ToolSpec>,
    feature: &str,
) {
    let p = &data.passes;
    if !p.pitch.given {
        optional_row(
            rows,
            "passes",
            "peck",
            "Peck",
            "all the way at once",
            p.peck,
            Dim::Length,
            feature,
        );
        if p.peck.given {
            optional_row(
                rows,
                "passes",
                "chip_break",
                "Chip break distance",
                "out of the hole each peck",
                p.chip_break,
                Dim::Length,
                feature,
            );
        }
        optional_row(
            rows,
            "passes",
            "dwell",
            "Dwell at the bottom, s",
            "none",
            p.dwell,
            Dim::Number,
            feature,
        );
    }
    optional_row(
        rows,
        "passes",
        "pitch",
        "Tap: thread pitch",
        "drilled, not tapped",
        p.pitch,
        Dim::Length,
        feature,
    );
    if p.pitch.given {
        rows.push(pick("passes/hand", "Thread", &HANDS, p.hand));
        let rpm = match (data.rpm, tool) {
            (r, _) if r > 0.0 => Some(r),
            (_, Some(t)) => Some(f64::from(t.rpm)),
            (_, None) => None,
        };
        let feed = match rpm {
            Some(rpm) => format!("{:.0} mm/min at {rpm:.0} rev/min", p.pitch.value * rpm),
            None => "the pitch times the spindle's speed".into(),
        };
        rows.push(Widget::Text {
            text: format!(
                "Fed in at the pitch a turn, {feed}, and back out with the spindle \
                 reversed. Only a machine that taps can write it: LinuxCNC, Fanuc, \
                 Haas, Centroid CNC12 and Heidenhain do; Masso, GRBL and Mach cannot."
            ),
            mono: false,
        });
    }
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

/// The Corners group, for a strategy that can relieve them.
pub fn relief_group(data: &OperationData, feature: &str) -> Option<Widget> {
    if !data.strategy().uses().relief {
        return None;
    }
    let p = &data.passes;
    let mut rows = vec![Widget::Toggle {
        id: "passes/relief".into(),
        label: "Relieve inside corners, so a square part fits".into(),
        on: p.relief,
    }];
    if p.relief {
        rows.push(pick(
            "passes/relief_style",
            "Style",
            &RELIEF_STYLES,
            p.relief_style,
        ));
        optional_row(
            &mut rows,
            "passes",
            "relief_angle",
            "Corners sharper than",
            "every corner",
            p.relief_angle,
            Dim::Angle,
            feature,
        );
        let how = match p.relief_style {
            ReliefStyle::Dogbone => "out into it, halfway between its walls",
            _ => "on along one wall, into the other",
        };
        rows.push(Widget::Text {
            text: format!(
                "Where the tool leaves an inside corner round, it goes {how}, until \
                 its side reaches the corner's point, and back, at every level. The \
                 notch cuts past the part, as it is meant to: it is no gouge."
            ),
            mono: false,
        });
    }
    Some(group("Corners", rows))
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
        "chip_break" => 0.5,
        "dwell" => 0.5,
        "pitch" => coarse_pitch(d),
        "finishing_stepover" => 0.1 * d,
        "finishing_feed" => data.feed,
        "max_depth" => 1.0,
        "thread_diameter" => nominal(data.passes.thread_pitch),
        "relief_angle" => obtuse().value,
        _ => return None,
    })
}

/// The ISO metric coarse threads, diameter and pitch.
const COARSE: [(f64, f64); 11] = [
    (2.0, 0.4),
    (2.5, 0.45),
    (3.0, 0.5),
    (4.0, 0.7),
    (5.0, 0.8),
    (6.0, 1.0),
    (8.0, 1.25),
    (10.0, 1.5),
    (12.0, 1.75),
    (16.0, 2.0),
    (20.0, 2.5),
];

/// The ISO metric coarse pitch of the thread nearest `diameter` across, as a
/// tap of that size most likely cuts.
fn coarse_pitch(diameter: f64) -> f64 {
    COARSE
        .iter()
        .min_by(|a, b| (a.0 - diameter).abs().total_cmp(&(b.0 - diameter).abs()))
        .map_or(1.0, |(_, p)| *p)
}

/// The diameter of the ISO metric coarse thread whose pitch is nearest
/// `pitch`: where a thread's major diameter most likely starts.
fn nominal(pitch: f64) -> f64 {
    COARSE
        .iter()
        .min_by(|a, b| (a.1 - pitch).abs().total_cmp(&(b.1 - pitch).abs()))
        .map_or(6.0, |(d, _)| *d)
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
    p.thread_passes = p.thread_passes.round().clamp(1.0, 20.0);
    p.tip_offset = p.tip_offset.max(0.0);
    p.boss_width = p.boss_width.max(0.0);
    p.rest_adjustment = p.rest_adjustment.max(0.0);
    p.relief_angle.value = p.relief_angle.value.clamp(1.0, 180.0);
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
        ("trace_direction", PanelEvent::Choice { index, .. }) => {
            let StrategySpec::Engrave { direction, .. } = s else {
                return false;
            };
            return set(
                Some(direction),
                TRACE_DIRECTIONS.get(*index).map(|(x, _)| *x),
            );
        }
        ("spring_pass", PanelEvent::Toggle { on, .. }) => {
            let StrategySpec::Thread { spring_pass, .. } = s else {
                return false;
            };
            return set(Some(spring_pass), Some(*on));
        }
        ("reverse", PanelEvent::Toggle { on, .. }) => {
            let (StrategySpec::Engrave { reverse, .. } | StrategySpec::Slot { reverse, .. }) = s
            else {
                return false;
            };
            return set(Some(reverse), Some(*on));
        }
        ("chamfer_side", PanelEvent::Choice { index, .. }) => {
            let StrategySpec::Chamfer { side, .. } = s else {
                return false;
            };
            return set(Some(side), CHAMFER_SIDES.get(*index).map(|(x, _)| *x));
        }
        ("slot_pattern", PanelEvent::Choice { index, .. }) => {
            let StrategySpec::Slot { pattern, .. } = s else {
                return false;
            };
            return set(Some(pattern), SLOT_PATTERNS.get(*index).map(|(x, _)| *x));
        }
        ("hand", PanelEvent::Choice { index, .. }) => {
            return set(
                Some(&mut data.passes.hand),
                HANDS.get(*index).map(|(x, _)| *x),
            );
        }
        ("relief_style", PanelEvent::Choice { index, .. }) => {
            return set(
                Some(&mut data.passes.relief_style),
                RELIEF_STYLES.get(*index).map(|(x, _)| *x),
            );
        }
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
                dwell: None,
                chip_break: None,
                pitch: None,
                ..
            }
        ));
    }

    /// Rest machining, ticked, reaches an adaptive clearing with its
    /// adjustment and comes back from it; unticked, it is off and the
    /// adjustment waits.
    #[test]
    fn rest_machining_reaches_an_adaptive_clearing() {
        let mut adaptive = op("adaptive");
        assert!(matches!(
            adaptive.operation("").strategy,
            StrategySpec::Adaptive { rest: None, .. }
        ));
        let tick = |on| PanelEvent::Toggle {
            id: "passes/rest".into(),
            on,
        };
        assert!(passes_event(&mut adaptive, "rest", &tick(true), None));
        let number = PanelEvent::Number {
            id: "passes/rest_adjustment".into(),
            value: 0.3,
        };
        assert!(passes_event(
            &mut adaptive,
            "rest_adjustment",
            &number,
            None
        ));
        let cut = adaptive.operation("");
        let StrategySpec::Adaptive { rest: Some(r), .. } = cut.strategy else {
            panic!("{:?}", cut.strategy);
        };
        assert_eq!(r.adjustment, 0.3);
        assert_eq!(PassesData::of(&cut), adaptive.passes);
        assert!(passes_event(&mut adaptive, "rest", &tick(false), None));
        assert!(matches!(
            adaptive.operation("").strategy,
            StrategySpec::Adaptive { rest: None, .. }
        ));
        assert_eq!(adaptive.passes.rest_adjustment, 0.3);
    }

    /// Chip breaking and tapping reach the operation and come back from it,
    /// and while it taps, the peck and the rest wait, kept.
    #[test]
    fn chip_breaking_and_tapping_reach_the_operation() {
        let mut drill = op("drill");
        let tick = |field: &str| PanelEvent::Toggle {
            id: format!("passes/{field}/given"),
            on: true,
        };
        assert!(passes_event(&mut drill, "peck/given", &tick("peck"), None));
        assert!(passes_event(
            &mut drill,
            "chip_break/given",
            &tick("chip_break"),
            None
        ));
        let cut = drill.operation("");
        assert!(matches!(
            cut.strategy,
            StrategySpec::Drill {
                peck: Some(2.0),
                chip_break: Some(0.5),
                pitch: None,
                ..
            }
        ));
        assert_eq!(PassesData::of(&cut), drill.passes);
        let names: Vec<_> = drill
            .passes
            .parameters(&cut)
            .into_iter()
            .map(|(_, name, ..)| name)
            .collect();
        assert!(names.contains(&"chip_break"), "{names:?}");

        // An M8 tap, left-hand.
        let tap: ToolSpec = serde_json::from_value(serde_json::json!({
            "kind": "flat", "diameter": 8.0, "number": 1
        }))
        .unwrap();
        assert!(passes_event(
            &mut drill,
            "pitch/given",
            &tick("pitch"),
            Some(&tap)
        ));
        let left = PanelEvent::Choice {
            id: "passes/hand".into(),
            index: 1,
        };
        assert!(passes_event(&mut drill, "hand", &left, None));
        let cut = drill.operation("");
        assert!(matches!(
            cut.strategy,
            StrategySpec::Drill {
                peck: None,
                chip_break: None,
                dwell: None,
                pitch: Some(1.25),
                hand: ThreadHand::Left,
            }
        ));
        let back = OperationData::from(&drill.to_value()).unwrap();
        assert_eq!(back.passes.peck, Opt::of(Some(2.0)), "the peck waits");
        // A document from before them drills as it did.
        let mut old = op("drill").to_value();
        for field in ["chip_break", "pitch", "hand"] {
            old["passes"].as_object_mut().unwrap().remove(field);
        }
        let old = OperationData::from(&old).unwrap();
        assert_eq!(old.operation(""), op("drill").operation(""));
        let names: Vec<_> = drill
            .passes
            .parameters(&cut)
            .into_iter()
            .map(|(_, name, ..)| name)
            .collect();
        assert!(
            names.contains(&"pitch") && !names.contains(&"peck"),
            "{names:?}"
        );
    }

    /// A bore's pitch, a boss's width and a thread's settings reach the
    /// operation and come back from it; a bore has finishing passes.
    #[test]
    fn hole_milling_reaches_the_operation() {
        let mut bore = op("bore");
        bore.passes.bore_pitch = 0.5;
        bore.passes.finishing = true;
        let cut = bore.operation("");
        assert!(matches!(
            cut.strategy,
            StrategySpec::Bore { pitch: 0.5, .. }
        ));
        assert!(cut.passes.finishing.is_some());
        assert_eq!(PassesData::of(&cut), bore.passes);

        let mut circular = op("circular");
        circular.passes.boss_width = 4.0;
        let cut = circular.operation("");
        assert!(matches!(
            cut.strategy,
            StrategySpec::Circular {
                boss_width: 4.0,
                ..
            }
        ));

        let mut thread = op("thread");
        let tick = PanelEvent::Toggle {
            id: "passes/thread_diameter/given".into(),
            on: true,
        };
        thread.passes.thread_pitch = 1.25;
        assert!(passes_event(
            &mut thread,
            "thread_diameter/given",
            &tick,
            None
        ));
        assert_eq!(thread.passes.thread_diameter, Opt::of(Some(8.0)), "an M8's");
        let spring = PanelEvent::Toggle {
            id: "passes/spring_pass".into(),
            on: true,
        };
        assert!(passes_event(&mut thread, "spring_pass", &spring, None));
        let left = PanelEvent::Choice {
            id: "passes/hand".into(),
            index: 1,
        };
        assert!(passes_event(&mut thread, "hand", &left, None));
        thread.passes.thread_passes = 3.0;
        let cut = thread.operation("");
        assert!(matches!(
            cut.strategy,
            StrategySpec::Thread {
                pitch: 1.25,
                hand: ThreadHand::Left,
                diameter: Some(8.0),
                radial_passes: 3,
                spring_pass: true,
                ..
            }
        ));
        assert_eq!(PassesData::of(&cut), thread.passes);
        assert_eq!(OperationData::from(&thread.to_value()).unwrap(), thread);
        let names: Vec<_> = thread
            .passes
            .parameters(&cut)
            .into_iter()
            .map(|(_, name, ..)| name)
            .collect();
        assert!(names.contains(&"thread_diameter") && names.contains(&"thread_pitch"));
        // A document from before them reads.
        let mut old = op("drill").to_value();
        for field in [
            "bore_pitch",
            "boss_width",
            "thread_pitch",
            "thread_diameter",
            "thread_passes",
        ] {
            old["passes"].as_object_mut().unwrap().remove(field);
        }
        assert_eq!(
            OperationData::from(&old).unwrap().operation(""),
            op("drill").operation("")
        );
    }

    #[test]
    fn shank_and_holder_are_used_only_by_a_3d_finish() {
        for (strategy, kept) in [("parallel", true), ("pocket-clearing", false)] {
            let mut data = op(strategy);
            let toggle = PanelEvent::Toggle {
                id: "passes/use_holder".into(),
                on: true,
            };
            assert!(passes_event(&mut data, "use_holder", &toggle, None));
            let mut o = data.operation.clone();
            data.passes.apply(&mut o);
            assert_eq!(o.passes.use_holder, kept, "{strategy}");
            let shown = strategy_rows(&data, None, "f")
                .iter()
                .any(|w| matches!(w, Widget::Toggle { id, .. } if id == "passes/use_holder"));
            assert_eq!(shown, kept, "{strategy}");
            if kept {
                assert!(PassesData::of(&o).use_holder);
            }
        }
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
