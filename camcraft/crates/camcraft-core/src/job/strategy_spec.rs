//! Which strategy an operation uses, and the parameters only it has, with the
//! choices they are made from.

use serde::de::value::MapDeserializer;
use serde::{Deserialize, Serialize};

use super::{Depths, StockToLeave, is_default, is_yes, yes};

/// Strategy and the parameters only it has.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "strategy", rename_all = "kebab-case")]
pub enum StrategySpec {
    /// Raster finishing.
    Parallel {
        /// Pass direction in degrees; zero runs along +X.
        #[serde(default)]
        angle: f64,
    },
    /// Finishing passes a constant distance apart along the surface, offset in
    /// from the boundary, so the cusp between them is the same on a slope as on
    /// the flat. Fusion's Scallop.
    Scallop {
        /// Which way each pass cuts.
        #[serde(default, skip_serializing_if = "is_default")]
        direction: CutDirection,
        /// From the boundary in, or from the middle out.
        #[serde(default, skip_serializing_if = "is_default")]
        order: PassOrder,
        /// Only where the surface is at least this steep, in degrees from level.
        #[serde(default, skip_serializing_if = "is_default")]
        slope_from: f64,
        /// And at most this steep, in degrees from level.
        #[serde(default = "ninety", skip_serializing_if = "is_ninety")]
        slope_to: f64,
    },
    /// Passes round the part at levels a stepdown apart ([`Depths::Required`]),
    /// each where the tool touches the walls at its height, and one at the foot
    /// of every wall standing on a flat. Fusion's Contour: a waterline.
    Contour {
        /// Which way each pass cuts; climbing unless asked.
        #[serde(default = "climb", skip_serializing_if = "is_climb")]
        direction: CutDirection,
        /// Level by level, or each piece of the part all the way down.
        #[serde(default, skip_serializing_if = "is_default")]
        order: ContourOrder,
        /// Only where the surface is at least this steep, in degrees from level.
        #[serde(default, skip_serializing_if = "is_default")]
        slope_from: f64,
        /// And at most this steep, in degrees from level.
        #[serde(default = "ninety", skip_serializing_if = "is_ninety")]
        slope_to: f64,
    },
    /// Each flat face of the part cleared at its own height, rings a stepover
    /// apart in from its edges, and nothing else. Fusion's Flat.
    Flat {
        /// Which way each ring cuts; climbing unless asked.
        #[serde(default = "climb", skip_serializing_if = "is_climb")]
        direction: CutDirection,
        /// From the edges in, or from the middle out.
        #[serde(default, skip_serializing_if = "is_default")]
        order: PassOrder,
    },
    /// A spiral about a centre, a stepover further out each lap, dropped onto
    /// the part. Fusion's Spiral.
    Spiral {
        /// Its centre, in the setup's frame; the boundary's middle unless given.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        centre: Option<[f64; 2]>,
        /// Nothing nearer the centre than this, in millimetres.
        #[serde(default, skip_serializing_if = "is_default")]
        inner_radius: f64,
        /// Nothing further out than this, in millimetres; out to the whole
        /// boundary if zero.
        #[serde(default, skip_serializing_if = "is_default")]
        outer_radius: f64,
        /// Which way it turns: to climb, or to cut conventional.
        #[serde(default = "climb", skip_serializing_if = "is_climb")]
        direction: CutDirection,
        /// In from the outer radius, or out from the inner.
        #[serde(default, skip_serializing_if = "is_default")]
        order: PassOrder,
        /// Only where the surface is at least this steep, in degrees from level.
        #[serde(default, skip_serializing_if = "is_default")]
        slope_from: f64,
        /// And at most this steep, in degrees from level.
        #[serde(default = "ninety", skip_serializing_if = "is_ninety")]
        slope_to: f64,
    },
    /// Spokes out from a centre, no more than a stepover apart at the outer
    /// radius, dropped onto the part. Fusion's Radial.
    Radial {
        /// Its centre, in the setup's frame; the boundary's middle unless given.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        centre: Option<[f64; 2]>,
        /// Nothing nearer the centre than this, in millimetres.
        #[serde(default, skip_serializing_if = "is_default")]
        inner_radius: f64,
        /// Nothing further out than this, in millimetres; out to the whole
        /// boundary if zero.
        #[serde(default, skip_serializing_if = "is_default")]
        outer_radius: f64,
        /// The first spoke's angle, in degrees anticlockwise from +X.
        #[serde(default, skip_serializing_if = "is_default")]
        angle_from: f64,
        /// The last's: a whole turn round unless asked.
        #[serde(default = "full_turn", skip_serializing_if = "is_full_turn")]
        angle_to: f64,
        /// Out and back, or every spoke one way.
        #[serde(default, skip_serializing_if = "is_default")]
        direction: RadialDirection,
        /// Only where the surface is at least this steep, in degrees from level.
        #[serde(default, skip_serializing_if = "is_default")]
        slope_from: f64,
        /// And at most this steep, in degrees from level.
        #[serde(default = "ninety", skip_serializing_if = "is_ninety")]
        slope_to: f64,
    },
    /// A spiral that turns from the boundary's shape into a centre, its laps a
    /// stepover apart, dropped onto the part. Fusion's Morphed Spiral.
    MorphedSpiral {
        /// Where it turns into, in the setup's frame; the point furthest inside
        /// the boundary unless given.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        centre: Option<[f64; 2]>,
        /// Which way it turns: to climb, or to cut conventional.
        #[serde(default = "climb", skip_serializing_if = "is_climb")]
        direction: CutDirection,
        /// In from the boundary, or out from the centre.
        #[serde(default, skip_serializing_if = "is_default")]
        order: PassOrder,
        /// Only where the surface is at least this steep, in degrees from level.
        #[serde(default, skip_serializing_if = "is_default")]
        slope_from: f64,
        /// And at most this steep, in degrees from level.
        #[serde(default = "ninety", skip_serializing_if = "is_ninety")]
        slope_to: f64,
    },
    /// Z-level clearing by offsets, a level each stepdown ([`Depths::Required`]):
    /// each level's rings a stepover apart in from its edges, cut from the middle
    /// out, the one along the walls last. Fusion's 3D Pocket Clearing.
    PocketClearing {
        /// Rings shorter than this are not worth the entry.
        #[serde(default = "default_min_ring")]
        min_ring_length: f64,
        /// Cut conventional rather than climb.
        #[serde(default, skip_serializing_if = "is_default")]
        conventional: bool,
        /// Add a level on each flat face, so floors are left just the stock to
        /// leave down. Fusion's flat area detection.
        #[serde(default = "yes", skip_serializing_if = "is_yes")]
        flat_areas: bool,
    },
    /// Z-level clearing with the tool's load held constant: each pass steered
    /// to keep the arc of the tool in material at the optimal load, the
    /// stepover ([`Depths::Required`]). Fusion's 3D Adaptive Clearing.
    Adaptive {
        /// Which way the passes cut; climbing unless asked.
        #[serde(default = "climb", skip_serializing_if = "is_climb")]
        direction: CutDirection,
        /// Add a level on each flat face, so floors are left just the stock to
        /// leave down. Fusion's flat area detection.
        #[serde(default = "yes", skip_serializing_if = "is_yes")]
        flat_areas: bool,
    },
    /// A 2D contour: the tool run round the boundary, beside it or on it, level
    /// by level down to its floor. Fusion's 2D Contour.
    Profile {
        /// Which side of the boundary the tool cuts on.
        #[serde(default, skip_serializing_if = "is_default")]
        side: ProfileSide,
        /// Cut conventional rather than climb.
        #[serde(default, skip_serializing_if = "is_default")]
        conventional: bool,
    },
    /// A 2D pocket: the boundary cleared, from its middle out to its walls, level
    /// by level down to its floor. Fusion's 2D Pocket.
    Pocket {
        /// Cut conventional rather than climb.
        #[serde(default, skip_serializing_if = "is_default")]
        conventional: bool,
    },
    /// The stock's top cut flat, straight passes side by side across its
    /// footprint, down to the bottom: the model's top unless another is asked
    /// for. Fusion's Face.
    Face {
        /// Pass direction in degrees; zero runs along +X.
        #[serde(default, skip_serializing_if = "is_default")]
        angle: f64,
        /// Which way the passes cut.
        #[serde(default, skip_serializing_if = "is_default")]
        direction: FaceDirection,
        /// How far each pass runs on once the tool is clear of the stock, in
        /// millimetres.
        #[serde(default, skip_serializing_if = "is_default")]
        pass_extension: f64,
        /// How far past the stock's footprint is faced, in millimetres: Fusion's
        /// stock offset.
        #[serde(default, skip_serializing_if = "is_default")]
        stock_offset: f64,
    },
    /// Holes, picked by their rims, each drilled straight down, or tapped.
    Drill {
        /// Depth of each peck, backing out between; all at once when not given.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        peck: Option<f64>,
        /// Seconds to pause at the bottom, for a canned cycle that can.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dwell: Option<f64>,
        /// Between pecks, back off only this far to break the chip, rather
        /// than out of the hole: Fusion's chip breaking, and its chip break
        /// distance (`G73`). Only with a peck.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        chip_break: Option<f64>,
        /// Tap each hole, rather than drill it: the thread's pitch, in
        /// millimetres a turn. The tap is fed in at the pitch a turn of the
        /// spindle, which reverses at the bottom to feed it back out (`G84`,
        /// `G74` left-hand). No peck, chip breaking or dwell.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pitch: Option<f64>,
        /// Which way the thread turns, when tapping.
        #[serde(default, skip_serializing_if = "is_default")]
        hand: ThreadHand,
    },
    /// Holes bored with an end mill: down a helix, `pitch` a turn, round
    /// once at the bottom, then out to the wall in finishing passes there
    /// ([`Operation::finishing`](super::Operation::finishing)), each in and
    /// out along an arc. Climbing unless asked. Picked as a drill's holes
    /// are. Fusion's Bore ([`crate::strategy::holes::bore`]).
    Bore {
        /// How far down each turn of the helix goes, in millimetres.
        #[serde(default = "default_bore_pitch", skip_serializing_if = "is_bore_pitch")]
        pitch: f64,
        /// Cut conventional rather than climb.
        #[serde(default, skip_serializing_if = "is_default")]
        conventional: bool,
    },
    /// Holes and round bosses milled in circles, level by level a stepdown
    /// apart: a hole from its middle out to its wall, a boss from `boss_width`
    /// out from it in to it, a stepover apart, each level entered down a
    /// helix at the ramp angle and its last circle left along an arc. Picked
    /// as a drill's holes are. Fusion's Circular
    /// ([`crate::strategy::holes::circular`]).
    Circular {
        /// Cut conventional rather than climb.
        #[serde(default, skip_serializing_if = "is_default")]
        conventional: bool,
        /// How far out from a boss its material goes, in millimetres: what is
        /// cleared in from. One circle round it when zero.
        #[serde(default, skip_serializing_if = "is_default")]
        boss_width: f64,
    },
    /// Threads milled with a thread mill, in holes or on bosses: round a helix
    /// at the pitch, from the thread's bottom up or its top down as climbing
    /// and the hand say, in and out along arcs, in radial passes each a part
    /// of the thread's depth deeper, the last once more if asked. A
    /// multi-form mill cuts as many turns of the thread as it has rows of
    /// teeth each time round. Picked as a drill's holes are. Fusion's Thread
    /// ([`crate::strategy::holes::thread`]).
    Thread {
        /// Millimetres a turn.
        #[serde(
            default = "default_thread_pitch",
            skip_serializing_if = "is_thread_pitch"
        )]
        pitch: f64,
        /// Which way it turns.
        #[serde(default, skip_serializing_if = "is_default")]
        hand: ThreadHand,
        /// Its major diameter, in millimetres: a hole's, which is the hole
        /// plus the thread's depth either side, or a boss's own, when not
        /// given.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diameter: Option<f64>,
        /// How many radial passes the thread's depth is cut in. Not
        /// `passes`, which every operation's own are called beside it.
        #[serde(default = "one", skip_serializing_if = "is_one")]
        radial_passes: u32,
        /// The last pass cut twice, for a tool that springs from the wall.
        #[serde(default, skip_serializing_if = "is_default")]
        spring_pass: bool,
        /// Cut conventional rather than climb.
        #[serde(default, skip_serializing_if = "is_default")]
        conventional: bool,
    },
    /// The tool's centre run along curves and edges, open or closed, `depth`
    /// below the top: an engraving, a groove, a chamfer with a V-bit. Fusion's
    /// Trace and Engrave. Arcs are cut as arcs. Each curve ramps down along
    /// itself, round a closed one and back and forth along an open one, no
    /// more than a stepdown each time along it, so it never lifts between its
    /// levels; then once along it at the depth.
    Engrave {
        /// How deep it cuts below the top, in millimetres.
        #[serde(
            default = "default_engrave_depth",
            skip_serializing_if = "is_engrave_depth"
        )]
        depth: f64,
        /// Which way it runs along each curve.
        #[serde(default, skip_serializing_if = "is_default")]
        direction: TraceDirection,
        /// Each curve run the other way.
        #[serde(default, skip_serializing_if = "is_default")]
        reverse: bool,
    },
    /// A slot along curves and edges, open or closed: the tool's centre run
    /// along them `depth` below the top, as wide as the tool. Engraving's
    /// curves and cut ([`StrategySpec::Engrave`]), deeper and in levels a
    /// stepdown apart, both ways or always the same way. Arcs are cut as
    /// arcs. Fusion's Slot.
    Slot {
        /// How deep it cuts below the top, in millimetres.
        #[serde(default = "default_slot_depth", skip_serializing_if = "is_slot_depth")]
        depth: f64,
        /// Back and forth along it, or always the same way.
        #[serde(default, skip_serializing_if = "is_default")]
        pattern: SlotPattern,
        /// Each curve run the other way from how it was drawn.
        #[serde(default, skip_serializing_if = "is_default")]
        reverse: bool,
    },
    /// Edges chamfered, or deburred, with a chamfer mill or a V-bit: its
    /// flank run along each, its centre beside the edge away from the
    /// material, its tip deep enough that the chamfer is `width` wide, and
    /// the tip offset below that. Each edge from its own height. Fusion's 2D
    /// Chamfer ([`crate::strategy::chamfer`]).
    Chamfer {
        /// How wide the chamfer is, seen from above, in millimetres.
        #[serde(
            default = "default_chamfer_width",
            skip_serializing_if = "is_chamfer_width"
        )]
        width: f64,
        /// How far below the chamfer's foot the tip goes, so the flank cuts
        /// it and not the tip's corner, in millimetres.
        #[serde(default = "default_tip_offset", skip_serializing_if = "is_tip_offset")]
        tip_offset: f64,
        /// Which side of each edge the tool goes.
        #[serde(default, skip_serializing_if = "is_default")]
        side: ChamferSide,
    },
    /// Closed areas carved with a V-bit along their middle: at each point as
    /// deep as the V reaches the walls either side, so the width it cuts is the
    /// area's, a letter's stroke. Where the area is wider than the V reaches
    /// at `max_depth`, it cuts the walls to that depth and leaves the flat
    /// floor between them, and says so.
    VCarve {
        /// The deepest it cuts below the top, in millimetres: the V's own full
        /// depth, where it is as wide as the tool, when not given.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_depth: Option<f64>,
    },
    /// Lines of G-code of a person's own, written into the program where it
    /// stands in the setup, as Fusion's Manual NC: the operation's
    /// [`gcode`](super::Operation::gcode). It has no tool and no toolpath,
    /// and cuts nothing.
    CustomGcode {},
}

