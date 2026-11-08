//! A job's cutters: the physical tool, its speeds and feeds, and the checks that
//! it is one.

use serde::{Deserialize, Serialize};

use super::{StockToLeave, ThreadHand, is_default};
use crate::cutter::tool::{
    Ball, Bullnose, BullnoseFamily, Cone, Cutter, Flat, Tapered, ToolError, ToolProfile, Undercut,
};
use crate::is_positive;
use camcraft_sim::stock::{Body, Ring};

/// A cutter.
///
/// The engine cuts with `kind`, `diameter` and `corner`. Everything after them
/// describes the physical tool the way Fusion's tool library does, for the person
/// setting it up and, later, for holder collision in playback and the machine's
/// tool table. All of it is optional, so a job that names only the cutting
/// geometry is complete.
///
/// Its speeds and feeds are [`Preset`]s: named sets an operation copies from.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSpec {
    /// Shape.
    pub kind: ToolKind,
    /// Diameter in millimetres.
    pub diameter: f64,
    /// Corner radius, for a bullnose, a tapered mill (half its diameter for
    /// a tapered ball), a face mill's inserts, and a V-bit's round tip, an
    /// engraving tool's.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub corner: f64,
    /// Included angle in degrees, for a cone: 90 for a 90° V-bit; a spot
    /// drill's, countersink's and drill's point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle: Option<f64>,
    /// Diameter of a cone's flat tip; zero comes to a point.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tip_diameter: f64,
    /// A tapered mill's taper, or a face mill's lead, in degrees from the
    /// axis, each side: 0 is square, 45 a 45° face mill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taper_angle: Option<f64>,
    /// The neck a lollipop's or slot cutter's head stands on, in
    /// millimetres: narrower than the head, which reaches in under what
    /// overhangs it by the difference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neck_diameter: Option<f64>,
    /// A tap's thread pitch, in millimetres a turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch: Option<f64>,
    /// Which way a tap's thread turns.
    #[serde(default, skip_serializing_if = "is_default")]
    pub hand: ThreadHand,
    /// How long a tap's chamfer is, its lead-in at the tip, in millimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chamfer_length: Option<f64>,
    /// How far a thread mill's lowest tooth is above its end, in millimetres:
    /// its tip, below the teeth. The tooth cuts the thread; the end is where
    /// its path is.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tip_length: f64,
    /// The finest and coarsest pitch a thread mill's teeth cut, in
    /// millimetres a turn; any when not given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pitch_range: Option<[f64; 2]>,
    /// A thread mill's rows of teeth, a pitch apart: one for a single-form
    /// mill, more for a multi-form one, which cuts that many turns of the
    /// thread each time round. One when not given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub teeth: Option<u32>,
    /// Tool number in the program.
    #[serde(default = "one_u32")]
    pub number: u32,
    /// Spindle speed.
    #[serde(default = "default_rpm")]
    pub rpm: u32,
    /// Description for the operator.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    /// Cutting edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flutes: Option<u32>,
    /// Length of the cutting edges, in millimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flute_length: Option<f64>,
    /// Length from the tip to where the shank reaches full diameter, in millimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shoulder_length: Option<f64>,
    /// Shank diameter, in millimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shank_diameter: Option<f64>,
    /// Overall length, in millimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overall_length: Option<f64>,
    /// Length out of the holder, tip to holder face, in millimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stickout: Option<f64>,
    /// What the cutter is made of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<ToolMaterial>,
    /// Maker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    /// Maker's product code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// What holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<Holder>,
    /// Which way it turns.
    #[serde(default, skip_serializing_if = "is_default")]
    pub spindle: SpindleDirection,
    /// Coolant while it cuts.
    #[serde(default, skip_serializing_if = "is_default")]
    pub coolant: Coolant,
    /// Named speeds and feeds, the first being the one a new operation gets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presets: Vec<Preset>,
}

