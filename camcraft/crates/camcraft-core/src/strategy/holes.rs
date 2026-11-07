//! Holes and round bosses milled with the side of a tool, as Fusion's hole
//! strategies mill them: bored down a helix ([`bore`]), cleared in circles
//! level by level ([`circular`]), and threaded round a helix at the thread's
//! pitch ([`thread`]).
//!
//! Every move round a hole is an arc about its axis, a helix where it goes
//! down or up as it turns, so the post writes each as `G2` or `G3` with a Z.
//! Into and out of a finishing circle or a thread the tool goes along a half
//! circle, tangent to it, from inside a hole or outside a boss, so it never
//! leaves a mark where it meets the wall. Each hole is one pass, from a link
//! to just above it to a retract out of it, as a drilled hole is, so the
//! linker, the simulation and the checks see it as they see anything else.
//!
//! Which way round each goes is the caller's: climbing inside a hole, with
//! the material outside the circle, goes the other way round from climbing
//! outside a boss.

use std::f64::consts::{PI, TAU};

use glam::{DVec2, DVec3, dvec2};

use crate::job::ThreadHand;
use crate::path::toolpath::{Arc, MoveRole, Toolpath};
use crate::strategy::drilling::Round;
use crate::strategy::{StrategyError, check};

/// What a hole operation milled: each hole and boss, in the order cut.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HoleReport {
    /// The holes and bosses.
    pub rounds: Vec<Round>,
}

/// Why a hole cannot be milled as asked.
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum MillError {
    /// The tool is too wide for the hole.
    #[error(
        "the hole at ({x:.3}, {y:.3}) is {across:.3} mm across: too small for the tool and \
         what is left on its wall"
    )]
    TooSmall {
        /// Where.
        x: f64,
        /// Where.
        y: f64,
        /// How wide it is.
        across: f64,
    },
    /// The finishing passes take more than the hole has room for.
    #[error(
        "the finishing passes at ({x:.3}, {y:.3}) go further than the hole has room for: \
         fewer, or a smaller stepover"
    )]
    NoRoomToFinish {
        /// Where.
        x: f64,
        /// Where.
        y: f64,
    },
    /// A thread's diameter leaves it no depth.
    #[error("the thread at ({x:.3}, {y:.3}) is no wider than the hole it is cut in")]
    NoThread {
        /// Where.
        x: f64,
        /// Where.
        y: f64,
    },
    /// A thread has no length to it.
    #[error("the thread at ({x:.3}, {y:.3}) has no length above the tool's tip")]
    NoLength {
        /// Where.
        x: f64,
        /// Where.
        y: f64,
    },
    /// A strategy failure.
    #[error(transparent)]
    Strategy(#[from] StrategyError),
}

impl From<crate::path::toolpath::ToolpathError> for MillError {
    fn from(e: crate::path::toolpath::ToolpathError) -> Self {
        MillError::Strategy(e.into())
    }
}

fn too_small(r: &Round) -> MillError {
    MillError::TooSmall {
        x: r.hole.at.x,
        y: r.hole.at.y,
        across: 2.0 * r.hole.radius,
    }
}

/// `x`, a whole number at least one, as a count: no more than a million.
fn count(x: f64) -> u32 {
    let x = if x.is_finite() {
        x.clamp(1.0, 1.0e6)
    } else {
        1.0
    };
    // Clamped to a whole number in range first, so the narrowing is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = x.round() as u32;
    n
}

/// The unit vector at `angle` from +X.
fn ray(angle: f64) -> DVec2 {
    dvec2(angle.cos(), angle.sin())
}

/// The point `radius` from `centre` along `angle`, at `z`.
fn on(centre: DVec2, radius: f64, angle: f64, z: f64) -> DVec3 {
    (centre + ray(angle) * radius).extend(z)
}

/// Round the circle about `centre` the path is on, through `sweep` radians
/// (counter-clockwise when positive), going to `z` evenly as it turns: arcs
/// of no more than a turn each, a helix where `z` is not where it is. The
/// angle it ends at.
fn turn(
    path: &mut Toolpath,
    centre: DVec2,
    sweep: f64,
    z: f64,
    role: MoveRole,
    feed: f64,
) -> Result<f64, MillError> {
    let from = path.end().unwrap_or(centre.extend(z));
    let radius = (from.truncate() - centre).length();
    let start = (from.y - centre.y).atan2(from.x - centre.x);
    let n = count((sweep.abs() / TAU - 1e-9).ceil());
    let pieces = f64::from(n);
    for k in 1..=n {
        let t = f64::from(k) / pieces;
        let to = on(centre, radius, start + sweep * t, from.z + (z - from.z) * t);
        let arc = Arc {
            centre: centre.extend(0.0),
            axis: DVec3::Z,
            sweep: sweep / pieces,
        };
        path.arc_to(to, arc, role, feed)?;
    }
    Ok(start + sweep)
}