impl StrategySpec {
    /// The strategy a job file calls `name`, each of its parameters as a file
    /// that gives none has it: what a new operation of it starts from.
    pub fn named(name: &str) -> Option<Self> {
        let only = std::iter::once(("strategy", name));
        Self::deserialize(MapDeserializer::<_, serde::de::value::Error>::new(only)).ok()
    }

    /// The strategy's name as a job file writes it, which is also how a program's
    /// section comment names it.
    pub fn name(&self) -> &'static str {
        match self {
            StrategySpec::Parallel { .. } => "parallel",
            StrategySpec::Scallop { .. } => "scallop",
            StrategySpec::Contour { .. } => "contour",
            StrategySpec::Flat { .. } => "flat",
            StrategySpec::Spiral { .. } => "spiral",
            StrategySpec::Radial { .. } => "radial",
            StrategySpec::MorphedSpiral { .. } => "morphed-spiral",
            StrategySpec::PocketClearing { .. } => "pocket-clearing",
            StrategySpec::Adaptive { .. } => "adaptive",
            StrategySpec::Profile { .. } => "profile",
            StrategySpec::Pocket { .. } => "pocket",
            StrategySpec::Face { .. } => "face",
            StrategySpec::Drill { .. } => "drill",
            StrategySpec::Bore { .. } => "bore",
            StrategySpec::Circular { .. } => "circular",
            StrategySpec::Thread { .. } => "thread",
            StrategySpec::Engrave { .. } => "engrave",
            StrategySpec::VCarve { .. } => "v-carve",
            StrategySpec::Slot { .. } => "slot",
            StrategySpec::Chamfer { .. } => "chamfer",
            StrategySpec::CustomGcode {} => "custom-gcode",
        }
    }

    /// The thread a drilling operation taps, if it taps.
    pub fn tap(&self) -> Option<crate::strategy::drilling::Tap> {
        match *self {
            StrategySpec::Drill {
                pitch: Some(pitch),
                hand,
                ..
            } => Some(crate::strategy::drilling::Tap { pitch, hand }),
            _ => None,
        }
    }

    /// Whether it cuts conventional rather than climb, where that is a setting of
    /// its own. Facing's direction is its passes' own, and not asked here.
    pub fn conventional(&self) -> bool {
        match *self {
            StrategySpec::Profile { conventional, .. }
            | StrategySpec::Pocket { conventional }
            | StrategySpec::PocketClearing { conventional, .. }
            | StrategySpec::Bore { conventional, .. }
            | StrategySpec::Circular { conventional, .. }
            | StrategySpec::Thread { conventional, .. } => conventional,
            StrategySpec::Adaptive { direction, .. }
            | StrategySpec::Scallop { direction, .. }
            | StrategySpec::Contour { direction, .. }
            | StrategySpec::Flat { direction, .. }
            | StrategySpec::Spiral { direction, .. }
            | StrategySpec::MorphedSpiral { direction, .. } => {
                direction == CutDirection::Conventional
            }
            StrategySpec::Engrave { direction, .. } => direction == TraceDirection::Conventional,
            StrategySpec::Parallel { .. }
            | StrategySpec::Radial { .. }
            | StrategySpec::Face { .. }
            | StrategySpec::Drill { .. }
            | StrategySpec::VCarve { .. }
            | StrategySpec::Slot { .. }
            | StrategySpec::Chamfer { .. }
            | StrategySpec::CustomGcode {} => false,
        }
    }

    /// The range of slopes it keeps to, in degrees from level, where it has one.
    pub fn slopes(&self) -> Option<(f64, f64)> {
        match *self {
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
            | StrategySpec::Spiral {
                slope_from,
                slope_to,
                ..
            }
            | StrategySpec::Radial {
                slope_from,
                slope_to,
                ..
            }
            | StrategySpec::MorphedSpiral {
                slope_from,
                slope_to,
                ..
            } => Some((slope_from, slope_to)),
            _ => None,
        }
    }

    /// The stock to leave Fusion starts this strategy with: roughing leaves half
    /// a millimetre each way for the finish, and the rest cut to size.
    pub fn default_stock_to_leave(&self) -> StockToLeave {
        match self {
            StrategySpec::PocketClearing { .. } | StrategySpec::Adaptive { .. } => StockToLeave {
                radial: 0.5,
                axial: 0.5,
            },
            _ => StockToLeave::default(),
        }
    }

    /// The multiple depths this strategy starts with: roughing and 2D cutting
    /// 2 mm a level, facing its bottom in one pass.
    pub fn default_stepdown(&self) -> Option<f64> {
        match self.uses().depths {
            Depths::None => None,
            _ if matches!(self, StrategySpec::Face { .. }) => None,
            // Down to their depth in one, unless asked: engraving is shallow.
            _ if matches!(
                self,
                StrategySpec::Engrave { .. }
                    | StrategySpec::VCarve { .. }
                    | StrategySpec::Chamfer { .. }
            ) =>
            {
                None
            }
            // A slot is cut the tool's full width: half a 2D cut's.
            _ if matches!(self, StrategySpec::Slot { .. }) => Some(1.0),
            // A finishing waterline's, as Fusion's Contour.
            _ if matches!(self, StrategySpec::Contour { .. }) => Some(1.0),
            _ => Some(2.0),
        }
    }
}

