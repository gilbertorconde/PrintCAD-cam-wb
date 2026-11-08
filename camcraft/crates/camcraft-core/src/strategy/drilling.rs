//! Drilling: holes from picked round edges, and a plunge down each.
//!
//! A hole is picked by its rim, the round edge where it meets the top of the part.
//! Pick its bottom rim as well and it is drilled to there; otherwise it is drilled
//! to the operation's bottom height. Each hole is a pass of its own: a link to just
//! above it, plunges down (in pecks, when asked, backing out to clear chips between
//! them, or only a little way to break them), and a retract. A tapped hole is fed
//! down and back out at the thread's pitch a turn. So the linker, simulator and
//! checks treat a drilled hole as they treat anything else, and a post that knows
//! canned cycles can write each hole as one line instead ([`Hole`] carries what it
//! needs).

use glam::{DVec2, DVec3, dvec3};

use crate::job::{HoleOrder, ThreadHand};
use crate::path::toolpath::{MoveRole, Toolpath};
use crate::setup::features::Chain;
use crate::strategy::nesting::nearest;
use crate::strategy::{StrategyError, check};

/// How far above a hole's top a drill starts feeding, and backs out to between
/// pecks: the R plane of a canned cycle.
pub const RETRACT_GAP: f64 = 2.0;

/// One hole to drill, in the setup's frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hole {
    /// Where its axis is.
    pub at: DVec2,
    /// The height it starts at: its rim.
    pub top: f64,
    /// The height the drill's tip goes down to.
    pub bottom: f64,
    /// Its radius, from the picked rim.
    pub radius: f64,
}

impl Hole {
    /// The height a drill feeds down from and backs out to: the R plane.
    pub fn retract(&self) -> f64 {
        self.top + RETRACT_GAP
    }
}

/// How each hole is drilled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrillParams {
    /// Depth of each peck; the whole depth at once when `None`.
    pub peck: Option<f64>,
    /// How far to back off between pecks to break the chip, rather than out
    /// of the hole; only with a peck.
    pub chip_break: Option<f64>,
    /// Feed down into the hole, in mm/min.
    pub feed: f64,
    /// Feed back out, in mm/min.
    pub retract_feed: f64,
    /// Tap each hole, rather than drill it: down and back out at `feed`, which
    /// is the thread's pitch a turn. No pecks.
    pub tap: Option<Tap>,
}

/// A thread tapped.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tap {
    /// Millimetres a turn.
    pub pitch: f64,
    /// Which way it turns.
    pub hand: ThreadHand,
}

impl Tap {
    /// The feed that keeps to the pitch at `rpm`, in mm/min.
    pub fn feed(&self, rpm: f64) -> f64 {
        self.pitch * rpm
    }
}

/// A round feature picked, or found by its diameter: a hole, or a round
/// boss, which is milled from outside.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Round {
    /// Where it is, its top and bottom, and its radius.
    pub hole: Hole,
    /// Whether it is a boss: the material inside its circle, not outside.
    pub boss: bool,
}

/// A round edge: its circle, in the setup's frame, and whether the round
/// face it bounds is a boss's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rim {
    /// Its centre.
    pub centre: DVec3,
    /// Its radius.
    pub radius: f64,
    /// Whether the material is inside it.
    pub boss: bool,
}

/// Why picked edges are not holes.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HoleError {
    /// Nothing was picked.
    #[error("pick the round edge at the top of each hole to drill")]
    NothingPicked,
    /// No hole of the models is as wide as asked.
    #[error("no hole of the setup's models is {diameter:.3} mm across along the tool")]
    NoneAsWide {
        /// The diameter asked for, in millimetres.
        diameter: f64,
    },
    /// A picked edge is not round.
    #[error("edge {edge} of '{model}' is not round, so it is not a hole's rim")]
    NotRound {
        /// The model.
        model: String,
        /// Its number there.
        edge: u32,
    },
    /// A picked rim is not level, so its hole does not run straight down.
    #[error("the hole at edge {edge} of '{model}' is not vertical in this setup")]
    NotVertical {
        /// The model.
        model: String,
        /// Its number there.
        edge: u32,
    },
    /// A picked face is not a round hole's or boss's wall standing along the
    /// tool.
    #[error("face {face} of '{model}' is not the round wall of a hole or a boss along the tool")]
    NotRoundFace {
        /// The model.
        model: String,
        /// Its number there.
        face: u32,
    },
    /// A boss was picked for what only holes have.
    #[error("the round at ({x:.3}, {y:.3}) is a boss, not a hole")]
    Boss {
        /// Where.
        x: f64,
        /// Where.
        y: f64,
    },
    /// A hole would be drilled upward, or not at all.
    #[error("the hole at ({x:.3}, {y:.3}) has its bottom at or above its top")]
    NoDepth {
        /// Where.
        x: f64,
        /// Where.
        y: f64,
    },
}

