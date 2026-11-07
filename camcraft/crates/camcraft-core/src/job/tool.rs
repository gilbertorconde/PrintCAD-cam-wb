//! A job's cutters: the physical tool, its speeds and feeds, and the checks that
//! it is one.

use serde::{Deserialize, Serialize};

use super::{StockToLeave, is_default};
use crate::cutter::tool::{Ball, Bullnose, Cone, Cutter, Flat, ToolError};
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
    /// Corner radius, for a bullnose.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub corner: f64,
    /// Included angle in degrees, for a cone: 90 for a 90° V-bit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle: Option<f64>,
    /// Diameter of a cone's flat tip; zero comes to a point.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tip_diameter: f64,
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
    /// A cone: a V-bit, or a chamfer mill with a flat tip.
    Cone,
    /// A twist drill: its point is a cone, 118° unless its angle says otherwise.
    Drill,
    /// A thread mill: teeth of the thread's form round a neck, `diameter`
    /// across their points, cutting a thread as it goes round a helix. It
    /// cuts only threads.
    ThreadMill,
}

/// The point angle a drill has when none is given.
pub const DRILL_POINT: f64 = 118.0;

impl ToolSpec {
    /// The radius of its corner, as Fusion's `tool_cornerRadius`: a ball's is
    /// half its diameter, a bullnose's its corner, any other's none.
    pub fn corner_radius(&self) -> f64 {
        match self.kind {
            ToolKind::Ball => self.diameter / 2.0,
            ToolKind::Bullnose => self.corner,
            ToolKind::Flat | ToolKind::Cone | ToolKind::Drill | ToolKind::ThreadMill => 0.0,
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
        let shank_from = if thread_mill {
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
        if let Some(flutes) = self.flute_length.filter(|_| !thread_mill) {
            // Without a shank of its own the cutter is as wide all the way up.
            let to = match self.shank_diameter {
                Some(_) => shank_from.unwrap_or(flutes),
                None => top,
            };
            ring(
                Piece::Body,
                flutes,
                to.min(top),
                self.diameter,
                self.diameter,
            );
        }
        match (self.shank_diameter, shank_from) {
            (Some(d), Some(from)) => ring(Piece::Shank, from, top, d, d),
            (Some(_), None) if thread_mill => out
                .unchecked
                .push("the shank: the thread mill has no shoulder (neck) length".into()),
            (Some(_), None) => out
                .unchecked
                .push("the shank: the tool has no flute or shoulder length".into()),
            _ => {}
        }
        if self.flute_length.is_none() && !thread_mill {
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
        if self.kind == ToolKind::Cone && self.angle.is_none() {
            bad("angle", "a V-bit or chamfer mill needs its angle".into());
        } else if let Err(e) = self.build() {
            let field = match e {
                ToolError::BadRadius(_) => "diameter",
                ToolError::BadAngle(_) => "angle",
                ToolError::BadTip { .. } => "tip_diameter",
                ToolError::BadCorner { .. } | ToolError::ConeStock => "corner",
            };
            bad(field, e.to_string());
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
        match self.kind {
            ToolKind::Flat => Flat::new(r).map(Cutter::from),
            ToolKind::Ball => Ball::new(r).map(Cutter::from),
            ToolKind::Bullnose => Bullnose::new(r, self.corner).map(Cutter::from),
            ToolKind::Cone => {
                let angle = self.angle.unwrap_or(0.0);
                Cone::new(r, self.tip_diameter / 2.0, (angle / 2.0).to_radians()).map(Cutter::from)
            }
            ToolKind::Drill => {
                let angle = self.angle.unwrap_or(DRILL_POINT);
                Cone::new(r, 0.0, (angle / 2.0).to_radians()).map(Cutter::from)
            }
            // What the material sees, seen from above: a disc its teeth's
            // width, swept round the thread. A height field has no undercut,
            // so the groove the teeth cut in the wall is taken whole, as a
            // flat end mill its size takes it.
            ToolKind::ThreadMill => Flat::new(r).map(Cutter::from),
        }
    }
}