/// Which way an engraving runs along each curve.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TraceDirection {
    /// Round each closed curve as cutting what it encloses climbs: clockwise
    /// seen from above, with a clockwise spindle. An open curve as drawn.
    #[default]
    Climb,
    /// Round each closed curve the other way. An open curve as drawn.
    Conventional,
    /// Every curve the way it was drawn.
    AsDrawn,
}

/// How a slot goes down to its depth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SlotPattern {
    /// Back and forth along it, as an engraving goes down: each time along
    /// it a stepdown deeper, ramping down as it goes, then once at the
    /// depth. It never lifts, and never plunges. Round and round a closed
    /// one.
    #[default]
    Bidirectional,
    /// Always the same way: a level a stepdown deeper each time along it,
    /// lifting at its end and going back to its start, entered as the
    /// linking says (ramping along it unless asked to plunge).
    Directional,
}

/// Which side of each edge a chamfer's tool goes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChamferSide {
    /// Away from the material, as the part either side of the edge says:
    /// inside a pocket's rim, outside an outline.
    #[default]
    Auto,
    /// The other side.
    Flipped,
}

/// Which way a thread turns: a tapped one's, or a milled one's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThreadHand {
    /// Right-hand: tapped in with the spindle turning clockwise (`G84`).
    #[default]
    Right,
    /// Left-hand: tapped in turning counter-clockwise (`G74`).
    Left,
}

