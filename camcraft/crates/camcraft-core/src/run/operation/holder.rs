//! What stands above the tool's flutes, its body past them, its shank and its
//! holder: watched as an operation is cut into the material, and, where a 3D
//! finish asks, kept off the part by leaving out the cuts where it meets it.

use camcraft_geom::heightmap::HeightMap;
use camcraft_sim::stock::{Contact, MoveClass, Reach, SimMove, Swept, Tops, Watch};

use super::*;
use crate::job::{AboveFlutes, Piece};
use crate::path::toolpath::{MoveKind, MoveRole, ToolpathError};
use crate::run::{Clash, Clashes};

/// The watch kept on an operation's tool above its flutes while it is cut.
pub(super) struct Watching<'a> {
    watch: Watch<'a>,
    /// Each ring's piece, and how wide it is at most.
    pieces: Vec<(Piece, f64)>,
    stickout: Option<f64>,
    unchecked: Vec<String>,
    /// Where in the path the next cut starts: the move the tool is at.
    at: usize,
    /// What each ring met, by ring.
    found: Vec<Option<Clash>>,
}

impl<'a> Watching<'a> {
    /// A watch on `spec`'s tool above its flutes over `stock`, to the
    /// tolerance `margin`, saying where it is the part (`part`) that is hit.
    pub(super) fn new(stock: &Stock, spec: &ToolSpec, margin: f64, part: &'a HeightMap) -> Self {
        let AboveFlutes {
            body,
            pieces,
            stickout,
            unchecked,
        } = spec.above_flutes();
        let reach = Reach::new(&body, margin, Some(part));
        let pieces = pieces
            .into_iter()
            .zip(&body.rings)
            .map(|(p, r)| (p, 2.0 * r.radius_from.max(r.radius_to)))
            .collect::<Vec<_>>();
        Self {
            watch: stock.watch(reach),
            found: vec![None; pieces.len()],
            pieces,
            stickout,
            unchecked,
            at: 0,
        }
    }

    /// Cut `moves` into `stock` with `tool`, watching: the first is where the
    /// tool already is, the last move cut before, or the path's first.
    pub(super) fn cut<T: camcraft_sim::stock::SweptTool>(
        &mut self,
        stock: &mut Stock,
        moves: &[SimMove],
        tool: &T,
    ) -> Swept {
        let base = self.at;
        let (found, pieces) = (&mut self.found, &self.pieces);
        let swept = stock.sweep_watching(moves, tool, &mut self.watch, &mut |i, contacts| {
            let rapid = moves.get(i).is_some_and(|m| m.class == MoveClass::Rapid);
            record(found, pieces, base + i, rapid, contacts);
        });
        self.at = base + moves.len().saturating_sub(1);
        swept
    }

    /// What it found.
    pub(super) fn finish(self) -> Clashes {
        let found: Vec<Clash> = self.found.into_iter().flatten().collect();
        let into_holder = found
            .iter()
            .filter(|c| matches!(c.piece, Piece::Holder(_)))
            .map(|c| c.deepest)
            .fold(None, |m: Option<f64>, d| Some(m.map_or(d, |m| m.max(d))));
        Clashes {
            needs_stickout: self.stickout.zip(into_holder).map(|(s, d)| s + d),
            checked: !self.pieces.is_empty(),
            unchecked: self.unchecked,
            trimmed: 0,
            found,
        }
    }
}

/// The warning for `clash`, one of `all`.
pub(super) fn clash_warning(clash: &Clash, all: &Clashes) -> String {
    clash.line(all.needs_stickout)
}

/// Cut a path kept from an earlier run into `stock` again, watching `spec`'s
/// tool above its flutes as [`run_operation`] does.
pub(in crate::run) fn cut_kept(
    stock: &mut Stock,
    path: &Toolpath,
    spec: &ToolSpec,
    tool: &EffectiveTool,
    margin: f64,
    part: &HeightMap,
) -> (SimReport, Clashes) {
    let before = stock.volume();
    let mut watching = Watching::new(stock, spec, margin, part);
    let swept = watching.cut(stock, &sim_moves_of(path.moves()), tool);
    let sim = SimReport {
        removed_mm3: (before - stock.volume()).max(0.0),
        rapid_contacts: swept.rapid_contacts,
        deepest_rapid_contact_mm: swept.deepest_rapid_contact_mm,
    };
    (sim, watching.finish())
}

/// Add what each ring met at move `index` to `found`.
fn record(
    found: &mut [Option<Clash>],
    pieces: &[(Piece, f64)],
    index: usize,
    rapid: bool,
    contacts: &[Option<Contact>],
) {
    for (slot, (c, piece)) in found.iter_mut().zip(contacts.iter().zip(pieces)) {
        let Some(c) = c else { continue };
        let (piece, diameter) = (&piece.0, piece.1);
        match slot {
            Some(clash) => {
                clash.moves += 1;
                clash.deepest = clash.deepest.max(c.excess);
                clash.part |= c.part;
                clash.rapid |= rapid;
            }
            None => {
                *slot = Some(Clash {
                    piece: *piece,
                    diameter,
                    first_move: index,
                    moves: 1,
                    deepest: c.excess,
                    part: c.part,
                    rapid,
                });
            }
        }
    }
}

