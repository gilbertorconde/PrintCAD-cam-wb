//! Probing an uneven surface, and correcting cut depths to it.
//!
//! A probe operation ([`StrategySpec::Probe`](crate::job::StrategySpec::Probe))
//! touches the stock's top at the points of a grid, and the controller records
//! where it touched. Read back ([`parse`]) those results are a [`ProbeMap`]: how
//! far the real surface is above or below the work zero at each point, and,
//! between the points, bilinearly. A setup that corrects Z from them
//! ([`ZCorrection`](crate::job::ZCorrection)) moves every move of its paths up
//! or down by the map ([`correct`]), splitting the long ones so the tool follows
//! the surface between the points: an engraving on a warped board, a PCB's
//! isolation cut, or a single point probed to set the work zero.
//!
//! The map is in the program's coordinates, measured from the work zero, as
//! the controller records it: a point probed 0.2 mm above the work zero's
//! height raises everything over it by 0.2 mm.

use glam::{DVec2, DVec3};

use crate::path::toolpath::{Move, MoveKind, Toolpath, ToolpathError};
use crate::strategy::drilling::Hole;

/// Points closer than this along X or Y, in millimetres, are on the same row
/// or column of the grid: a controller records where it was sent to within
/// its resolution, never as far apart as this.
const SAME_LINE: f64 = 0.01;

/// The surface a grid of probed points describes: a height at each point of
/// the grid, and between them, bilinearly.
#[derive(Debug, Clone, PartialEq)]
pub struct ProbeMap {
    /// The grid's columns, in millimetres, increasing.
    xs: Vec<f64>,
    /// Its rows, in millimetres, increasing.
    ys: Vec<f64>,
    /// The height probed at each point, in millimetres, row by row:
    /// `zs[iy * xs.len() + ix]`.
    zs: Vec<f64>,
}

/// Why probe results are not a map.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProbeMapError {
    /// No point at all.
    #[error("there are no probed points")]
    Empty,
    /// A line that is not three numbers.
    #[error("line {line} is not x y z: {text}")]
    NotNumbers {
        /// The line, from one.
        line: usize,
        /// What it says.
        text: String,
    },
    /// A number that is not finite.
    #[error("a probed point is not a number: {0:?}")]
    NotFinite([f64; 3]),
    /// Two points at one place of the grid.
    #[error("x {x} y {y} is probed twice")]
    Twice {
        /// Where, in the file's units.
        x: f64,
        /// Where, in the file's units.
        y: f64,
    },
    /// The points do not fill a grid.
    #[error(
        "the points are not a grid: {points} point(s) on {columns} column(s) and {rows} \
         row(s), where a grid has one at every column of every row"
    )]
    NotAGrid {
        /// How many points.
        points: usize,
        /// How many columns they are on.
        columns: usize,
        /// How many rows.
        rows: usize,
    },
}

/// The points of a probe result file, each `[x, y, z]` as written.
///
/// Two forms are read. LinuxCNC's log (`(PROBEOPEN file)`): a line a point,
/// its numbers apart by spaces, X Y Z first and the other axes after, which
/// are left. And a CSV of `x,y,z`, as a sender's probe log or a spreadsheet
/// writes one. A heading before the first point, a blank line and a comment
/// (`#`, `;` or `(`) are passed over; any other line that is not numbers is
/// refused, not skipped, since a point left out moves the map.
pub fn parse(text: &str) -> Result<Vec<[f64; 3]>, ProbeMapError> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(['#', ';', '(']) {
            continue;
        }
        let fields: Vec<&str> = line
            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .filter(|f| !f.is_empty())
            .collect();
        let numbers: Option<Vec<f64>> = fields
            .iter()
            .take(3)
            .map(|f| f.parse::<f64>().ok())
            .collect();
        match numbers.as_deref() {
            Some(&[x, y, z]) => {
                let p = [x, y, z];
                if !p.iter().all(|v| v.is_finite()) {
                    return Err(ProbeMapError::NotFinite(p));
                }
                out.push(p);
            }
            // A heading, before any point.
            _ if out.is_empty() && fields.iter().any(|f| f.parse::<f64>().is_err()) => {}
            _ => {
                return Err(ProbeMapError::NotNumbers {
                    line: i + 1,
                    text: line.to_string(),
                });
            }
        }
    }
    Ok(out)
}