impl ThreadHand {
    /// Which way the spindle turns to tap it in.
    pub fn spindle(self) -> super::SpindleDirection {
        match self {
            ThreadHand::Right => super::SpindleDirection::Clockwise,
            ThreadHand::Left => super::SpindleDirection::CounterClockwise,
        }
    }
}

/// Which way facing passes cut.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FaceDirection {
    /// Back and forth, one pass each way. Fusion's default.
    #[default]
    BothWays,
    /// Every pass climbing.
    Climb,
    /// Every pass conventional.
    Conventional,
}

/// Which way a strategy's passes cut, where it has the choice: scallop, contour,
/// flat, spiral and morphed spiral finishing, and adaptive clearing.
///
/// Each strategy says what its default is: scallop goes both ways, as Fusion's
/// does, and the rest climb.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CutDirection {
    /// Both ways. A finishing pass: closed rings climbing, and an open pass
    /// started at whichever end is nearer. Adaptive clearing: climbing, and
    /// cutting back conventional where a pass ends beside what is left, rather
    /// than going back to climb it.
    #[default]
    BothWays,
    /// Every pass climbing.
    Climb,
    /// Every pass conventional.
    Conventional,
}

impl CutDirection {
    /// The generators' own.
    pub fn engine(self) -> crate::strategy::adaptive::Direction {
        match self {
            CutDirection::BothWays => crate::strategy::adaptive::Direction::BothWays,
            CutDirection::Climb => crate::strategy::adaptive::Direction::Climb,
            CutDirection::Conventional => crate::strategy::adaptive::Direction::Conventional,
        }
    }
}