/// Named speeds and feeds for a tool: "Roughing", "Finishing", "MDF", whatever
/// the person calls it. Nothing reads the name; an operation copies the values.
///
/// Every value is optional, and choosing a preset sets only the values it has,
/// so a "Finishing" preset can hold a stepover and stock to leave and keep the
/// operation's feeds.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    /// What the person calls it.
    pub name: String,
    /// A note, e.g. where the numbers came from.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    /// Spindle speed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpm: Option<u32>,
    /// Cutting feed, in mm/min.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feed: Option<f64>,
    /// Plunge feed, in mm/min.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plunge_feed: Option<f64>,
    /// Ramp feed, in mm/min.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ramp_feed: Option<f64>,
    /// Depth of each slice, in millimetres, for strategies that slice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stepdown: Option<f64>,
    /// Distance between passes, in millimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stepover: Option<f64>,
    /// Material to leave.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stock_to_leave: Option<StockToLeave>,
}

/// `base`, or `base 2`, `base 3`… if one of `presets` already has that name.
pub fn free_preset_name(presets: &[Preset], base: &str) -> String {
    let taken = |n: &str| presets.iter().any(|p| p.name == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|k| format!("{base} {k}"))
        .find(|n| !taken(n))
        .unwrap_or_default()
}

impl Default for ToolSpec {
    /// A 6 mm flat end mill: the commonest router cutter, and a sensible start.
    fn default() -> Self {
        Self {
            kind: ToolKind::Flat,
            diameter: 6.0,
            corner: 0.0,
            angle: None,
            tip_diameter: 0.0,
            taper_angle: None,
            neck_diameter: None,
            pitch: None,
            hand: ThreadHand::Right,
            chamfer_length: None,
            tip_length: 0.0,
            pitch_range: None,
            teeth: None,
            number: one_u32(),
            rpm: default_rpm(),
            comment: String::new(),
            flutes: None,
            flute_length: None,
            shoulder_length: None,
            shank_diameter: None,
            overall_length: None,
            stickout: None,
            material: None,
            vendor: None,
            product: None,
            holder: None,
            spindle: SpindleDirection::default(),
            coolant: Coolant::default(),
            presets: Vec::new(),
        }
    }
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// Which way a spindle turns, seen from above.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SpindleDirection {
    /// Clockwise: every ordinary right-hand cutter.
    #[default]
    Clockwise,
    /// Counter-clockwise, for a left-hand cutter.
    CounterClockwise,
}

/// Coolant while a tool cuts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Coolant {
    /// None: dry, as a router usually cuts.
    #[default]
    Off,
    /// Flood.
    Flood,
    /// Mist.
    Mist,
}

/// What a cutter is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolMaterial {
    /// Solid carbide.
    Carbide,
    /// High-speed steel.
    Hss,
}

/// A tool holder, as a stack of cylinders from the holder face upward.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Holder {
    /// Its name, e.g. "ER20 collet chuck".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Cylinders, from the face that meets the tool upward.
    #[serde(default)]
    pub segments: Vec<HolderSegment>,
}

/// One piece of a holder: a cylinder, or a cone where its top is given and
/// differs from its foot, as Fusion's segments have a lower and an upper
/// diameter.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HolderSegment {
    /// Diameter at its foot, the end nearer the tool, in millimetres.
    pub diameter: f64,
    /// Length, in millimetres.
    pub length: f64,
    /// Diameter at its top, in millimetres; its foot's when not given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_diameter: Option<f64>,
}

impl HolderSegment {
    /// A cylinder `diameter` across and `length` long.
    pub fn cylinder(diameter: f64, length: f64) -> Self {
        Self {
            diameter,
            length,
            top_diameter: None,
        }
    }

    /// Its diameter at its top.
    pub fn top(&self) -> f64 {
        self.top_diameter.unwrap_or(self.diameter)
    }
}

/// A part of a tool that does not cut: what stands above the flutes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece {
    /// The cutter itself above its flute length, up to its shank.
    Body,
    /// The shank, up to the holder.
    Shank,
    /// One segment of the holder, from the one at the tool.
    Holder(usize),
}

/// What of a tool stands above its flutes, as the simulator checks it: its
/// rings, each the [`Piece`] it is, and where the holder starts.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AboveFlutes {
    /// Its rings, from the tip upward.
    pub body: Body,
    /// What each ring is.
    pub pieces: Vec<Piece>,
    /// How far the holder's face is from the tip, when there is a holder
    /// checked.
    pub stickout: Option<f64>,
    /// What could not be checked, and why.
    pub unchecked: Vec<String>,
}