/// A half circle tangent to the circle of `radius` about `centre` at `angle`,
/// from the path's point on the same ray, inside a hole or outside a boss,
/// onto that circle, going to `z`; or, `out`, from the circle back to `away`
/// from the centre on the ray. `ccw` is the way round the circle is cut. A
/// half circle from inside turns the way the circle does; from outside, the
/// other way, so both meet it going its way.
#[allow(clippy::too_many_arguments)]
fn lead(
    path: &mut Toolpath,
    centre: DVec2,
    (radius, away): (f64, f64),
    angle: f64,
    ccw: bool,
    z: f64,
    out: bool,
    feed: f64,
) -> Result<(), MillError> {
    let u = ray(angle);
    let mid = centre + u * (radius + away) / 2.0;
    let turns_with = away < radius;
    let sense = if ccw == turns_with { 1.0 } else { -1.0 };
    let to = if out {
        (centre + u * away).extend(z)
    } else {
        (centre + u * radius).extend(z)
    };
    let arc = Arc {
        centre: mid.extend(0.0),
        axis: DVec3::Z,
        sweep: sense * PI,
    };
    path.arc_to(to, arc, MoveRole::Lead, feed)?;
    Ok(())
}

/// Straight to `to`, a plunge going down and a retract going up, in air.
fn straight(path: &mut Toolpath, to: DVec3, feeds: (f64, f64)) -> Result<(), MillError> {
    let Some(from) = path.end() else {
        return Ok(());
    };
    if from.distance(to) < 1e-9 {
        return Ok(());
    }
    let (role, feed) = if to.z < from.z - 1e-9 {
        (MoveRole::Plunge, feeds.0)
    } else if to.z > from.z + 1e-9 {
        (MoveRole::Retract, feeds.1)
    } else {
        (MoveRole::Lead, feeds.0)
    };
    path.feed_to(to, role, feed)?;
    Ok(())
}

/// How a hole is bored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoreParams {
    /// The tool's radius.
    pub tool_radius: f64,
    /// Stock left on the wall, and on the floor.
    pub leave: (f64, f64),
    /// How far down each turn goes.
    pub pitch: f64,
    /// Whether it goes counter-clockwise round the hole, seen from above.
    pub ccw: bool,
    /// Feed round the helix and the circles.
    pub feed: f64,
    /// Feed out of the hole.
    pub retract_feed: f64,
    /// Finishing passes at the bottom: how many, how far apart, their feed,
    /// and whether the last is cut twice.
    pub finishing: Option<(u32, f64, f64, bool)>,
}

/// Each hole bored: down a helix from just above it to its bottom, `pitch`
/// a turn, round once there to take the helix's slope off the floor, then
/// each finishing pass out a stepover nearer the wall, the last at it, each
/// in and out along a half circle; then straight up out of it.
///
/// The second of what it returns is the holes it leaves a core standing in:
/// wider than twice the tool, so the helix does not reach their middle.
pub fn bore(rounds: &[Round], p: &BoreParams) -> Result<(Toolpath, Vec<usize>), MillError> {
    check("pitch", p.pitch)?;
    check("feed", p.feed)?;
    check("retract feed", p.retract_feed)?;
    let sweep_sign = if p.ccw { 1.0 } else { -1.0 };
    let mut path = Toolpath::new();
    let mut cores = Vec::new();
    for (i, r) in rounds.iter().enumerate() {
        let h = &r.hole;
        let last = h.radius - p.tool_radius - p.leave.0;
        if last <= 1e-3 {
            return Err(too_small(r));
        }
        let (passes, stepover, finish_feed, repeat) =
            p.finishing.unwrap_or((0, 0.0, p.feed, false));
        let helix = last - f64::from(passes) * stepover;
        if helix <= 1e-3 {
            return Err(MillError::NoRoomToFinish {
                x: h.at.x,
                y: h.at.y,
            });
        }
        if helix > p.tool_radius {
            cores.push(i);
        }
        let bottom = h.bottom + p.leave.1;
        let top = h.retract();
        let c = h.at;
        path.feed_to(on(c, helix, 0.0, top), MoveRole::Link, p.feed)?;
        let turns = (top - bottom) / p.pitch;
        turn(
            &mut path,
            c,
            sweep_sign * turns * TAU,
            bottom,
            MoveRole::Cut,
            p.feed,
        )?;
        // Once round at the bottom, level, so the floor is flat.
        let angle = turn(
            &mut path,
            c,
            sweep_sign * TAU,
            bottom,
            MoveRole::Cut,
            p.feed,
        )?;
        let mut radii: Vec<f64> = (1..=passes)
            .map(|k| helix + f64::from(k) * stepover)
            .collect();
        if repeat && let Some(&r) = radii.last() {
            radii.push(r);
        }
        for radius in radii {
            // In from far enough inside for a half circle half the tool across,
            // within what the helix cleared.
            let away = radius - p.tool_radius.min(radius);
            straight(
                &mut path,
                on(c, away, angle, bottom),
                (finish_feed, finish_feed),
            )?;
            lead(
                &mut path,
                c,
                (radius, away),
                angle,
                p.ccw,
                bottom,
                false,
                finish_feed,
            )?;
            turn(
                &mut path,
                c,
                sweep_sign * TAU,
                bottom,
                MoveRole::Cut,
                finish_feed,
            )?;
            lead(
                &mut path,
                c,
                (radius, away),
                angle,
                p.ccw,
                bottom,
                true,
                finish_feed,
            )?;
        }
        let up = path.end().map_or(c, DVec3::truncate).extend(top);
        path.feed_to(up, MoveRole::Retract, p.retract_feed)?;
    }
    Ok((path, cores))
}