/// Which way round passes nested one in another are cut: scallop's, a flat's,
/// a spiral's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PassOrder {
    /// From the boundary in. Fusion's default.
    #[default]
    OutsideIn,
    /// Each region's innermost passes first, out to the boundary.
    InsideOut,
}

impl PassOrder {
    /// The generators' own.
    pub fn engine(self) -> crate::strategy::nesting::Order {
        match self {
            PassOrder::OutsideIn => crate::strategy::nesting::Order::OutsideIn,
            PassOrder::InsideOut => crate::strategy::nesting::Order::InsideOut,
        }
    }
}

/// Which order a contour's levels are cut in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContourOrder {
    /// Each level all the way round before the next down. Fusion's default.
    #[default]
    ByDepth,
    /// Each piece of the part all the way down before the next.
    ByArea,
}

impl ContourOrder {
    /// The generator's own.
    pub fn engine(self) -> crate::strategy::waterline::Order {
        match self {
            ContourOrder::ByDepth => crate::strategy::waterline::Order::ByDepth,
            ContourOrder::ByArea => crate::strategy::waterline::Order::ByArea,
        }
    }
}

/// Which way radial spokes are cut.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RadialDirection {
    /// Out and back in, one after the other. Fusion's default.
    #[default]
    BothWays,
    /// Every spoke from the centre out.
    Outward,
    /// Every spoke in to the centre.
    Inward,
}