/// The holes picked rims make, in the order first picked: one per axis, from its
/// highest rim to its lowest, or to `bottom` when the rims picked of it are all
/// at one height, as both edges of a counterbore's floor are.
///
/// `circle` gives each chain's circle, centre and radius, in the setup's frame;
/// `tol` is how close two rims' axes must be to be one hole's, and two rims'
/// heights to be one.
pub fn holes(
    chains: &[Chain],
    circle: impl Fn(&Chain) -> Option<(DVec3, f64)>,
    bottom: f64,
    tol: f64,
) -> Result<Vec<Hole>, HoleError> {
    let rim = |c: &Chain| {
        circle(c).map(|(centre, radius)| Rim {
            centre,
            radius,
            boss: false,
        })
    };
    Ok(rounds(chains, rim, bottom, tol)?
        .into_iter()
        .map(|r| r.hole)
        .collect())
}

/// [`holes`], each a hole or a boss as `rim` says of its rims: a hole and a
/// boss on one axis, a tube's, are two.
pub fn rounds(
    chains: &[Chain],
    rim: impl Fn(&Chain) -> Option<Rim>,
    bottom: f64,
    tol: f64,
) -> Result<Vec<Round>, HoleError> {
    if chains.is_empty() {
        return Err(HoleError::NothingPicked);
    }
    // Each axis, with the heights of its rims and the widest radius.
    let mut axes: Vec<(DVec2, bool, Vec<f64>, f64)> = Vec::new();
    for chain in chains {
        let Some(Rim {
            centre,
            radius,
            boss,
        }) = rim(chain)
        else {
            return Err(HoleError::NotRound {
                model: chain.model.clone(),
                edge: chain.edge,
            });
        };
        let level = chain
            .points
            .iter()
            .all(|p| (p.z - centre.z).abs() <= tol.max(1e-6));
        if !level {
            return Err(HoleError::NotVertical {
                model: chain.model.clone(),
                edge: chain.edge,
            });
        }
        let at = centre.truncate();
        match axes
            .iter_mut()
            .find(|(a, b, _, _)| *b == boss && a.distance(at) <= tol)
        {
            Some((_, _, zs, r)) => {
                zs.push(centre.z);
                *r = r.max(radius);
            }
            None => axes.push((at, boss, vec![centre.z], radius)),
        }
    }
    axes.into_iter()
        .map(|(at, boss, zs, radius)| {
            let top = zs.iter().copied().fold(f64::MIN, f64::max);
            let low = zs.iter().copied().fold(f64::MAX, f64::min);
            let bottom = if top - low > tol.max(1e-6) {
                low
            } else {
                bottom
            };
            if bottom >= top {
                return Err(HoleError::NoDepth { x: at.x, y: at.y });
            }
            Ok(Round {
                hole: Hole {
                    at,
                    top,
                    bottom,
                    radius,
                },
                boss,
            })
        })
        .collect()
}

