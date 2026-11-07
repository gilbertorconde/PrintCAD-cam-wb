//! Repeating a strategy's toolpath as a [`Pattern`] says: a `Toolpath ->
//! Toolpath` transform, as ordering and tabs are, made before linking, so
//! the linker links every copy into the material as it is cut, and the
//! simulation, the checks and the program see them all.
//!
//! # Turned over, cut the same way
//!
//! A mirrored copy runs the other way round: what climbed cuts
//! conventional. So each pass of it is turned round where that is safe,
//! and only there:
//!
//! - a pass all at one level, after the link into it, is run backwards
//!   whole, from its other end, which the link then goes to: a ring, a
//!   profile's level, a slot's level one way;
//! - otherwise each stretch of cutting that ends where it starts, a ring
//!   cut round within the pass, is run backwards in place, from and back to
//!   the same point, so whatever comes before and after it is unchanged: a
//!   ramp down onto it stays as it is.
//!
//! What is left, ramps and open passes that go down as they go, is cut the
//! other way, and said ([`Repeated::unturned`]). A strategy that steers its
//! passes by the material ahead of them, adaptive clearing, is never turned
//! round: run backwards, a pass would meet what it was steered round all at
//! once. Nor is a drill's.
//!
//! A run repeats adaptive clearing and pocket clearing no way here: it
//! works each copy out again where it is, mirrored ones the other way
//! round, and joins them with [`push_copy`].

use camcraft_geom::contour::{Contour, Region};
use glam::{DMat2, DVec2, DVec3};

use crate::job::{MirrorAxis, Pattern};
use crate::path::toolpath::{Move, MoveKind, MoveRole, Toolpath, ToolpathError};

/// One copy: where a point of the original goes, seen from above.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Copy {
    /// Turned, and maybe turned over.
    pub turn: DMat2,
    /// Then moved.
    pub shift: DVec2,
}

impl Copy {
    /// The original, where it is.
    pub const SAME: Self = Self {
        turn: DMat2::IDENTITY,
        shift: DVec2::ZERO,
    };

    /// Whether it turns the original over.
    pub fn mirrored(&self) -> bool {
        self.turn.determinant() < 0.0
    }

    /// Where `p` goes, its height kept.
    pub fn point(&self, p: DVec3) -> DVec3 {
        (self.turn * p.truncate() + self.shift).extend(p.z)
    }

    /// Where a direction goes.
    fn direction(&self, v: DVec3) -> DVec3 {
        (self.turn * v.truncate()).extend(v.z)
    }

    /// The copy that undoes this one: where a point of this copy came from.
    pub fn inverse(&self) -> Copy {
        let turn = self.turn.inverse();
        Copy {
            turn,
            shift: -(turn * self.shift),
        }
    }

    /// Where `region` goes, seen from above. Turned over, its rings wind the
    /// other way, which a region read even-odd does not mind.
    pub fn region(&self, region: &Region) -> Region {
        if *self == Self::SAME {
            return region.clone();
        }
        let rings = region
            .rings()
            .iter()
            .filter_map(|r| {
                Contour::new(
                    r.points()
                        .iter()
                        .map(|&p| self.turn * p + self.shift)
                        .collect(),
                )
            })
            .collect();
        Region::new(rings)
    }

    /// `moves` where this copy puts them: an arc's centre and axis with them,
    /// and turned over, its sweep the other way. Nothing is turned round.
    pub fn moves(&self, moves: &[Move]) -> Vec<Move> {
        moved(moves, self)
    }

    /// `self` after `first`.
    fn after(&self, first: &Copy) -> Copy {
        Copy {
            turn: self.turn * first.turn,
            shift: self.turn * first.shift + self.shift,
        }
    }
}