/// How holes and bosses are cleared in circles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CircularParams {
    /// The tool's radius.
    pub tool_radius: f64,
    /// Stock left on the wall, and on the floor.
    pub leave: (f64, f64),
    /// How far apart the circles are.
    pub stepover: f64,
    /// The deepest each level is below the last; all at once when `None`.
    pub stepdown: Option<f64>,
    /// How steeply it goes down from level to level, in radians.
    pub ramp_angle: f64,
    /// How far out from a boss its material goes.
    pub boss_width: f64,
    /// Whether it goes counter-clockwise round a hole, seen from above; round
    /// a boss it goes the other way, to cut the same way.
    pub ccw: bool,
    /// Feed round the circles.
    pub feed: f64,
    /// Feed down the helix between levels.
    pub ramp_feed: f64,
    /// Feed out of it.
    pub retract_feed: f64,
}

/// The radii of a round's circles, in the order cut: a hole's from its
/// middle out, close enough in for the tool to cover the middle, to its wall;
/// a boss's from `boss_width` out in to its wall.
fn circles(r: &Round, p: &CircularParams) -> Result<Vec<f64>, MillError> {
    let h = &r.hole;
    if r.boss {
        let last = h.radius + p.tool_radius + p.leave.0;
        let n = if p.boss_width > 0.0 {
            count((p.boss_width / p.stepover - 1e-9).ceil())
        } else {
            0
        };
        return Ok((0..=n)
            .map(|k| last + p.boss_width * f64::from(n - k) / f64::from(n.max(1)))
            .collect());
    }
    let last = h.radius - p.tool_radius - p.leave.0;
    if last <= 1e-3 {
        return Err(too_small(r));
    }
    let n = count((last / p.stepover).max(last / (0.9 * p.tool_radius)).ceil());
    Ok((1..=n)
        .map(|k| last * f64::from(k) / f64::from(n))
        .collect())
}