/// The values of `v`, those within [`SAME_LINE`] of each other taken as one,
/// at their mean, increasing.
fn lines_of(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(f64::total_cmp);
    let mut out: Vec<(f64, f64)> = Vec::new();
    for x in v {
        match out.last_mut() {
            Some((sum, n)) if x - *sum / *n <= SAME_LINE => {
                *sum += x;
                *n += 1.0;
            }
            _ => out.push((x, 1.0)),
        }
    }
    out.into_iter().map(|(sum, n)| sum / n).collect()
}

/// Which of `lines` `v` is on: the nearest.
fn nearest(lines: &[f64], v: f64) -> usize {
    let i = lines.partition_point(|&l| l < v);
    match (i.checked_sub(1).and_then(|j| lines.get(j)), lines.get(i)) {
        (Some(&a), Some(&b)) if v - a <= b - v => i - 1,
        (Some(_), None) => i - 1,
        _ => i,
    }
}

impl ProbeMap {
    /// The map of probed `points`, each `[x, y, z]` in units of `scale`
    /// millimetres: 25.4 for a program in inches. They must fill a grid,
    /// each column at every row, in any order: a single point is a grid of
    /// one, a level offset everywhere, and a single row or column is level
    /// across it.
    pub fn from_points(points: &[[f64; 3]], scale: f64) -> Result<Self, ProbeMapError> {
        if points.is_empty() {
            return Err(ProbeMapError::Empty);
        }
        if let Some(p) = points.iter().find(|p| !p.iter().all(|v| v.is_finite())) {
            return Err(ProbeMapError::NotFinite(*p));
        }
        let mm: Vec<[f64; 3]> = points.iter().map(|p| p.map(|v| v * scale)).collect();
        let xs = lines_of(mm.iter().map(|p| p[0]).collect());
        let ys = lines_of(mm.iter().map(|p| p[1]).collect());
        let (nx, ny) = (xs.len(), ys.len());
        if nx * ny != mm.len() {
            return Err(ProbeMapError::NotAGrid {
                points: mm.len(),
                columns: nx,
                rows: ny,
            });
        }
        let mut zs = vec![None; nx * ny];
        for (p, raw) in mm.iter().zip(points) {
            let k = nearest(&ys, p[1]) * nx + nearest(&xs, p[0]);
            match zs.get_mut(k) {
                Some(slot @ None) => *slot = Some(p[2]),
                _ => {
                    return Err(ProbeMapError::Twice {
                        x: raw[0],
                        y: raw[1],
                    });
                }
            }
        }
        // As many points as places, none twice: every place has one.
        let zs = zs.into_iter().map(|z| z.unwrap_or(0.0)).collect();
        Ok(Self { xs, ys, zs })
    }

    /// The probe results in `text` ([`parse`]), in units of `scale`
    /// millimetres, as a map.
    pub fn read(text: &str, scale: f64) -> Result<Self, ProbeMapError> {
        Self::from_points(&parse(text)?, scale)
    }

    /// How many columns and rows the grid has.
    pub fn size(&self) -> (usize, usize) {
        (self.xs.len(), self.ys.len())
    }

    /// The grid's lowest and highest corners, in millimetres.
    pub fn bounds(&self) -> (DVec2, DVec2) {
        let first = |v: &[f64]| v.first().copied().unwrap_or(0.0);
        let last = |v: &[f64]| v.last().copied().unwrap_or(0.0);
        (
            DVec2::new(first(&self.xs), first(&self.ys)),
            DVec2::new(last(&self.xs), last(&self.ys)),
        )
    }