impl RadialDirection {
    /// The generator's own.
    pub fn engine(self) -> crate::strategy::patterns::Spokes {
        match self {
            RadialDirection::BothWays => crate::strategy::patterns::Spokes::BothWays,
            RadialDirection::Outward => crate::strategy::patterns::Spokes::Outward,
            RadialDirection::Inward => crate::strategy::patterns::Spokes::Inward,
        }
    }
}

/// Which side of its boundary a profile cuts on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileSide {
    /// Outside it: the tool's side runs along the boundary from beyond, and what
    /// the boundary encloses is kept. Cutting a part out: its holes are cut from
    /// inside them.
    #[default]
    Outside,
    /// Inside it: what the boundary encloses is cut away along its edge. Cutting
    /// a hole out: anything left standing in it is cut round from outside.
    Inside,
    /// The tool's centre on it: a slot or a groove along it.
    On,
}

fn ninety() -> f64 {
    90.0
}

fn full_turn() -> f64 {
    360.0
}

// A signature serde's `skip_serializing_if` requires.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_full_turn(v: &f64) -> bool {
    (*v - 360.0).abs() < f64::EPSILON
}

fn climb() -> CutDirection {
    CutDirection::Climb
}

// A signature serde's `skip_serializing_if` requires.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_climb(d: &CutDirection) -> bool {
    *d == CutDirection::Climb
}

