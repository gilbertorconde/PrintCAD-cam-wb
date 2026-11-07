//! The blocks of material a stock starts as: boxes, and round bars.
//!
//! A block is in the frame the simulation works in, the tool coming down Z. The
//! stock holds one height a column, so a block is what stands under its top: a
//! bar lying across the tool is material down to the floor below its widest,
//! as the tool, coming from above, can never tell it is not.

use camcraft_geom::mesh::Aabb;
use glam::{DVec2, DVec3};

/// An axis of the frame, as a round bar lies along it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Across the table, as the machine's X.
    X,
    /// Along the table, as the machine's Y.
    Y,
    /// Up the tool.
    Z,
}

impl Axis {
    /// Which coordinate (0, 1 or 2).
    pub fn index(self) -> usize {
        match self {
            Axis::X => 0,
            Axis::Y => 1,
            Axis::Z => 2,
        }
    }

    /// The axis of coordinate `i`; Z for anything past Y.
    pub fn of(i: usize) -> Self {
        match i {
            0 => Axis::X,
            1 => Axis::Y,
            _ => Axis::Z,
        }
    }
}

/// A round bar: a cylinder along an axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    /// What it lies along.
    pub axis: Axis,
    /// The middle of its axis.
    pub centre: DVec3,
    /// Its radius.
    pub radius: f64,
    /// Its length, end to end.
    pub length: f64,
}

/// One block of material.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Block {
    /// A box.
    Box(Aabb),
    /// A round bar.
    Bar(Bar),
}

impl Block {
    /// The box around it.
    pub fn bounds(&self) -> Aabb {
        match self {
            Block::Box(b) => *b,
            Block::Bar(bar) => {
                let (r, l) = (bar.radius, bar.length * 0.5);
                let half = match bar.axis {
                    Axis::X => DVec3::new(l, r, r),
                    Axis::Y => DVec3::new(r, l, r),
                    Axis::Z => DVec3::new(r, r, l),
                };
                Aabb {
                    min: bar.centre - half,
                    max: bar.centre + half,
                }
            }
        }
    }

    /// Its top: the highest it stands anywhere.
    pub fn top(&self) -> f64 {
        self.bounds().max.z
    }

    /// It moved by `by`.
    #[must_use]
    pub fn translated(&self, by: DVec3) -> Self {
        match self {
            Block::Box(b) => Block::Box(b.translated(by)),
            Block::Bar(bar) => Block::Bar(Bar {
                centre: bar.centre + by,
                ..*bar
            }),
        }
    }

    /// The highest its top stands within `grow` of column `p`, or `None` where
    /// it stands nowhere that near. Grown, a grid sampling it at points `grow`
    /// apart either way takes in every column it covers at all: more material,
    /// never less, as a box's cells are taken to its edges.
    pub fn top_near(&self, p: DVec2, grow: f64) -> Option<f64> {
        let b = self.bounds();
        let (lo, hi) = (b.min.truncate() - grow, b.max.truncate() + grow);
        if p.x < lo.x || p.y < lo.y || p.x > hi.x || p.y > hi.y {
            return None;
        }
        let Block::Bar(bar) = self else {
            return Some(b.max.z);
        };
        let c = bar.centre.truncate();
        // How far from its axis, seen from above, less what the grid's
        // spacing could hide.
        let off = match bar.axis {
            Axis::Z => (p.distance(c) - grow).max(0.0),
            Axis::X => ((p.y - c.y).abs() - grow).max(0.0),
            Axis::Y => ((p.x - c.x).abs() - grow).max(0.0),
        };
        if off > bar.radius {
            return None;
        }
        Some(match bar.axis {
            Axis::Z => b.max.z,
            Axis::X | Axis::Y => bar.centre.z + (bar.radius * bar.radius - off * off).sqrt(),
        })
    }

    /// The lowest its underside comes within `grow` of column `p`, or `None`
    /// where it stands nowhere that near: as [`Block::top_near`], from below.
    pub fn bottom_near(&self, p: DVec2, grow: f64) -> Option<f64> {
        let top = self.top_near(p, grow)?;
        Some(match self {
            Block::Bar(bar) if bar.axis != Axis::Z => 2.0 * bar.centre.z - top,
            _ => self.bounds().min.z,
        })
    }

    /// Where it stands above height `z`, seen from above, as a ring of points
    /// counter-clockwise: a polygon around all of it, its corners at most
    /// `tolerance` outside a round edge. `None` where it is all at or below `z`.
    pub fn above(&self, z: f64, tolerance: f64) -> Option<Vec<DVec2>> {
        let b = self.bounds();
        if b.max.z <= z {
            return None;
        }
        let rectangle =
            |lo: DVec2, hi: DVec2| vec![lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y)];
        let bar = match self {
            Block::Box(_) => return Some(rectangle(b.min.truncate(), b.max.truncate())),
            Block::Bar(bar) => bar,
        };
        let c = bar.centre.truncate();
        let r = bar.radius;
        match bar.axis {
            Axis::Z => Some(around_circle(c, r, tolerance)),
            Axis::X | Axis::Y => {
                // Below its axis the bar is as wide as it gets: the column
                // stands to its widest.
                let dz = (z - bar.centre.z).max(0.0);
                let half = (r * r - dz * dz).max(0.0).sqrt();
                let (lo, hi) = (b.min.truncate(), b.max.truncate());
                Some(match bar.axis {
                    Axis::X => {
                        rectangle(DVec2::new(lo.x, c.y - half), DVec2::new(hi.x, c.y + half))
                    }
                    _ => rectangle(DVec2::new(c.x - half, lo.y), DVec2::new(c.x + half, hi.y)),
                })
            }
        }
    }
}

/// A polygon around the circle at `centre` of `radius`, its sides touching
/// it and its corners no more than `tolerance` outside it.
fn around_circle(centre: DVec2, radius: f64, tolerance: f64) -> Vec<DVec2> {
    use std::f64::consts::{PI, TAU};
    // A corner of n sides stands r (1 / cos(π/n) − 1) outside the circle.
    let tolerance = tolerance.max(radius * 1e-6);
    let half = (radius / (radius + tolerance)).clamp(-1.0, 1.0).acos();
    let n = (PI / half.max(1e-3)).ceil().clamp(8.0, 4096.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = n as u32;
    let corner = radius / (PI / f64::from(n)).cos();
    (0..n)
        .map(|k| centre + DVec2::from_angle(TAU * f64::from(k) / f64::from(n)) * corner)
        .collect()
}