/// Whether `op`'s strategy follows the part, so leaving out a cut where the
/// tool above its flutes meets the part leaves the rest whole: the 3D
/// finishes.
pub(super) fn trims(op: &Operation) -> bool {
    matches!(
        op.strategy,
        StrategySpec::Parallel { .. }
            | StrategySpec::Scallop { .. }
            | StrategySpec::Contour { .. }
            | StrategySpec::Flat { .. }
            | StrategySpec::Spiral { .. }
            | StrategySpec::Radial { .. }
            | StrategySpec::MorphedSpiral { .. }
    )
}

/// `raw` with every cut left out where `spec`'s tool above its flutes would
/// meet the part (`part`, as a field), to the tolerance `margin`, and a link
/// for the linker where the cuts go on; how many were left out.
///
/// The part does not change as the operation cuts, so this is checked once,
/// before linking; material left above the part by an earlier operation is
/// the check's to find.
pub(super) fn trim(
    raw: &Toolpath,
    spec: &ToolSpec,
    part: &HeightMap,
    margin: f64,
) -> Result<(Toolpath, u64), ToolpathError> {
    let above = spec.above_flutes();
    let reach = Reach::new(&above.body, margin, None);
    if reach.is_empty() {
        return Ok((raw.clone(), 0));
    }
    let mut tops = Tops::of(part);
    let mut found = vec![None; reach.rings()];
    let mut hits = |from: DVec3, m: &Move| {
        found.fill(None);
        let mut a = from;
        for b in m.chords(from, margin.max(1.0e-3)) {
            reach.straight(part, &mut tops, a, b, &mut found);
            a = b;
        }
        found.iter().any(Option::is_some)
    };
    leave_out(raw, |from, m| m.role == MoveRole::Cut && hits(from, m))
}

/// `raw` with every move `out` says to leave out (from where the tool is)
/// left out, a link for the linker where the moves go on; how many were.
pub(in crate::run) fn leave_out(
    raw: &Toolpath,
    mut out: impl FnMut(DVec3, &Move) -> bool,
) -> Result<(Toolpath, u64), ToolpathError> {
    let mut kept: Vec<Move> = Vec::with_capacity(raw.len());
    let link = |kept: &mut Vec<Move>, m: Move| {
        // A link straight after another goes where the second does.
        if kept.last().is_some_and(|l| l.role == MoveRole::Link) {
            kept.pop();
        }
        kept.push(m);
    };
    let (mut dropped, mut gap) = (0, false);
    let mut at: Option<DVec3> = None;
    for m in raw.moves() {
        let from = at.unwrap_or(m.to);
        at = Some(m.to);
        if out(from, m) {
            dropped += 1;
            gap = true;
            continue;
        }
        if m.role == MoveRole::Link {
            gap = false;
            link(&mut kept, *m);
            continue;
        }
        if gap {
            gap = false;
            let feed = m.feed.or(Some(1.0));
            link(
                &mut kept,
                Move {
                    to: from,
                    kind: MoveKind::Linear,
                    role: MoveRole::Link,
                    feed,
                },
            );
        }
        kept.push(*m);
    }
    while kept.last().is_some_and(|l| l.role == MoveRole::Link) {
        kept.pop();
    }
    let mut out = Toolpath::new();
    for m in kept {
        out.push(m)?;
    }
    Ok((out, dropped))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::job::{Holder, HolderSegment};
    use glam::dvec3;

    /// A floor at 0 with a wall 30 mm tall from x = 10 on, and a pass along
    /// the floor towards it: the cuts within the holder's reach of the wall
    /// are left out, and what is left still starts with a link.
    #[test]
    fn cuts_where_the_holder_meets_the_part_are_left_out() {
        let mut field = HeightMap::covering(
            glam::dvec2(-30.0, -10.0),
            glam::dvec2(30.0, 10.0),
            0.25,
            0.0,
        )
        .unwrap();
        for iy in 0..field.ny() {
            for ix in 0..field.nx() {
                if field.position(ix, iy).unwrap().x >= 10.0 {
                    field.set(ix, iy, 30.0);
                }
            }
        }
        let tool = ToolSpec {
            flute_length: Some(10.0),
            stickout: Some(20.0),
            holder: Some(Holder {
                name: String::new(),
                segments: vec![HolderSegment::cylinder(28.0, 10.0)],
            }),
            ..ToolSpec::default()
        };
        let mut raw = Toolpath::new();
        raw.feed_to(dvec3(-20.0, 0.0, 0.0), MoveRole::Link, 500.0)
            .unwrap();
        for k in 1..=29 {
            raw.feed_to(dvec3(-20.0 + f64::from(k), 0.0, 0.0), MoveRole::Cut, 500.0)
                .unwrap();
        }
        let (kept, dropped) = trim(&raw, &tool, &field, 0.05).unwrap();
        // The holder reaches 14 mm, less the margin: every cut ending past
        // x = -4 goes.
        let last = kept.moves().last().unwrap().to.x;
        assert_eq!(last, -4.0);
        assert_eq!(dropped, 13);
        assert_eq!(kept.moves()[0].role, MoveRole::Link);
        // Out further, with flutes as long, the holder is over the wall's
        // top and nothing goes.
        let long = ToolSpec {
            flute_length: Some(30.0),
            stickout: Some(31.0),
            ..tool.clone()
        };
        let (all, none) = trim(&raw, &long, &field, 0.05).unwrap();
        assert_eq!((all.len(), none), (raw.len(), 0));
    }
}