/// A tool field that is not physically meaningful, and why.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{field}: {why}")]
pub struct ToolFieldError {
    /// Which field, as it is named in the file: `presets.feed` for a preset's.
    pub field: &'static str,
    /// Which preset, by position, for a preset's field.
    pub preset: Option<usize>,
    /// What is wrong with it.
    pub why: String,
}

fn one_u32() -> u32 {
    1
}

fn default_rpm() -> u32 {
    18000
}

/// Cutter shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolKind {
    /// Square end.
    Flat,
    /// Ball nose.
    Ball,
    /// Corner radius.
    Bullnose,
    /// A tapered end mill: `diameter` across its tip, its flank widening at
    /// its taper angle up its flute length; a tapered ball where its corner
    /// is half its diameter.
    Tapered,
    /// A face mill: inserts `diameter` across at the face, their corner
    /// radius, and their lead angle, their edge widening that far from the
    /// axis up their cutting height (its flute length).
    FaceMill,
    /// A lollipop: a ball `diameter` across on a narrower neck, which
    /// reaches in under what overhangs it.
    Lollipop,
    /// A slot (key, T-slot) cutter: a disc `diameter` across and its flute
    /// length thick, on a narrower neck, cutting a groove into a wall.
    SlotCutter,
    /// A cone: a V-bit, or a chamfer mill with a flat tip; with a round
    /// tip (its corner), an engraving tool.
    Cone,
    /// A spot or centre drill: a cone its point angle, 90° unless given,
    /// that starts a hole, or chamfers one spotted to a diameter.
    SpotDrill,
    /// A countersink: a cone its point angle, 90° unless given, that cuts a
    /// countersunk head's seat.
    Countersink,
    /// A twist drill: its point is a cone, 118° unless its angle says otherwise.
    Drill,
    /// A tap: `diameter` its thread's, at its pitch and hand, its chamfer
    /// leading in from about the thread's minor diameter at the tip.
    Tap,
    /// A thread mill: teeth of the thread's form round a neck, `diameter`
    /// across their points, cutting a thread as it goes round a helix. It
    /// cuts only threads.
    ThreadMill,
    /// A touch probe: a ball `diameter` across on a stylus, which the
    /// controller stops when it touches. It cuts nothing, and the spindle
    /// does not turn it.
    Probe,
}

/// The point angle a drill has when none is given.
pub const DRILL_POINT: f64 = 118.0;

/// The point angle a spot drill or countersink has when none is given.
pub const SPOT_POINT: f64 = 90.0;

/// How deep an ISO metric thread is, radially, for each millimetre of its
/// pitch: `5/8 · H`, `H = √3/2 · p`, the internal thread's minor diameter
/// `D − 1.0825 p`.
const THREAD_DEPTH: f64 = 0.541_266;

impl ToolKind {
    /// Its name as a job file writes it.
    pub fn name(self) -> &'static str {
        match self {
            ToolKind::Flat => "flat",
            ToolKind::Ball => "ball",
            ToolKind::Bullnose => "bullnose",
            ToolKind::Tapered => "tapered",
            ToolKind::FaceMill => "face-mill",
            ToolKind::Lollipop => "lollipop",
            ToolKind::SlotCutter => "slot-cutter",
            ToolKind::Cone => "cone",
            ToolKind::SpotDrill => "spot-drill",
            ToolKind::Countersink => "countersink",
            ToolKind::Drill => "drill",
            ToolKind::Tap => "tap",
            ToolKind::ThreadMill => "thread-mill",
            ToolKind::Probe => "probe",
        }
    }

    /// Whether it is a cone that cuts with its flank: a V-bit or chamfer
    /// mill, a spot drill, a countersink.
    pub fn is_cone(self) -> bool {
        matches!(
            self,
            ToolKind::Cone | ToolKind::SpotDrill | ToolKind::Countersink
        )
    }

    /// Whether its head reaches in under what overhangs it.
    pub fn undercuts(self) -> bool {
        matches!(self, ToolKind::Lollipop | ToolKind::SlotCutter)
    }

    /// Whether it has a point that leaves a hole's top its width: a drill,
    /// and the cones that spot one.
    pub fn is_pointed(self) -> bool {
        matches!(
            self,
            ToolKind::Drill | ToolKind::SpotDrill | ToolKind::Countersink | ToolKind::Cone
        )
    }
}