    /// The lowest and highest heights probed, in millimetres.
    pub fn heights(&self) -> (f64, f64) {
        self.zs
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &z| {
                (lo.min(z), hi.max(z))
            })
    }

    fn z(&self, ix: usize, iy: usize) -> f64 {
        self.zs.get(iy * self.xs.len() + ix).copied().unwrap_or(0.0)
    }

    /// The surface's height over `p`, and how far `p` is outside the grid
    /// (zero inside it). Outside, the height at the nearest point of its
    /// edge: the surface is not guessed past where it was probed. A grid of
    /// one column (or row) is level across it, so has no edge that way.
    pub fn at(&self, p: DVec2) -> (f64, f64) {
        let (mut lo, mut hi) = self.bounds();
        if self.xs.len() < 2 {
            (lo.x, hi.x) = (p.x, p.x);
        }
        if self.ys.len() < 2 {
            (lo.y, hi.y) = (p.y, p.y);
        }
        let q = p.clamp(lo, hi);
        let (ix, tx) = cell(&self.xs, q.x);
        let (iy, ty) = cell(&self.ys, q.y);
        let (jx, jy) = (
            (ix + 1).min(self.xs.len() - 1),
            (iy + 1).min(self.ys.len() - 1),
        );
        let low = self.z(ix, iy) * (1.0 - tx) + self.z(jx, iy) * tx;
        let high = self.z(ix, jy) * (1.0 - tx) + self.z(jx, jy) * tx;
        (low * (1.0 - ty) + high * ty, p.distance(q))
    }

    /// The height over `p`, as [`at`](Self::at) has it.
    pub fn height(&self, p: DVec2) -> f64 {
        self.at(p).0
    }

    /// Whether every height probed is the same: one level offset, as a
    /// single point gives.
    pub fn is_level(&self) -> bool {
        let (lo, hi) = self.heights();
        hi - lo <= 1e-9
    }
}

/// The cell of `lines` `v` (inside them) is in, from its first line, and how
/// far along it `v` is, from 0 to 1. A single line is its own cell.
fn cell(lines: &[f64], v: f64) -> (usize, f64) {
    if lines.len() < 2 {
        return (0, 0.0);
    }
    let i = lines
        .partition_point(|&l| l <= v)
        .saturating_sub(1)
        .min(lines.len() - 2);
    match (lines.get(i), lines.get(i + 1)) {
        (Some(&a), Some(&b)) if b > a => (i, ((v - a) / (b - a)).clamp(0.0, 1.0)),
        _ => (i, 0.0),
    }
}

/// What a probe operation probes.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProbeReport {
    /// Its grid's columns.
    pub columns: usize,
    /// And its rows.
    pub rows: usize,
    /// How far below the top each point may go before the controller gives
    /// up, in millimetres.
    pub max_depth: f64,
}

/// Where the columns (or rows) of a probe grid go from `lo` to `hi`: `count`
/// of them, or as few as keep them no more than `spacing` apart, spread
/// evenly with one at each end. One alone goes in the middle.
pub fn grid_lines(lo: f64, hi: f64, count: Option<u32>, spacing: f64) -> Vec<f64> {
    let width = (hi - lo).max(0.0);
    let n = match count {
        Some(n) => n.max(1),
        None if width <= 1e-9 || spacing.is_nan() || spacing <= 0.0 => 1,
        None => {
            let n = (width / spacing - 1e-9).ceil().clamp(1.0, 1.0e4);
            // At least one and at most ten thousand, so the narrowing is exact.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let n = n as u32;
            n + 1
        }
    };
    if n == 1 {
        return vec![(lo + hi) / 2.0];
    }
    (0..n)
        .map(|k| lo + width * f64::from(k) / f64::from(n - 1))
        .collect()
}

/// A probe operation's path over the grid of `xs` and `ys`: in at the
/// `clearance` height, then at each point, row by row and back and forth
/// along them, down from the `retract` height at `feed` to the `top`, where
/// it expects to touch, and back up at rapid; out at the clearance height.
/// Each touch is a [`MoveRole::Plunge`](crate::path::toolpath::MoveRole):
/// the post writes it as the controller's probing move, which goes on below
/// the top until it touches.
pub fn probe_path(
    xs: &[f64],
    ys: &[f64],
    (clearance, retract, top): (f64, f64, f64),
    feed: f64,
) -> Result<Toolpath, ToolpathError> {
    use crate::path::toolpath::MoveRole;
    let mut path = Toolpath::new();
    let mut last = None;
    for (row, &y) in ys.iter().enumerate() {
        let mut along: Vec<f64> = xs.to_vec();
        if row % 2 == 1 {
            along.reverse();
        }
        for x in along {
            let over = DVec3::new(x, y, retract);
            if path.is_empty() {
                path.rapid_to(DVec3::new(x, y, clearance))?;
            }
            path.rapid_to(over)?;
            path.feed_to(DVec3::new(x, y, top), MoveRole::Plunge, feed)?;
            path.rapid_to(over)?;
            last = Some(over);
        }
    }
    if let Some(p) = last {
        path.rapid_to(DVec3::new(p.x, p.y, clearance))?;
    }
    Ok(path)
}

