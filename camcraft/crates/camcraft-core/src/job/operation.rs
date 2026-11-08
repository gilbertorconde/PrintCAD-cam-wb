//! An operation, shaped the way Fusion shapes one: its strategy, then what it
//! machines (Fusion's Geometry tab: a selection, a 3D boundary or a 2D outline),
//! Heights, Passes and Linking.

use serde::{Deserialize, Serialize};

use super::{
    Depths, Heights, HolePicking, Linking, MachiningBoundary, Outline, Passes, Pattern, Preset,
    ProfileSide, Selection, StrategySpec, ToolKind, ToolSpec, is_default, is_yes, yes,
};

/// One operation.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Operation {
    /// Name, for comments and diagnostics.
    pub name: String,
    /// Which strategy, and its own parameters.
    #[serde(flatten)]
    pub strategy: StrategySpec,
    /// Which cutter, by name from the job's tools. A new operation has none yet,
    /// and cannot run until it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// The tool's preset its values were copied from, by name. Only a record: the
    /// operation cuts with its own values, whatever the preset now says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// Faces and edges picked on the models, for whatever of it reads them: a
    /// boundary or outline of the selection, a height measured from it, the holes
    /// a drill drills.
    #[serde(default, skip_serializing_if = "is_default")]
    pub selection: Selection,
    /// How a hole operation finds more holes than those picked, and the order
    /// it takes them in ([`Uses::holes`](super::Uses::holes)).
    #[serde(default, skip_serializing_if = "is_default")]
    pub holes: HolePicking,
    /// Where a 3D operation's tool may go
    /// ([`Uses::boundary`](super::Uses::boundary)).
    #[serde(default, skip_serializing_if = "is_default")]
    pub boundary: MachiningBoundary,
    /// What a 2D operation cuts round or clears
    /// ([`Uses::outline`](super::Uses::outline)).
    #[serde(default, skip_serializing_if = "is_default")]
    pub outline: Outline,
    /// Where the tool travels, and the levels it cuts between.
    #[serde(default, skip_serializing_if = "is_default")]
    pub heights: Heights,
    /// How the passes are laid out.
    #[serde(default, skip_serializing_if = "is_default")]
    pub passes: Passes,
    /// How the tool moves between cuts.
    #[serde(default, skip_serializing_if = "is_default")]
    pub linking: Linking,
    /// Its toolpath repeated in rows, round a centre, or mirrored, before it
    /// is linked: none unless asked. A job file calls it `transform`, as a
    /// slot's own `pattern` is written beside it.
    #[serde(default, skip_serializing_if = "is_default")]
    pub transform: Pattern,
    /// Whether it runs. One turned off stays in the job, but is not worked out,
    /// cut, checked or posted.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub enabled: bool,
    /// The lines a Custom G-code operation ([`StrategySpec::CustomGcode`])
    /// writes into the program, one a line; no other reads them.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub gcode: String,
}

impl Operation {
    /// A new operation called `name` of `strategy`, with no tool yet, and
    /// everything else as Fusion starts it: the strategy's own multiple depths
    /// and stock to leave ([`StrategySpec::default_stepdown`],
    /// [`StrategySpec::default_stock_to_leave`]), and the rest as a job file that
    /// gives none has it.
    pub fn new(name: impl Into<String>, strategy: StrategySpec) -> Self {
        Self {
            name: name.into(),
            strategy,
            tool: None,
            preset: None,
            selection: Selection::default(),
            holes: HolePicking::default(),
            boundary: MachiningBoundary::default(),
            outline: Outline::default(),
            heights: Heights::default(),
            passes: Passes {
                stepdown: strategy.default_stepdown(),
                stock_to_leave: strategy.default_stock_to_leave(),
                ..Passes::default()
            },
            linking: Linking::default(),
            transform: Pattern::default(),
            enabled: true,
            gcode: String::new(),
        }
    }