impl ToolSpec {
    /// The radius of its corner, as Fusion's `tool_cornerRadius`: a ball's is
    /// half its diameter, a bullnose's its corner, any other's none.
    pub fn corner_radius(&self) -> f64 {
        match self.kind {
            ToolKind::Ball | ToolKind::Lollipop => self.diameter / 2.0,
            ToolKind::Bullnose | ToolKind::Tapered | ToolKind::FaceMill | ToolKind::Cone => {
                self.corner
            }
            ToolKind::Flat
            | ToolKind::SlotCutter
            | ToolKind::SpotDrill
            | ToolKind::Countersink
            | ToolKind::Drill
            | ToolKind::Tap
            | ToolKind::ThreadMill
            | ToolKind::Probe => 0.0,
        }
    }

    /// How wide the flat of its end is, for facing: a flat end mill's
    /// diameter, a bullnose's or face mill's less its corners; `None` for a
    /// tool with no flat end.
    pub fn flat_diameter(&self) -> Option<f64> {
        let flat = match self.kind {
            ToolKind::Flat => self.diameter,
            ToolKind::Bullnose | ToolKind::FaceMill => self.diameter - 2.0 * self.corner,
            _ => return None,
        };
        (flat > 0.0).then_some(flat)
    }

    /// The largest diameter it cuts: a tapered mill's or a face mill's at
    /// the top of its flutes, any other's its diameter.
    pub fn cutting_diameter(&self) -> f64 {
        match self.kind {
            ToolKind::Tapered | ToolKind::FaceMill => {
                self.build().map_or(self.diameter, |c| 2.0 * c.max_radius())
            }
            _ => self.diameter,
        }
    }

    /// How tall a lollipop's or slot cutter's head is, from its tip to its
    /// neck: a slot cutter's flute length, a lollipop's ball up to where it
    /// meets the neck.
    pub fn head_height(&self) -> Option<f64> {
        match self.kind {
            ToolKind::SlotCutter => self.flute_length,
            ToolKind::Lollipop => {
                let r = self.diameter / 2.0;
                let n = self.neck_diameter? / 2.0;
                (n < r).then(|| r + (r * r - n * n).sqrt())
            }
            _ => None,
        }
    }

    /// How far below a hole's top its point goes to leave it `diameter`
    /// across there: a spot to a diameter, or a countersink's seat. `None`
    /// for a tool with no point, or one too small or too blunt for it.
    pub fn spot_depth(&self, diameter: f64) -> Option<f64> {
        if !self.kind.is_pointed() || self.corner > 0.0 {
            return None;
        }
        let Ok(Cutter::Cone(cone)) = self.build() else {
            return None;
        };
        let r = diameter / 2.0;
        (r > cone.tip() && r <= cone.max_radius() + 1e-9)
            .then(|| cone.height_at(r.min(cone.max_radius())).unwrap_or(0.0))
    }