// A signature serde's `skip_serializing_if` requires.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_ninety(v: &f64) -> bool {
    (*v - 90.0).abs() < f64::EPSILON
}

/// A millimetre a turn: Fusion's bore's pitch.
fn default_bore_pitch() -> f64 {
    1.0
}

// A signature serde's `skip_serializing_if` requires.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_bore_pitch(v: &f64) -> bool {
    (*v - 1.0).abs() < f64::EPSILON
}

/// A millimetre a turn: an M6's coarse pitch.
fn default_thread_pitch() -> f64 {
    1.0
}

// A signature serde's `skip_serializing_if` requires.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_thread_pitch(v: &f64) -> bool {
    (*v - 1.0).abs() < f64::EPSILON
}

fn one() -> u32 {
    1
}

// A signature serde's `skip_serializing_if` requires.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_one(v: &u32) -> bool {
    *v == 1
}

/// Half a millimetre: deep enough to read, shallow enough for a fine V-bit
/// and thin stock.
fn default_engrave_depth() -> f64 {
    0.5
}

// A signature serde's `skip_serializing_if` requires.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_engrave_depth(v: &f64) -> bool {
    (*v - 0.5).abs() < f64::EPSILON
}

/// Three millimetres: a keyway's or a T-track's, in a few levels.
fn default_slot_depth() -> f64 {
    3.0
}

// A signature serde's `skip_serializing_if` requires.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_slot_depth(v: &f64) -> bool {
    (*v - 3.0).abs() < f64::EPSILON
}

/// Half a millimetre: an edge broken, as a deburr.
fn default_chamfer_width() -> f64 {
    0.5
}

// A signature serde's `skip_serializing_if` requires.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_chamfer_width(v: &f64) -> bool {
    (*v - 0.5).abs() < f64::EPSILON
}

/// A fifth of a millimetre: the tip's corner kept off the chamfer's foot.
fn default_tip_offset() -> f64 {
    0.2
}

// A signature serde's `skip_serializing_if` requires.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_tip_offset(v: &f64) -> bool {
    (*v - 0.2).abs() < f64::EPSILON
}

fn default_min_ring() -> f64 {
    3.0
}