    /// Why this operation cannot cut with `tool`, if it cannot.
    ///
    /// A cone slices badly into Z levels (its reach widens all the way up, so the
    /// levels need far too many offsets) and cannot leave stock (grown by a ball it
    /// is not a cone); it finishes, engraves and chamfers.
    ///
    /// Negative stock shrinks the tool, as Fusion's does, so it can take no more
    /// than the tool's radius beside; and in 3D, where a round end has to stay
    /// round, no more than its corner radius down.
    pub fn cannot_use(&self, tool: &ToolSpec) -> Option<&'static str> {
        // A probe touches and cuts nothing; nothing else touches.
        let probing = matches!(self.strategy, StrategySpec::Probe { .. });
        if probing != (tool.kind == ToolKind::Probe) {
            return Some(if probing {
                "probe with a touch probe"
            } else {
                "a touch probe cuts nothing; use a probe operation"
            });
        }
        if probing {
            return None;
        }
        let uses = self.strategy.uses();
        // A thread mill's teeth are the thread's form: it cuts nothing else,
        // and a thread is cut by nothing else.
        let threading = matches!(self.strategy, StrategySpec::Thread { .. });
        if threading != (tool.kind == ToolKind::ThreadMill) {
            return Some(if threading {
                "mill a thread with a thread mill"
            } else {
                "a thread mill only mills threads; use a thread operation"
            });
        }
        if let StrategySpec::Thread { pitch, .. } = self.strategy
            && tool.pitch_problem(pitch).is_some()
        {
            return Some("the thread's pitch is outside the thread mill's range of pitches");
        }
        if let Some(why) = self.cannot_use_kind(tool) {
            return Some(why);
        }
        if uses.stepover && self.passes.stepover > tool.diameter {
            return Some(
                "the stepover is wider than the tool: it would leave ridges between passes",
            );
        }
        let leave = self.stock_to_leave();
        let leaves = leave.radial != 0.0 || leave.axial != 0.0;
        let least = self.least_stock_to_leave(tool);
        if leave.radial <= least.radial {
            return Some("negative stock to leave beside must be less than the tool's radius");
        }
        if leave.axial < least.axial {
            return Some(
                "negative stock to leave down must be no more than the tool's corner radius",
            );
        }
        if !tool.kind.is_cone() {
            return None;
        }
        match self.strategy {
            StrategySpec::PocketClearing { .. } | StrategySpec::Adaptive { .. } => {
                return Some(
                    "a V-bit or chamfer mill cannot rough in Z levels; use a parallel pass",
                );
            }
            StrategySpec::Contour { .. } => {
                return Some(
                    "a V-bit or chamfer mill cannot finish in Z levels; use a parallel pass",
                );
            }
            StrategySpec::Flat { .. } => {
                return Some("a V-bit or chamfer mill cannot finish a flat");
            }
            StrategySpec::Pocket { .. } => {
                return Some("a V-bit or chamfer mill cannot clear a pocket's floor flat");
            }
            StrategySpec::Face { .. } => {
                return Some("a V-bit or chamfer mill cannot face flat");
            }
            StrategySpec::Profile { side, .. } if side != ProfileSide::On => {
                return Some(
                    "a V-bit or chamfer mill profiles only on the line: its width changes \
                     with depth",
                );
            }
            _ => {}
        }
        if leaves {
            return Some("a V-bit or chamfer mill cuts to size: set stock to leave to zero");
        }
        None
    }

    /// Why its strategy cannot cut with a tool of `tool`'s kind, if it
    /// cannot: each kind cuts what its shape cuts.
    fn cannot_use_kind(&self, tool: &ToolSpec) -> Option<&'static str> {
        let kind = tool.kind;
        let drilling = matches!(self.strategy, StrategySpec::Drill { .. });
        // Bored and milled with the side of an end mill, which a point, a tap
        // or a head on a neck has not.
        if matches!(
            self.strategy,
            StrategySpec::Bore { .. } | StrategySpec::Circular { .. }
        ) && (kind.is_pointed()
            || kind.undercuts()
            || matches!(kind, ToolKind::Tap | ToolKind::Tapered | ToolKind::FaceMill))
        {
            return Some("bore and mill holes with an end mill: it cuts with its side");
        }
        if kind == ToolKind::Drill && !drilling {
            return Some("a drill only drills; use a drilling operation");
        }
        if let StrategySpec::Drill {
            pitch, hand, spot, ..
        } = self.strategy
        {
            return self.cannot_drill(tool, pitch.map(|p| (p, hand)), spot);
        }
        if kind == ToolKind::Tap {
            return Some("a tap only taps; use a drilling operation that taps");
        }
        // Its width at its tip and its point: neither is a side.
        if matches!(kind, ToolKind::SpotDrill | ToolKind::Countersink)
            && !matches!(
                self.strategy,
                StrategySpec::Chamfer { .. } | StrategySpec::Engrave { .. }
            )
        {
            return Some(
                "a spot drill or countersink drills, spots and chamfers; use a drilling, \
                 chamfer or engraving operation",
            );
        }
        // A head under an overhang, at one height: a 2D profile or a slot,
        // straight to its depth, entered outside the material.
        if kind.undercuts() {
            if !matches!(
                self.strategy,
                StrategySpec::Profile { .. } | StrategySpec::Slot { .. }
            ) {
                return Some(
                    "a lollipop or slot cutter undercuts at one height: use it in a 2D \
                 profile or a slot",
                );
            }
            if self.stepdown().is_some() {
                return Some(
                    "a lollipop or slot cutter cuts at one height: turn multiple depths off \
                 and set the bottom where it cuts",
                );
            }
        }
        if kind == ToolKind::FaceMill
            && !matches!(
                self.strategy,
                StrategySpec::Face { .. } | StrategySpec::Flat { .. }
            )
        {
            return Some("a face mill faces: use it in facing or flat finishing");
        }
        // Its width changes with depth: a finish in 3D, where the drop
        // finds where its flank touches, or an engraving.
        if kind == ToolKind::Tapered
            && !matches!(
                self.strategy,
                StrategySpec::Parallel { .. }
                    | StrategySpec::Scallop { .. }
                    | StrategySpec::Contour { .. }
                    | StrategySpec::Spiral { .. }
                    | StrategySpec::Radial { .. }
                    | StrategySpec::MorphedSpiral { .. }
                    | StrategySpec::Engrave { .. }
            )
        {
            return Some(
                "a tapered mill finishes in 3D, or engraves: its width changes with depth",
            );
        }
        let round_tip = kind == ToolKind::Cone && tool.corner > 0.0;
        // Its depth is the V's, from how wide the area is: no other shape
        // cuts its width to its depth.
        if matches!(self.strategy, StrategySpec::VCarve { .. })
            && (kind != ToolKind::Cone || round_tip)
        {
            return Some(
                "V-carving cuts with a sharp V-bit: its width follows how deep its V goes",
            );
        }
        // A slot is as wide as the tool, all the way down.
        if matches!(self.strategy, StrategySpec::Slot { .. }) && kind.is_cone() {
            return Some(
                "a slot is cut with an end mill its width: a V-bit's width changes with \
                 depth; engrave with it instead",
            );
        }
        // The chamfer is the cone's flank: a cone with flank enough for it.
        if let StrategySpec::Chamfer {
            width, tip_offset, ..
        } = self.strategy
        {
            if !kind.is_cone() || round_tip {
                return Some(
                    "deburr with a chamfer mill or a V-bit, a spot drill or a countersink: \
                     the chamfer is its flank",
                );
            }
            let angle = tool.point_angle();
            let half = (angle.unwrap_or(f64::NAN) / 2.0).to_radians();
            let place = crate::strategy::chamfer::Place::new(
                width,
                tip_offset,
                half,
                tool.tip_diameter / 2.0,
                tool.diameter / 2.0,
            );
            if place.is_err() {
                return Some(
                    "the chamfer needs more of the cone than the tool has: make it narrower, \
                     the tip offset less, or take a wider tool",
                );
            }
        }
        if matches!(self.strategy, StrategySpec::Face { .. })
            && tool.flat_diameter().is_none()
            && !kind.is_cone()
        {
            return Some(
                "facing needs a flat end: use a face mill, or a flat or bull-nose end mill",
            );
        }
        None
    }

    /// Why drilling, tapping at `tap` (its pitch and hand) or spotting to
    /// `spot` across, cannot be done with `tool`, if it cannot.
    fn cannot_drill(
        &self,
        tool: &ToolSpec,
        tap: Option<(f64, crate::job::ThreadHand)>,
        spot: Option<f64>,
    ) -> Option<&'static str> {
        let kind = tool.kind;
        if let Some((pitch, hand)) = tap {
            return match kind {
                ToolKind::Tap if tool.pitch.is_some_and(|p| (p - pitch).abs() > 1e-6) => {
                    Some("the tap's pitch is not the one the operation taps at")
                }
                ToolKind::Tap if tool.hand != hand => {
                    Some("the tap's thread turns the other way from the one the operation taps")
                }
                // Documents from before taps were a kind tapped with a flat
                // end mill or a drill its size; they still do, and the run
                // says to make it a tap.
                ToolKind::Tap | ToolKind::Flat | ToolKind::Drill => None,
                _ => Some("tap with a tap"),
            };
        }
        if kind == ToolKind::Tap {
            return Some("a tap only taps: set the drilling to tap at its pitch");
        }
        if let Some(d) = spot {
            return tool.spot_depth(d).is_none().then_some(
                "spot to a diameter with a spot drill, countersink or drill whose point is \
                 that wide",
            );
        }
        (!matches!(
            kind,
            ToolKind::Drill | ToolKind::Flat | ToolKind::SpotDrill | ToolKind::Countersink
        ))
        .then_some("drill holes with a drill, a spot drill, a countersink, or a flat end mill that plunges")
    }

    /// Copy the values `preset` has into this operation, leaving the rest, and
    /// record where they came from.
    pub fn apply_preset(&mut self, preset: &Preset) {
        if let Some(rpm) = preset.rpm {
            self.passes.rpm = Some(rpm);
        }
        if let Some(feed) = preset.feed {
            self.passes.feed = feed;
        }
        if let Some(plunge) = preset.plunge_feed {
            self.linking.plunge_feed = plunge;
        }
        if let Some(ramp) = preset.ramp_feed {
            self.linking.ramp_feed = Some(ramp);
        }
        // A preset's stepover is a slotting or finishing one: an adaptive
        // operation's is its load, a fraction of it.
        if let Some(stepover) = preset.stepover
            && !self.strategy.uses().load
        {
            self.passes.stepover = stepover;
        }
        if let Some(d) = preset.stepdown {
            // Multiple depths stay as they were chosen, on or off.
            let depths = self.strategy.uses().depths;
            match (&mut self.strategy, depths) {
                // A tap does not peck.
                (StrategySpec::Drill { pitch: Some(_), .. }, _) => {}
                (StrategySpec::Drill { peck, .. }, _) => *peck = Some(d),
                (_, Depths::Required) => self.passes.stepdown = Some(d),
                (_, Depths::Optional) if self.passes.stepdown.is_some() => {
                    self.passes.stepdown = Some(d)
                }
                _ => {}
            }
        }
        if let Some(s) = preset.stock_to_leave {
            self.passes.stock_to_leave = s;
        }
        self.preset = Some(preset.name.clone());
    }
}