/// A path corrected to a map, and how much of it was outside the map.
#[derive(Debug, Clone, PartialEq)]
pub struct Corrected {
    /// The path, every move moved up or down by the map.
    pub path: Toolpath,
    /// How many of its points were outside the map, corrected as at its edge.
    pub outside: usize,
    /// The furthest of them, in millimetres.
    pub furthest: f64,
}

/// `path`, every point of it moved along Z by `map`'s height there.
///
/// A straight feeding move is split so no piece is longer than `segment`
/// seen from above, each piece's end on the surface, so the tool follows it
/// between the probed points. An arc, or a helix, is cut into chords within
/// `tolerance`, each split as a straight move is, unless the map is level
/// under it, when it is moved whole and stays an arc. A rapid is moved at its
/// end alone: it goes straight there, as it did, above everything.
pub fn correct(
    path: &Toolpath,
    map: &ProbeMap,
    segment: f64,
    tolerance: f64,
) -> Result<Corrected, ToolpathError> {
    let mut out = Toolpath::new();
    let mut outside = 0;
    let mut furthest: f64 = 0.0;
    let mut lift = |p: DVec3| {
        let (dz, off) = map.at(p.truncate());
        if off > 1e-9 {
            outside += 1;
            furthest = furthest.max(off);
        }
        DVec3::new(p.x, p.y, p.z + dz)
    };
    let mut from: Option<DVec3> = None;
    for m in path.moves() {
        let start = from.unwrap_or(m.to);
        from = Some(m.to);
        match (m.kind, m.feed) {
            (MoveKind::Arc(a), Some(feed)) => {
                let chords = m.chords(start, tolerance);
                let level = map.is_level()
                    || chords.iter().chain([&start]).all(|p| {
                        (map.height(p.truncate()) - map.height(start.truncate())).abs() <= 1e-9
                    });
                if level {
                    let dz = lift(m.to).z - m.to.z;
                    let mut arc = a;
                    arc.centre.z += dz;
                    out.push(Move {
                        to: m.to + DVec3::Z * dz,
                        kind: MoveKind::Arc(arc),
                        ..*m
                    })?;
                    continue;
                }
                let mut at = start;
                for p in chords {
                    split(at, p, segment, &mut |q| out.feed_to(lift(q), m.role, feed))?;
                    at = p;
                }
            }
            (MoveKind::Linear, Some(feed)) => {
                split(start, m.to, segment, &mut |q| {
                    out.feed_to(lift(q), m.role, feed)
                })?;
            }
            _ => out.push(Move {
                to: lift(m.to),
                ..*m
            })?,
        }
    }
    Ok(Corrected {
        path: out,
        outside,
        furthest,
    })
}

/// The points that split the straight move from `a` to `b` into pieces no
/// longer than `segment` seen from above, `b` last, each handed to `each`.
fn split(
    a: DVec3,
    b: DVec3,
    segment: f64,
    each: &mut dyn FnMut(DVec3) -> Result<(), ToolpathError>,
) -> Result<(), ToolpathError> {
    let flat = a.truncate().distance(b.truncate());
    let n = if segment > 0.0 && flat.is_finite() {
        (flat / segment).ceil().clamp(1.0, 1.0e6)
    } else {
        1.0
    };
    // At least one and at most a million, so the narrowing is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = n as u32;
    for k in 1..=n {
        let p = if k == n {
            b
        } else {
            a.lerp(b, f64::from(k) / f64::from(n))
        };
        each(p)?;
    }
    Ok(())
}

/// A drilling operation's holes, each moved along Z by `map`'s height over
/// its axis, as its moves are: what its canned cycles are written from.
pub fn correct_holes(holes: &mut [Hole], map: &ProbeMap) {
    for h in holes {
        let dz = map.height(h.at);
        h.top += dz;
        h.bottom += dz;
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests;