/// Each hole and boss cleared in circles, level by level a stepdown apart
/// down to its bottom: down to each level along a helix on the first circle
/// at the ramp angle, round it once there, then out (or in) to each next
/// circle along a half circle and round it, and off the last along another,
/// away from the wall; then straight up out of it.
pub fn circular(rounds: &[Round], p: &CircularParams) -> Result<Toolpath, MillError> {
    check("stepover", p.stepover)?;
    check("feed", p.feed)?;
    check("ramp feed", p.ramp_feed)?;
    check("retract feed", p.retract_feed)?;
    if let Some(s) = p.stepdown {
        check("stepdown", s)?;
    }
    let mut path = Toolpath::new();
    for r in rounds {
        let h = &r.hole;
        let c = h.at;
        let radii = circles(r, p)?;
        let (Some(&first), Some(&last)) = (radii.first(), radii.last()) else {
            continue;
        };
        let ccw = p.ccw != r.boss;
        let sign = if ccw { 1.0 } else { -1.0 };
        let bottom = h.bottom + p.leave.1;
        if h.top - bottom <= 1e-6 {
            return Err(MillError::NoLength { x: c.x, y: c.y });
        }
        let top = h.retract();
        let levels = p
            .stepdown
            .map_or(1, |s| count(((h.top - bottom) / s - 1e-9).ceil()));
        let n = f64::from(levels);
        // Down a helix whose turn is as long as its slope allows, and no
        // longer than a level: a circle near a hole's middle is short.
        let pitch = (TAU * first * p.ramp_angle.tan())
            .max(0.05 * p.tool_radius)
            .min(h.top - bottom);
        path.feed_to(on(c, first, 0.0, top), MoveRole::Link, p.feed)?;
        let away = if r.boss {
            last + p.tool_radius
        } else {
            last - p.tool_radius.min(last)
        };
        let mut angle = 0.0;
        for k in 1..=levels {
            let z = h.top - (h.top - bottom) * f64::from(k) / n;
            let from = path.end().map_or(z, |e| e.z);
            let back = on(c, first, angle, from);
            if k > 1 && path.end().is_some_and(|e| e.distance(back) > 1e-9) {
                // Back to the first circle along the ray it left on, across
                // what this level cleared.
                path.feed_to(back, MoveRole::Transition, p.feed)?;
            }
            let turns = ((from - z) / pitch).max(0.0);
            if turns > 0.0 {
                turn(
                    &mut path,
                    c,
                    sign * turns * TAU,
                    z,
                    MoveRole::Ramp,
                    p.ramp_feed,
                )?;
            }
            angle = turn(&mut path, c, sign * TAU, z, MoveRole::Cut, p.feed)?;
            let mut at = first;
            for &radius in radii.iter().skip(1) {
                lead(&mut path, c, (radius, at), angle, ccw, z, false, p.feed)?;
                angle = turn(&mut path, c, sign * TAU, z, MoveRole::Cut, p.feed)?;
                at = radius;
            }
            if (away - last).abs() > 1e-6 {
                lead(&mut path, c, (last, away), angle, ccw, z, true, p.feed)?;
            }
        }
        let up = path.end().map_or(c, DVec3::truncate).extend(top);
        path.feed_to(up, MoveRole::Retract, p.retract_feed)?;
    }
    Ok(path)
}

/// How deep a 60° thread is, from its major diameter in to its minor, on
/// either side, at `pitch`: inside a nut five eighths of the sharp V's
/// height, on a bolt seventeen twenty-fourths, as ISO's basic profiles are.
pub fn thread_depth(pitch: f64, boss: bool) -> f64 {
    let sharp = 3f64.sqrt() / 2.0 * pitch;
    if boss {
        17.0 / 24.0 * sharp
    } else {
        5.0 / 8.0 * sharp
    }
}

/// How threads are milled.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreadParams {
    /// Millimetres a turn.
    pub pitch: f64,
    /// Which way it turns.
    pub hand: ThreadHand,
    /// Its major diameter; a hole's own plus the thread's depth either side,
    /// or a boss's own, when `None`.
    pub diameter: Option<f64>,
    /// How many radial passes the depth is cut in.
    pub passes: u32,
    /// Whether the last pass is cut twice.
    pub spring_pass: bool,
    /// The tool's radius, to its teeth's points.
    pub tool_radius: f64,
    /// How far its lowest tooth is above its end.
    pub tip_length: f64,
    /// How many turns of the thread it cuts each time round.
    pub teeth: u32,
    /// Whether it goes counter-clockwise inside a hole, seen from above;
    /// round a boss it goes the other way, to cut the same way.
    pub ccw: bool,
    /// Feed round the thread.
    pub feed: f64,
    /// Feed down into the hole, and back out.
    pub plunge_feed: f64,
    /// Feed out of it.
    pub retract_feed: f64,
}

/// One thread, as cut: where its passes go, for the tests and the report.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Threaded {
    /// The radius its teeth's points go to on the last pass: the thread's
    /// major radius in a hole, its minor on a boss.
    pub depth_radius: f64,
    /// Whether it goes up as it turns.
    pub rising: bool,
}