/// The order to take holes at `points` in, as `order` says, by their
/// places in it: two within `tol` of each other along X are in one column.
pub fn hole_order(points: &[DVec2], order: HoleOrder, tol: f64) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..points.len()).collect();
    let at = |i: usize| points.get(i).copied().unwrap_or_default();
    match order {
        HoleOrder::AsPicked => idx,
        HoleOrder::Nearest => {
            let mut out = Vec::with_capacity(idx.len());
            let Some(&first) = idx.first() else {
                return out;
            };
            let mut here = at(first);
            while !idx.is_empty() {
                let i = idx.remove(nearest(&idx, |&i| at(i).distance_squared(here)));
                here = at(i);
                out.push(i);
            }
            out
        }
        HoleOrder::XThenY => {
            idx.sort_by(|&a, &b| at(a).x.total_cmp(&at(b).x).then(a.cmp(&b)));
            // Columns: holes within the tolerance of the column's first along X.
            let mut columns: Vec<Vec<usize>> = Vec::new();
            for i in idx {
                match columns.last_mut() {
                    Some(c) if c.first().is_some_and(|&f| at(i).x - at(f).x <= tol) => c.push(i),
                    _ => columns.push(vec![i]),
                }
            }
            // Up the first column, down the next: no run back across.
            let mut out = Vec::with_capacity(points.len());
            for (k, mut c) in columns.into_iter().enumerate() {
                c.sort_by(|&a, &b| at(a).y.total_cmp(&at(b).y).then(a.cmp(&b)));
                if k % 2 == 1 {
                    c.reverse();
                }
                out.extend(c);
            }
            out
        }
    }
}

/// `holes` in drilling order: the first, then always the nearest not yet drilled.
/// Ties go to the earlier, so the order does not depend on anything but the holes.
pub fn nearest_first(holes: &[Hole]) -> Vec<Hole> {
    let points: Vec<DVec2> = holes.iter().map(|h| h.at).collect();
    hole_order(&points, HoleOrder::Nearest, 0.0)
        .into_iter()
        .filter_map(|i| holes.get(i).copied())
        .collect()
}

/// What a drilling operation did, for a post that writes canned cycles.
#[derive(Debug, Clone, PartialEq)]
pub struct DrillReport {
    /// The holes, in the order drilled.
    pub holes: Vec<Hole>,
    /// Depth of each peck, if it pecked.
    pub peck: Option<f64>,
    /// How far it backed off between pecks, if it broke the chip rather than
    /// backing out.
    pub chip_break: Option<f64>,
    /// Seconds to pause at the bottom, if asked.
    pub dwell: Option<f64>,
    /// The thread, if it tapped.
    pub tap: Option<Tap>,
    /// Feed down into each hole, in mm/min: a tap's, its pitch a turn.
    pub feed: f64,
}

/// A plunge down each hole, in order, each a pass the linker joins.
pub fn drill(holes: &[Hole], params: &DrillParams) -> Result<Toolpath, StrategyError> {
    for (name, v) in [("feed", params.feed), ("retract feed", params.retract_feed)] {
        check(name, v)?;
    }
    for (name, v) in [("peck", params.peck), ("chip break", params.chip_break)] {
        if let Some(v) = v {
            check(name, v)?;
        }
    }
    let mut path = Toolpath::new();
    for h in holes {
        let (x, y) = (h.at.x, h.at.y);
        let r = h.retract();
        path.feed_to(dvec3(x, y, r), MoveRole::Link, params.feed)?;
        if params.tap.is_some() {
            // In and back out at the pitch a turn, the spindle reversing at the
            // bottom: a tap backed out any faster strips the thread it cut.
            path.feed_to(dvec3(x, y, h.bottom), MoveRole::Plunge, params.feed)?;
            path.feed_to(dvec3(x, y, r), MoveRole::Retract, params.feed)?;
            continue;
        }
        let mut depth = h.top;
        loop {
            depth = match params.peck {
                Some(p) => (depth - p).max(h.bottom),
                None => h.bottom,
            };
            path.feed_to(dvec3(x, y, depth), MoveRole::Plunge, params.feed)?;
            if depth <= h.bottom {
                path.feed_to(dvec3(x, y, r), MoveRole::Retract, params.retract_feed)?;
                break;
            }
            if let Some(back) = params.chip_break {
                // Back off a little to break the chip, and on down from there.
                let to = (depth + back).min(r);
                path.feed_to(dvec3(x, y, to), MoveRole::Retract, params.retract_feed)?;
                continue;
            }
            path.feed_to(dvec3(x, y, r), MoveRole::Retract, params.retract_feed)?;
            // Back down to just above the last peck, and on from there.
            path.feed_to(
                dvec3(x, y, (depth + 0.5).min(r)),
                MoveRole::Plunge,
                params.feed,
            )?;
        }
    }
    Ok(path)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::indexing_slicing,
    clippy::cast_precision_loss
)]
mod tests;