/// Every copy `pattern` makes, the original first: the rows, each of them
/// round the centre, then all of them mirrored.
pub fn copies(pattern: &Pattern) -> Vec<Copy> {
    let mut out = vec![Copy::SAME];
    if let Some(l) = pattern.linear {
        out = (0..l.y_count.max(1))
            .flat_map(|j| {
                (0..l.x_count.max(1)).map(move |i| Copy {
                    turn: DMat2::IDENTITY,
                    shift: DVec2::new(f64::from(i) * l.x_spacing, f64::from(j) * l.y_spacing),
                })
            })
            .collect();
    }
    if let Some(p) = pattern.polar {
        let n = p.count.max(1);
        // A whole turn has the last copy one step short of the first again.
        let step = if p.angle >= 360.0 - 1e-9 || n == 1 {
            p.angle / f64::from(n)
        } else {
            p.angle / f64::from(n - 1)
        };
        let c = DVec2::from(p.centre);
        let round: Vec<Copy> = (0..n)
            .map(|k| {
                let turn = DMat2::from_angle((step * f64::from(k)).to_radians());
                Copy {
                    turn,
                    shift: c - turn * c,
                }
            })
            .collect();
        out = round
            .iter()
            .flat_map(|r| out.iter().map(move |c| r.after(c)))
            .collect();
    }
    if let Some(m) = pattern.mirror {
        let t = DVec2::from(m.through);
        let flip = match m.axis {
            // About a line along X: Y turned over.
            MirrorAxis::X => Copy {
                turn: DMat2::from_diagonal(DVec2::new(1.0, -1.0)),
                shift: DVec2::new(0.0, 2.0 * t.y),
            },
            MirrorAxis::Y => Copy {
                turn: DMat2::from_diagonal(DVec2::new(-1.0, 1.0)),
                shift: DVec2::new(2.0 * t.x, 0.0),
            },
        };
        let mirrored: Vec<Copy> = out.iter().map(|c| flip.after(c)).collect();
        out.extend(mirrored);
    }
    out
}

/// What repeating a toolpath made.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Repeated {
    /// The path: every copy, one after another, each starting with a link.
    pub path: Toolpath,
    /// How many copies, the original among them.
    pub copies: u32,
    /// Passes of mirrored copies that cut and could not be turned round, so
    /// cut the other way.
    pub unturned: u64,
}

/// `path` repeated as `pattern` says, each mirrored copy's passes turned
/// round where they can be, unless `steered`: passes steered by the
/// material ahead of them, which no copy turns round.
pub fn repeat(
    path: &Toolpath,
    pattern: &Pattern,
    steered: bool,
) -> Result<Repeated, ToolpathError> {
    let all = copies(pattern);
    let mut out = Repeated {
        copies: u32::try_from(all.len()).unwrap_or(u32::MAX),
        ..Repeated::default()
    };
    for c in &all {
        let moves = moved(path.moves(), c);
        let moves = if c.mirrored() && !steered {
            let (turned, unturned) = turned_round(&moves);
            out.unturned += unturned;
            turned
        } else {
            if c.mirrored() {
                out.unturned += passes_that_cut(&moves);
            }
            moves
        };
        push_copy(&mut out.path, moves)?;
    }
    Ok(out)
}

/// `moves`, one copy, after what `path` has: it goes to its start by a link,
/// which the linker makes.
pub fn push_copy(path: &mut Toolpath, moves: Vec<Move>) -> Result<(), ToolpathError> {
    if let Some(first) = moves.first()
        && first.role != MoveRole::Link
    {
        path.push(Move {
            to: first.to,
            kind: MoveKind::Linear,
            role: MoveRole::Link,
            feed: first.feed.or(moves.iter().find_map(|m| m.feed)),
        })?;
    }
    for m in moves {
        path.push(m)?;
    }
    Ok(())
}

/// `moves` where copy `c` puts them: an arc's centre and axis with them, and
/// turned over, its sweep the other way.
fn moved(moves: &[Move], c: &Copy) -> Vec<Move> {
    moves
        .iter()
        .map(|m| Move {
            to: c.point(m.to),
            kind: match m.kind {
                MoveKind::Arc(mut a) => {
                    a.centre = c.point(a.centre);
                    a.axis = c.direction(a.axis);
                    if c.mirrored() {
                        a.sweep = -a.sweep;
                    }
                    MoveKind::Arc(a)
                }
                k => k,
            },
            ..*m
        })
        .collect()
}

/// The passes, each a link and what follows it to the next; the moves
/// before the first link, if any, a pass of their own.
fn split_passes(moves: &[Move]) -> Vec<&[Move]> {
    moves.chunk_by(|_, m| m.role != MoveRole::Link).collect()
}