/// Each thread milled: for each radial pass, a part of the thread's depth
/// deeper than the last, and each time round the tool cuts (the whole
/// thread for a single-form mill, as many turns as it has rows of teeth for
/// a multi-form one): in from the middle of a hole, or from outside a boss,
/// along a half circle that goes a quarter pitch along the thread, round the
/// helix at the pitch a turn, and out along another. Right-hand threads
/// climb inside a hole going up, left-hand ones going down; on a boss the
/// other way. The tool's end goes no lower than the bottom, so its lowest
/// tooth stops its tip length above it.
pub fn thread(rounds: &[Round], p: &ThreadParams) -> Result<(Toolpath, Vec<Threaded>), MillError> {
    check("pitch", p.pitch)?;
    check("feed", p.feed)?;
    check("plunge feed", p.plunge_feed)?;
    check("retract feed", p.retract_feed)?;
    let mut path = Toolpath::new();
    let mut out = Vec::new();
    let teeth = p.teeth.max(1);
    let passes = p.passes.max(1);
    for r in rounds {
        let h = &r.hole;
        let c = h.at;
        let depth = thread_depth(p.pitch, r.boss);
        // The radius the teeth's points reach on the last pass, and the
        // way in from it to the first.
        let (last, inward) = if r.boss {
            let major = p.diameter.map_or(h.radius, |d| d / 2.0);
            (major - depth, 1.0)
        } else {
            let major = p.diameter.map_or(h.radius + depth, |d| d / 2.0);
            (major, -1.0)
        };
        if !r.boss && p.tool_radius >= h.radius {
            return Err(too_small(r));
        }
        if !r.boss && last <= h.radius - 1e-9 {
            return Err(MillError::NoThread { x: c.x, y: c.y });
        }
        if r.boss && last <= 1e-3 {
            return Err(MillError::NoThread { x: c.x, y: c.y });
        }
        let mut teeth_at: Vec<f64> = (1..=passes)
            .map(|k| last + inward * depth * f64::from(passes - k) / f64::from(passes))
            .collect();
        if p.spring_pass
            && let Some(&l) = teeth_at.last()
        {
            teeth_at.push(l);
        }
        let ccw = p.ccw != r.boss;
        let rising = ccw == (p.hand == ThreadHand::Right);
        let sign = if ccw { 1.0 } else { -1.0 };
        // The lowest the tool's end goes, a lead's quarter pitch below the
        // lowest tooth's start, is the bottom.
        let low = h.bottom + p.tip_length + p.pitch / 4.0;
        let length = h.top - low;
        if length <= 1e-6 {
            return Err(MillError::NoLength { x: c.x, y: c.y });
        }
        // Each time round, from where its lowest tooth starts: once along the
        // whole thread for a single-form mill; a turn for each length its
        // teeth span for a multi-form one.
        let (rounds_of, turns) = if teeth == 1 {
            (1, f64::from(count((length / p.pitch - 1e-9).ceil())))
        } else {
            let span = f64::from(teeth) * p.pitch;
            (count((length / span - 1e-9).ceil()), 1.0)
        };
        let mut starts: Vec<f64> = (0..rounds_of)
            .map(|k| low + f64::from(k) * f64::from(teeth) * p.pitch)
            .collect();
        if !rising {
            starts.reverse();
        }
        let top = h.retract();
        // Where the tool comes in from, away from the wall: a hole's middle,
        // or a tool's width out from a boss.
        let away_of = |at: f64| {
            if r.boss {
                at + p.tool_radius + 2.0 * p.tool_radius
            } else {
                0.0
            }
        };
        let path_radius = |at: f64| {
            if r.boss {
                at + p.tool_radius
            } else {
                at - p.tool_radius
            }
        };
        // A pass so shallow in a hole that the tool's centre is at its middle
        // cuts nothing the next does not.
        teeth_at.retain(|&at| path_radius(at) > 0.01);
        let Some(&first) = teeth_at.first() else {
            return Err(too_small(r));
        };
        path.feed_to(
            on(c, away_of(first), 0.0, top),
            MoveRole::Link,
            p.plunge_feed,
        )?;
        for &at in &teeth_at {
            let radius = path_radius(at);
            let away = away_of(at);
            for &start in &starts {
                let (from, to) = if rising {
                    (start, start + turns * p.pitch)
                } else {
                    (start + turns * p.pitch, start)
                };
                let along = if rising { 1.0 } else { -1.0 };
                let quarter = along * p.pitch / 4.0;
                let z = |tooth: f64| tooth - p.tip_length;
                straight(
                    &mut path,
                    on(c, away, 0.0, z(from - quarter)),
                    (p.plunge_feed, p.retract_feed),
                )?;
                lead(
                    &mut path,
                    c,
                    (radius, away),
                    0.0,
                    ccw,
                    z(from),
                    false,
                    p.feed,
                )?;
                turn(
                    &mut path,
                    c,
                    sign * turns * TAU,
                    z(to),
                    MoveRole::Cut,
                    p.feed,
                )?;
                lead(
                    &mut path,
                    c,
                    (radius, away),
                    0.0,
                    ccw,
                    z(to + quarter),
                    true,
                    p.feed,
                )?;
            }
        }
        let up = path.end().map_or(c, DVec3::truncate).extend(top);
        path.feed_to(up, MoveRole::Retract, p.retract_feed)?;
        out.push(Threaded {
            depth_radius: last,
            rising,
        });
    }
    Ok((path, out))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::indexing_slicing,
    clippy::cast_precision_loss
)]
mod tests;