    /// Its shape in words, past its kind and diameter, for a setup sheet:
    /// its corner, angles, pitch and neck, as it has them.
    pub fn shape_words(&self) -> String {
        let n = |v: f64| {
            let s = format!("{v:.3}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        };
        let mut out = Vec::new();
        match self.kind {
            ToolKind::Tapered => {
                out.push(format!(
                    "tapered {}° a side",
                    self.taper_angle.map_or_else(|| "?".into(), n)
                ));
                if self.corner > 0.0 {
                    out.push(format!("tip r{}", n(self.corner)));
                }
            }
            ToolKind::FaceMill => {
                out.push(format!("lead {}°", n(self.taper_angle.unwrap_or(0.0))));
                if self.corner > 0.0 {
                    out.push(format!("inserts r{}", n(self.corner)));
                }
            }
            ToolKind::Lollipop | ToolKind::SlotCutter => {
                if let Some(neck) = self.neck_diameter {
                    out.push(format!("neck Ø{}", n(neck)));
                }
                if let Some(h) = self.head_height() {
                    out.push(format!("{} tall", n(h)));
                }
            }
            ToolKind::Cone | ToolKind::SpotDrill | ToolKind::Countersink => {
                let point = if self.kind == ToolKind::Cone {
                    self.angle
                } else {
                    Some(self.angle.unwrap_or(SPOT_POINT))
                };
                out.push(format!("{}°", point.map_or_else(|| "?".into(), n)));
                if self.corner > 0.0 {
                    out.push(format!("tip r{}", n(self.corner)));
                }
            }
            ToolKind::Tap => {
                out.push(match self.pitch {
                    Some(p) => format!("{} mm pitch", n(p)),
                    None => "no pitch".into(),
                });
                out.push(match self.hand {
                    ThreadHand::Right => "right-hand".into(),
                    ThreadHand::Left => "left-hand".into(),
                });
                if let Some(c) = self.chamfer_length {
                    out.push(format!("chamfer {}", n(c)));
                }
            }
            _ => {}
        }
        out.join(", ")
    }

    /// Its point's included angle, in degrees: its own, else a drill's 118°
    /// and a spot drill's or countersink's 90°; `None` for a V-bit not
    /// given one, or a tool with no point.
    pub fn point_angle(&self) -> Option<f64> {
        match self.kind {
            ToolKind::Drill => Some(self.angle.unwrap_or(DRILL_POINT)),
            ToolKind::SpotDrill | ToolKind::Countersink => Some(self.angle.unwrap_or(SPOT_POINT)),
            ToolKind::Cone => self.angle,
            _ => None,
        }
    }

    /// How far its tip is from the spindle's gauge line, in millimetres, as
    /// a length offset (`G43`) holds it: what sticks out of the holder (its
    /// stickout, else its overall length), and the holder's own length on
    /// top, its segments stacked, when it has one. Without a holder the
    /// gauge line is taken at the holder's face, as a router's collet nut
    /// nearly is. `None` when neither length is given.
    pub fn gauge_length(&self) -> Option<f64> {
        let out = self.stickout.or(self.overall_length)?;
        let holder = self
            .holder
            .as_ref()
            .map_or(0.0, |h| h.segments.iter().map(|s| s.length).sum());
        Some(out + holder)
    }

    /// How many turns of a thread a thread mill cuts each time round: its
    /// rows of teeth, one unless given.
    pub fn threads_per_turn(&self) -> u32 {
        self.teeth.unwrap_or(1).max(1)
    }

    /// Why a thread mill cannot cut a thread of `pitch`, if it cannot: the
    /// pitch is outside its range.
    pub fn pitch_problem(&self, pitch: f64) -> Option<String> {
        let [fine, coarse] = self.pitch_range?;
        (pitch < fine - 1e-9 || pitch > coarse + 1e-9).then(|| {
            format!("a {pitch} mm pitch is outside the thread mill's {fine} to {coarse} mm")
        })
    }

    /// What stands above its flutes, from the tip upward, as far as its
    /// lengths say:
    ///
    /// - past its flute length, the cutter itself, as wide as it cuts, up to
    ///   its shoulder where it has a shank of its own, else up to the holder;
    /// - its shank, from its shoulder (or its flutes) up to the holder;
    /// - its holder's segments, stacked from its stickout (else its overall
    ///   length) up.
    ///
    /// A thread mill's teeth stand on a neck narrower than they are, which
    /// goes into the hole behind them, so nothing is checked between its
    /// teeth and its shoulder (the neck's length); its shank from there up.
    ///
    /// Nothing is guessed: what its lengths do not place is left out, and
    /// said in [`AboveFlutes::unchecked`].
    pub fn above_flutes(&self) -> AboveFlutes {
        let mut out = AboveFlutes::default();
        let out_of_holder = self.stickout.or(self.overall_length);
        let top = out_of_holder.unwrap_or(f64::INFINITY);
        let thread_mill = self.kind == ToolKind::ThreadMill;
        let undercuts = self.kind.undercuts();
        let shank_from = if thread_mill || undercuts {
            self.shoulder_length
        } else {
            self.shoulder_length.or(self.flute_length)
        };
        let mut ring = |piece, from: f64, to: f64, below: f64, above: f64| {
            if to > from && below > 0.0 && above > 0.0 {
                out.body.rings.push(Ring {
                    from,
                    to,
                    radius_from: below / 2.0,
                    radius_to: above / 2.0,
                });
                out.pieces.push(piece);
            }
        };
        // Without a shank of its own the cutter is as wide all the way up.
        let body_to = |from: f64| match self.shank_diameter {
            Some(_) => shank_from.unwrap_or(from),
            None => top,
        };
        if undercuts {
            // The neck, from the head up: what reaches past an overhang
            // the head is under, so what meets it is a clash.
            match (self.head_height(), self.neck_diameter) {
                (Some(from), Some(neck)) => {
                    ring(Piece::Body, from, body_to(from).min(top), neck, neck);
                }
                _ => out
                    .unchecked
                    .push("the neck: the tool has no neck diameter or head height".into()),
            }
        } else if let Some(flutes) = self.flute_length.filter(|_| !thread_mill) {
            let wide = self.cutting_diameter();
            ring(Piece::Body, flutes, body_to(flutes).min(top), wide, wide);
        }
        match (self.shank_diameter, shank_from) {
            (Some(d), Some(from)) => ring(Piece::Shank, from, top, d, d),
            (Some(_), None) if thread_mill => out
                .unchecked
                .push("the shank: the thread mill has no shoulder (neck) length".into()),
            (Some(_), None) if undercuts => out
                .unchecked
                .push("the shank: the tool has no shoulder length, where its neck ends".into()),
            (Some(_), None) => out
                .unchecked
                .push("the shank: the tool has no flute or shoulder length".into()),
            _ => {}
        }
        if self.flute_length.is_none() && !thread_mill && !undercuts {
            out.unchecked
                .push("the cutter above its flutes: the tool has no flute length".into());
        }
        let segments = self.holder.as_ref().map_or(&[][..], |h| &h.segments[..]);
        if segments.is_empty() {
            out.unchecked.push("the holder: the tool has none".into());
        } else if let Some(at) = out_of_holder {
            out.stickout = Some(at);
            let mut from = at;
            for (i, s) in segments.iter().enumerate() {
                ring(Piece::Holder(i), from, from + s.length, s.diameter, s.top());
                from += s.length;
            }
        } else {
            out.unchecked
                .push("the holder: the tool has no stickout or overall length".into());
        }
        out
    }

    /// The preset called `name`.
    pub fn preset(&self, name: &str) -> Option<&Preset> {
        self.presets.iter().find(|p| p.name == name)
    }

    /// Every problem with the tool's fields, each against the field it concerns,
    /// so an editor can show them where they are.
    pub fn check(&self) -> Vec<ToolFieldError> {
        let mut out = Vec::new();
        let mut bad = |field, why: String| {
            out.push(ToolFieldError {
                field,
                preset: None,
                why,
            });
        };
        let tapers = matches!(self.kind, ToolKind::Tapered | ToolKind::FaceMill)
            && self.taper_angle.is_some_and(|a| a > 0.0);
        if self.kind == ToolKind::Cone && self.angle.is_none() {
            bad("angle", "a V-bit or chamfer mill needs its angle".into());
        } else if self.kind == ToolKind::Tapered && self.taper_angle.is_none() {
            bad("taper_angle", "a tapered mill needs its taper angle".into());
        } else if tapers && self.flute_length.is_none() {
            bad(
                "flute_length",
                "its flank widens up its flutes: it needs its flute length".into(),
            );
        } else if self.kind.undercuts() && self.neck_diameter.is_none() {
            bad(
                "neck_diameter",
                "a lollipop or slot cutter needs its neck's diameter".into(),
            );
        } else if self.kind == ToolKind::SlotCutter && self.flute_length.is_none() {
            bad(
                "flute_length",
                "a slot cutter needs its cutting height, its flute length".into(),
            );
        } else if self.kind == ToolKind::Tap && self.pitch.is_none() {
            bad("pitch", "a tap needs its thread's pitch".into());
        } else if let Err(e) = self.build() {
            let field = match e {
                ToolError::BadRadius(_) => "diameter",
                ToolError::BadAngle(_) => "angle",
                ToolError::BadTaper(_) => "taper_angle",
                ToolError::BadNeck { .. } => "neck_diameter",
                ToolError::BadTip { .. } if self.kind == ToolKind::Tap => "chamfer_length",
                ToolError::BadTip { .. } => "tip_diameter",
                ToolError::BadCorner { .. } | ToolError::ConeStock => "corner",
            };
            bad(field, e.to_string());
        }
        for (field, value) in [
            ("pitch", self.pitch),
            ("chamfer_length", self.chamfer_length),
        ] {
            if let Some(v) = value
                && !is_positive(v)
            {
                bad(field, format!("must be greater than zero, got {v}"));
            }
        }
        if self.number == 0 {
            // Most controllers read T0 as no tool, with no length offset: every
            // depth would be off by the tool's length.
            bad("number", "tool numbers start at 1: T0 is no tool".into());
        }
        if self.rpm == 0 {
            bad("rpm", "spindle speed must be greater than zero".into());
        }
        if self.kind == ToolKind::ThreadMill {
            if self.teeth == Some(0) {
                bad(
                    "teeth",
                    "a thread mill has at least one row of teeth".into(),
                );
            }
            if let Some([fine, coarse]) = self.pitch_range
                && !(is_positive(fine) && is_positive(coarse) && fine <= coarse)
            {
                bad(
                    "pitch_range",
                    format!(
                        "the finest pitch, {fine}, must be more than 0 and no more than the coarsest, {coarse}"
                    ),
                );
            }
            if !(self.tip_length >= 0.0 && self.tip_length.is_finite()) {
                bad(
                    "tip_length",
                    format!("must be 0 or more, got {}", self.tip_length),
                );
            }
        }
        if self.flutes == Some(0) {
            bad("flutes", "a cutter has at least one flute".into());
        }
        let lengths = [
            ("flute_length", self.flute_length),
            ("shoulder_length", self.shoulder_length),
            ("shank_diameter", self.shank_diameter),
            ("overall_length", self.overall_length),
            ("stickout", self.stickout),
        ];
        for (field, value) in lengths {
            if let Some(v) = value
                && !is_positive(v)
            {
                bad(field, format!("must be greater than zero, got {v}"));
            }
        }
        // Tip upward: the flutes end before the shoulder, the shoulder before the
        // holder, and what sticks out of the holder is part of the whole tool.
        let order = [
            ("flute_length", self.flute_length),
            ("shoulder_length", self.shoulder_length),
            ("stickout", self.stickout),
            ("overall_length", self.overall_length),
        ];
        let known: Vec<(&'static str, f64)> = order
            .iter()
            .filter_map(|(f, v)| v.map(|v| (*f, v)))
            .collect();
        let words = |field: &str| match field {
            "flute_length" => "the flute length",
            "shoulder_length" => "the shoulder length",
            "stickout" => "the stickout",
            _ => "the overall length",
        };
        for pair in known.windows(2) {
            if let [(shorter, a), (longer, b)] = pair
                && a > b
            {
                bad(
                    shorter,
                    format!("{a} mm is longer than {} ({b} mm)", words(longer)),
                );
            }
        }
        if let Some(h) = &self.holder {
            for (i, seg) in h.segments.iter().enumerate() {
                if !is_positive(seg.diameter)
                    || !is_positive(seg.length)
                    || seg.top_diameter.is_some_and(|d| !is_positive(d))
                {
                    bad(
                        "holder",
                        format!("segment {} needs a positive diameter and length", i + 1),
                    );
                }
            }
        }
        out.extend(self.check_presets());
        out
    }

    fn check_presets(&self) -> Vec<ToolFieldError> {
        let mut out = Vec::new();
        for (i, p) in self.presets.iter().enumerate() {
            let mut bad = |field, why: String| {
                out.push(ToolFieldError {
                    field,
                    preset: Some(i),
                    why,
                });
            };
            let name = p.name.trim();
            if name.is_empty() {
                bad("presets.name", "a preset needs a name".into());
            } else if self.presets.iter().take(i).any(|q| q.name.trim() == name) {
                bad(
                    "presets.name",
                    format!("there is already a preset '{name}'"),
                );
            }
            if p.rpm == Some(0) {
                bad(
                    "presets.rpm",
                    "spindle speed must be greater than zero".into(),
                );
            }
            for (field, v) in [
                ("presets.feed", p.feed),
                ("presets.plunge_feed", p.plunge_feed),
                ("presets.ramp_feed", p.ramp_feed),
                ("presets.stepdown", p.stepdown),
                ("presets.stepover", p.stepover),
            ] {
                if let Some(v) = v
                    && !is_positive(v)
                {
                    bad(field, format!("must be greater than zero, got {v}"));
                }
            }
            if let Some(v) = p.stock_to_leave.and_then(|s| s.unusable()) {
                bad(
                    "presets.stock_to_leave",
                    format!("stock to leave must be a number, got {v}"),
                );
            }
        }
        out
    }

    /// The cutter this describes.
    pub fn build(&self) -> Result<Cutter, ToolError> {
        let r = self.diameter / 2.0;
        let half = |a: f64| (a / 2.0).to_radians();
        match self.kind {
            ToolKind::Flat => Flat::new(r).map(Cutter::from),
            ToolKind::Ball => Ball::new(r).map(Cutter::from),
            ToolKind::Bullnose => Bullnose::new(r, self.corner).map(Cutter::from),
            ToolKind::Tapered | ToolKind::FaceMill => {
                let taper = self.taper_angle.unwrap_or(0.0);
                if taper == 0.0 {
                    return Bullnose::new(r, self.corner).map(Cutter::from);
                }
                // The tip is `diameter` across, its corner within it, as a
                // bullnose's; the flank, from where the corner meets it,
                // widens up the flutes to the full radius.
                let flat = r - self.corner;
                if flat.is_nan() || flat < 0.0 {
                    return Err(ToolError::BadCorner {
                        corner: self.corner,
                        radius: r,
                    });
                }
                let alpha = taper.to_radians();
                let (rt, zt) = (
                    flat + self.corner * alpha.cos(),
                    self.corner * (1.0 - alpha.sin()),
                );
                let up = (self.flute_length.unwrap_or(0.0) - zt).max(0.0);
                Tapered::new(rt + up * alpha.tan(), flat, self.corner, alpha).map(Cutter::from)
            }
            ToolKind::Lollipop | ToolKind::SlotCutter => {
                let head = match self.kind {
                    ToolKind::Lollipop => Ball::new(r)?.as_bullnose(),
                    _ => Flat::new(r)?.as_bullnose(),
                };
                let neck = self.neck_diameter.unwrap_or(f64::NAN) / 2.0;
                let height = self.head_height().unwrap_or(f64::NAN);
                Undercut::new(head, neck, height).map(Cutter::from)
            }
            ToolKind::Cone if self.corner > 0.0 => {
                let angle = self.angle.unwrap_or(0.0);
                Tapered::new(r, self.tip_diameter / 2.0, self.corner, half(angle))
                    .map_err(|e| match e {
                        ToolError::BadTaper(a) => ToolError::BadAngle(2.0 * a),
                        e => e,
                    })
                    .map(Cutter::from)
            }
            ToolKind::Cone => {
                let angle = self.angle.unwrap_or(0.0);
                Cone::new(r, self.tip_diameter / 2.0, half(angle)).map(Cutter::from)
            }
            ToolKind::SpotDrill | ToolKind::Countersink => {
                let angle = self.angle.unwrap_or(SPOT_POINT);
                Cone::new(r, self.tip_diameter / 2.0, half(angle)).map(Cutter::from)
            }
            ToolKind::Drill => {
                let angle = self.angle.unwrap_or(DRILL_POINT);
                Cone::new(r, 0.0, half(angle)).map(Cutter::from)
            }
            // Its chamfer, from about the thread's minor diameter at the tip
            // out to the full one its chamfer length up; square without one.
            ToolKind::Tap => match (self.pitch, self.chamfer_length) {
                (Some(p), Some(c)) if c > 0.0 && p > 0.0 => {
                    let tip = (r - THREAD_DEPTH * p).max(0.0);
                    Cone::new(r, tip, ((r - tip) / c).atan()).map(Cutter::from)
                }
                _ => Flat::new(r).map(Cutter::from),
            },
            // What the material sees, seen from above: a disc its teeth's
            // width, swept round the thread. A height field has no undercut,
            // so the groove the teeth cut in the wall is taken whole, as a
            // flat end mill its size takes it.
            ToolKind::ThreadMill => Flat::new(r).map(Cutter::from),
            // Its ball, where it touches: it never goes into the material,
            // so what it would sweep there is never asked.
            ToolKind::Probe => Ball::new(r).map(Cutter::from),
        }
    }
}
