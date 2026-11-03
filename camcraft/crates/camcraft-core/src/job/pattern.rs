//! A pattern on an operation: its toolpath repeated in rows, round a centre,
//! and mirrored, before it is linked. Fusion's Pattern, and its Mirror.

use serde::{Deserialize, Serialize};

use super::is_default;

/// Copies of an operation's toolpath, in the setup's frame. Each part is
/// optional; given together, the rows are made first, each of them copied
/// round the centre, and all of them mirrored, the originals kept.
#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pattern {
    /// Rows along X and Y.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linear: Option<Linear>,
    /// Round a centre.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polar: Option<Polar>,
    /// Mirrored, the original kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<Mirror>,
}

/// Copies in rows: `x_count` along X, `x_spacing` apart, and of those,
/// `y_count` along Y, `y_spacing` apart. The original is the first.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Linear {
    /// How many along X, the original among them.
    #[serde(default = "one")]
    pub x_count: u32,
    /// How far apart along X, in millimetres; negative to go the other way.
    #[serde(default, skip_serializing_if = "is_default")]
    pub x_spacing: f64,
    /// How many along Y.
    #[serde(default = "one")]
    pub y_count: u32,
    /// How far apart along Y.
    #[serde(default, skip_serializing_if = "is_default")]
    pub y_spacing: f64,
}

/// `count` copies round `centre`, spread evenly over `angle` degrees
/// counter-clockwise from the original: a whole turn, `count` apart round
/// it; less, the first and last `angle` apart.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Polar {
    /// How many, the original among them.
    pub count: u32,
    /// Over how far, in degrees.
    #[serde(default = "full_turn")]
    pub angle: f64,
    /// About where, in the setup's frame.
    #[serde(default, skip_serializing_if = "is_default")]
    pub centre: [f64; 2],
}

/// The toolpath mirrored about a line along the setup's X or Y through a
/// point, the original kept.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Mirror {
    /// Along which axis the line runs.
    pub axis: MirrorAxis,
    /// A point it goes through, in the setup's frame.
    #[serde(default, skip_serializing_if = "is_default")]
    pub through: [f64; 2],
}

/// Which of the setup's axes a mirror's line runs along.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MirrorAxis {
    /// Along X: Y is turned over.
    #[default]
    X,
    /// Along Y: X is turned over.
    Y,
}

/// The most copies a pattern makes, so a slip of a count cannot make a run
/// that never ends.
pub const MOST_COPIES: u32 = 1000;

impl Pattern {
    /// Whether it makes any copy.
    pub fn repeats(&self) -> bool {
        self.copies() > 1
    }

    /// How many toolpaths it makes, the original among them.
    pub fn copies(&self) -> u32 {
        let rows = self
            .linear
            .map_or(1, |l| l.x_count.max(1).saturating_mul(l.y_count.max(1)));
        let round = self.polar.map_or(1, |p| p.count.max(1));
        let mirrored = if self.mirror.is_some() { 2 } else { 1 };
        rows.saturating_mul(round).saturating_mul(mirrored)
    }
}

fn one() -> u32 {
    1
}

fn full_turn() -> f64 {
    360.0
}