/// How many passes of `moves` cut.
fn passes_that_cut(moves: &[Move]) -> u64 {
    split_passes(moves)
        .iter()
        .filter(|p| p.iter().any(|m| m.role == MoveRole::Cut))
        .count() as u64
}

/// The moves from `from` through `moves`, run backwards: from the last's end
/// to `from`, each the same move the other way.
fn backwards(from: DVec3, moves: &[Move]) -> Vec<Move> {
    let mut starts = Vec::with_capacity(moves.len());
    let mut at = from;
    for m in moves {
        starts.push(at);
        at = m.to;
    }
    moves
        .iter()
        .zip(starts)
        .rev()
        .map(|(m, start)| Move {
            to: start,
            kind: match m.kind {
                MoveKind::Arc(a) => MoveKind::Arc(a.reversed()),
                k => k,
            },
            ..*m
        })
        .collect()
}

/// Whether two points are the same, as a ring closes.
fn same(a: DVec3, b: DVec3) -> bool {
    a.distance(b) < 1e-6
}

/// `moves`, a mirrored copy's, with each pass turned round where it can
/// be (see the module's notes), and how many passes that cut could not be.
fn turned_round(moves: &[Move]) -> (Vec<Move>, u64) {
    let mut out: Vec<Move> = Vec::with_capacity(moves.len());
    let mut unturned = 0;
    for pass in split_passes(moves) {
        let (Some(link), Some(rest)) = (pass.first(), pass.get(1..)) else {
            out.extend_from_slice(pass);
            continue;
        };
        let cuts = rest.iter().any(|m| m.role == MoveRole::Cut);
        if link.role == MoveRole::Link
            && !rest.is_empty()
            && rest.iter().all(|m| (m.to.z - link.to.z).abs() < 1e-9)
        {
            // Level: backwards whole, linked to from its other end.
            let back = backwards(link.to, rest);
            let start = rest.last().map_or(link.to, |m| m.to);
            out.push(Move { to: start, ..*link });
            out.extend(back);
            continue;
        }
        // Each ring within it, in place.
        let mut turned = false;
        let mut at = if link.role == MoveRole::Link {
            out.push(*link);
            link.to
        } else {
            // No link before it: it is from wherever the last ended.
            out.last().map_or(link.to, |m| m.to)
        };
        let body = if link.role == MoveRole::Link {
            rest
        } else {
            pass
        };
        let mut i = 0;
        while i < body.len() {
            let Some(m) = body.get(i) else { break };
            if m.role != MoveRole::Cut {
                out.push(*m);
                at = m.to;
                i += 1;
                continue;
            }
            // The stretch of cutting from here, and the furthest it comes
            // back to where it started.
            let end = body.get(i..).map_or(0, |s| {
                s.iter().take_while(|m| m.role == MoveRole::Cut).count()
            });
            let stretch = body.get(i..i + end).unwrap_or(&[]);
            let closes = stretch.iter().rposition(|m| same(m.to, at));
            match closes {
                Some(k) if k > 0 => {
                    let ring = stretch.get(..=k).unwrap_or(&[]);
                    out.extend(backwards(at, ring));
                    turned = true;
                    i += k + 1;
                }
                _ => {
                    out.push(*m);
                    at = m.to;
                    i += 1;
                }
            }
        }
        if cuts && !turned {
            unturned += 1;
        }
    }
    (out, unturned)
}

/// How far the cutting moves of `path`, from above, go outside the box
/// `lo`..`hi`, at most: zero inside it.
pub fn overhang(path: &[Move], lo: DVec2, hi: DVec2) -> f64 {
    path.iter()
        .filter(|m| m.role != MoveRole::Rapid && m.role != MoveRole::Link)
        .map(|m| {
            let p = m.to.truncate();
            let out = (lo - p).max(p - hi).max(DVec2::ZERO);
            out.length()
        })
        .fold(0.0, f64::max)
}

/// Each copy's path, apart: for checking each against the stock.
pub fn each(path: &Toolpath, pattern: &Pattern) -> Vec<Vec<Move>> {
    copies(pattern)
        .iter()
        .map(|c| moved(path.moves(), c))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp, clippy::indexing_slicing)]
mod tests;
