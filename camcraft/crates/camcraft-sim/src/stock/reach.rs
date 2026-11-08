//! What stands above a cutter's flutes, checked against material as the tool
//! moves: the cutter's own length past its flutes, its shank and its holder.
//!
//! The sweep takes the cutter to be all flute, as tall as it needs to be, so
//! it cuts whatever it passes through. A real tool cuts only up to its flute
//! length; above that it rubs, and above its stickout the holder hits. Each of
//! those is a [`Ring`]: a band of heights above the tip, as wide as it is. A
//! ring meets a column of material when the column stands higher, within its
//! radius, than the ring's foot as it passes.
//!
//! It is cheap because most of the time the ring is far above the material:
//! the field is kept in blocks with their tallest cell ([`Tops`]), and a block
//! no taller than the ring's foot is passed over without looking at its cells.

use camcraft_geom::heightmap::{HeightMap, cell_index};
use glam::{DVec2, DVec3, dvec2};

use super::{CellRect, SimMove, Stock, Swept, SweptTool};

/// One band of the tool above its flutes, as heights above its tip: a
/// cylinder, or a cone where its radius at its foot and top differ.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ring {
    /// Its foot, above the tip, in millimetres.
    pub from: f64,
    /// Its top, above the tip.
    pub to: f64,
    /// Its radius at its foot.
    pub radius_from: f64,
    /// Its radius at its top.
    pub radius_to: f64,
}

/// A cone checked as this many cylinders, each as wide as it gets: never
/// less than the cone, at most a quarter of its flare more.
const CONE_STEPS: u32 = 4;

/// The rings of a tool above its flutes, from the tip upward.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Body {
    /// Its rings; [`Contact::ring`] counts them.
    pub rings: Vec<Ring>,
}

impl Body {
    /// The cylinders checked for each ring: `(ring, foot, radius)`. A cone's
    /// are stacked up it, each as wide as the cone gets over it.
    fn cylinders(&self) -> Vec<(usize, f64, f64)> {
        let mut out = Vec::new();
        for (i, r) in self.rings.iter().enumerate() {
            let usable = |v: f64| v.is_finite() && v > 0.0;
            if !usable(r.radius_from) || !usable(r.radius_to) || r.to <= r.from {
                continue;
            }
            if (r.radius_from - r.radius_to).abs() < 1.0e-9 {
                out.push((i, r.from, r.radius_from));
                continue;
            }
            for k in 0..CONE_STEPS {
                let (a, b) = (
                    f64::from(k) / f64::from(CONE_STEPS),
                    f64::from(k + 1) / f64::from(CONE_STEPS),
                );
                let at = |t: f64| r.radius_from + (r.radius_to - r.radius_from) * t;
                out.push((i, r.from + (r.to - r.from) * a, at(a).max(at(b))));
            }
        }
        out
    }
}

/// Where a ring met material on one move: the most it met.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Contact {
    /// Which ring of the [`Body`].
    pub ring: usize,
    /// How far the material stood above the ring's foot, at most, in
    /// millimetres: how much higher the ring would have to be to clear it.
    pub excess: f64,
    /// Where, in XY.
    pub at: DVec2,
    /// Whether the part itself stands in the ring's way somewhere on the
    /// move, not only material left above it.
    pub part: bool,
}

/// Cells to a side of a block of [`Tops`].
const BLOCK: u32 = 8;

/// The tallest cell in each block of a field, so a ring well above the
/// material passes over most of it without looking at its cells.
///
/// Material is only ever taken away, so a block's top read before a cut is
/// never lower than after it: a stale top costs time, never a contact. So a
/// block cut into is only marked, and read again when a ring comes down to
/// its old top; one a ring never comes near is never read again.
#[derive(Debug, Clone, PartialEq)]
pub struct Tops {
    nbx: u32,
    nby: u32,
    tops: Vec<f32>,
    stale: Vec<bool>,
}

impl Tops {
    /// The tops of `field`'s blocks.
    pub fn of(field: &HeightMap) -> Self {
        let nbx = field.nx().div_ceil(BLOCK);
        let nby = field.ny().div_ceil(BLOCK);
        let n = nbx as usize * nby as usize;
        let mut out = Self {
            nbx,
            nby,
            tops: vec![f32::NEG_INFINITY; n],
            stale: vec![false; n],
        };
        for by in 0..nby {
            for bx in 0..nbx {
                out.read(field, bx, by);
            }
        }
        out
    }

    /// Mark the blocks holding any of `rect` as cut into.
    pub fn cut(&mut self, rect: CellRect) {
        for by in rect.y0 / BLOCK..=(rect.y1 / BLOCK).min(self.nby.saturating_sub(1)) {
            for bx in rect.x0 / BLOCK..=(rect.x1 / BLOCK).min(self.nbx.saturating_sub(1)) {
                let i = self.index(bx, by);
                if let Some(s) = self.stale.get_mut(i) {
                    *s = true;
                }
            }
        }
    }

    fn index(&self, bx: u32, by: u32) -> usize {
        by as usize * self.nbx as usize + bx as usize
    }

    /// Read block `(bx, by)`'s top from `field`.
    fn read(&mut self, field: &HeightMap, bx: u32, by: u32) -> f64 {
        let (nx, ny) = (field.nx(), field.ny());
        let data = field.data();
        let mut top = f32::NEG_INFINITY;
        for iy in by * BLOCK..((by + 1) * BLOCK).min(ny) {
            let row = iy as usize * nx as usize;
            let (x0, x1) = (bx * BLOCK, ((bx + 1) * BLOCK).min(nx));
            if let Some(cells) = data.get(row + x0 as usize..row + x1 as usize) {
                for v in cells {
                    if v.is_finite() && *v > top {
                        top = *v;
                    }
                }
            }
        }
        let i = self.index(bx, by);
        if let (Some(t), Some(s)) = (self.tops.get_mut(i), self.stale.get_mut(i)) {
            (*t, *s) = (top, false);
        }
        f64::from(top)
    }

    /// Block `(bx, by)`'s top, read again from `field` if it was cut into
    /// and its old one is above `over`, which is all the caller asks of it.
    fn top(&mut self, field: &HeightMap, bx: u32, by: u32, over: f64) -> f64 {
        let i = self.index(bx, by);
        let old = self
            .tops
            .get(i)
            .map_or(f64::NEG_INFINITY, |v| f64::from(*v));
        if old > over && self.stale.get(i).copied().unwrap_or(false) {
            self.read(field, bx, by)
        } else {
            old
        }
    }
}

/// Where along the straight move `a`-`b` (as a fraction of it) the tool's
/// axis is within `r` of `p` in XY, if anywhere.
fn within(p: DVec2, a: DVec2, b: DVec2, r: f64) -> Option<(f64, f64)> {
    let u = b - a;
    let w = p - a;
    let uu = u.length_squared();
    if uu < 1.0e-18 {
        return (w.length_squared() <= r * r).then_some((0.0, 1.0));
    }
    let wu = w.dot(u);
    let disc = wu * wu - uu * (w.length_squared() - r * r);
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    let (t0, t1) = (((wu - s) / uu).max(0.0), ((wu + s) / uu).min(1.0));
    (t0 <= t1).then_some((t0, t1))
}

/// The material a body passes on its way along moves.
#[derive(Debug, Clone)]
pub struct Reach<'a> {
    /// `(ring, foot, radius)` of each cylinder checked.
    cylinders: Vec<(usize, f64, f64)>,
    rings: usize,
    margin: f64,
    part: Option<&'a HeightMap>,
}

impl<'a> Reach<'a> {
    /// `body`'s reach. Material counts only where it stands more than
    /// `margin` into a ring and more than `margin` inside its radius: the
    /// tolerance the paths were made to, so a wall the cutter finished is not
    /// a contact beside it. `part`, as a field, says whether it is the part
    /// that is hit.
    pub fn new(body: &Body, margin: f64, part: Option<&'a HeightMap>) -> Self {
        Self {
            cylinders: body.cylinders(),
            rings: body.rings.len(),
            margin: margin.max(0.0),
            part,
        }
    }

    /// How many rings its body has: how long a slice of contacts, one for
    /// each, is.
    pub fn rings(&self) -> usize {
        self.rings
    }

    /// Whether there is anything to check.
    pub fn is_empty(&self) -> bool {
        self.cylinders.is_empty()
    }

    /// What the body meets going straight from `from` to `to`, over `field`
    /// as it stands (its blocks' tops `tops`): for each ring that meets
    /// material, the most it meets, kept in `found` by ring where it is more
    /// than what is there.
    ///
    /// Exact for a cylinder: the lowest the ring's foot comes while the axis
    /// is within its radius of a cell is at one end of that stretch, since
    /// the tip climbs or falls evenly along a straight move.
    pub fn straight(
        &self,
        field: &HeightMap,
        tops: &mut Tops,
        from: DVec3,
        to: DVec3,
        found: &mut [Option<Contact>],
    ) {
        if !from.is_finite() || !to.is_finite() {
            return;
        }
        let (origin, pitch) = (field.origin(), field.pitch());
        let (nx, ny) = (field.nx(), field.ny());
        let (a, b) = (from.truncate(), to.truncate());
        let low = from.z.min(to.z);
        for &(ring, foot, radius) in &self.cylinders {
            let r = radius - self.margin;
            if r <= 0.0 {
                continue;
            }
            let clear = low + foot + self.margin;
            let (lo, hi) = (a.min(b) - DVec2::splat(r), a.max(b) + DVec2::splat(r));
            let x0 = cell_index(((lo.x - origin.x) / pitch).floor(), nx);
            let x1 = cell_index(((hi.x - origin.x) / pitch).ceil(), nx);
            let y0 = cell_index(((lo.y - origin.y) / pitch).floor(), ny);
            let y1 = cell_index(((hi.y - origin.y) / pitch).ceil(), ny);
            let mut best = found.get(ring).copied().flatten();
            let mut part = best.is_some_and(|c| c.part);
            for by in y0 / BLOCK..=y1 / BLOCK {
                for bx in x0 / BLOCK..=x1 / BLOCK {
                    // Nothing in a block lower than the ring's foot meets it,
                    // and nothing in one no higher than the most found already
                    // adds to it, once the part is known to be hit.
                    let top = tops.top(field, bx, by, clear);
                    if top <= clear
                        || (part && best.is_some_and(|c| top - (low + foot) <= c.excess))
                    {
                        continue;
                    }
                    for iy in (by * BLOCK).max(y0)..=((by + 1) * BLOCK - 1).min(y1) {
                        for ix in (bx * BLOCK).max(x0)..=((bx + 1) * BLOCK - 1).min(x1) {
                            let Some(h) = field.get(ix, iy) else {
                                continue;
                            };
                            if h <= clear {
                                continue;
                            }
                            let p = origin + dvec2(f64::from(ix) * pitch, f64::from(iy) * pitch);
                            let Some((t0, t1)) = within(p, a, b, r) else {
                                continue;
                            };
                            let z = |t: f64| from.z + (to.z - from.z) * t;
                            let foot_at = z(t0).min(z(t1)) + foot;
                            let excess = h - foot_at;
                            if excess <= self.margin {
                                continue;
                            }
                            if !part {
                                part = self
                                    .part
                                    .and_then(|f| nearest(f, p))
                                    .is_some_and(|top| top - foot_at > self.margin);
                            }
                            if best.is_none_or(|c| excess > c.excess) {
                                best = Some(Contact {
                                    ring,
                                    excess,
                                    at: p,
                                    part,
                                });
                            }
                        }
                    }
                }
            }
            if let (Some(mut c), Some(slot)) = (best, found.get_mut(ring)) {
                c.part = part;
                *slot = Some(c);
            }
        }
    }
}

/// The height of `field`'s cell nearest `p`, if it has one there.
fn nearest(field: &HeightMap, p: DVec2) -> Option<f64> {
    let (origin, pitch) = (field.origin(), field.pitch());
    let fx = ((p.x - origin.x) / pitch).round();
    let fy = ((p.y - origin.y) / pitch).round();
    if !(0.0..f64::from(field.nx())).contains(&fx) || !(0.0..f64::from(field.ny())).contains(&fy) {
        return None;
    }
    field.get(cell_index(fx, field.nx()), cell_index(fy, field.ny()))
}

/// A stock swept with a body watched: the reach, and the stock's block tops,
/// kept as it is cut.
pub struct Watch<'a> {
    reach: Reach<'a>,
    tops: Tops,
}

impl Stock {
    /// Ready to watch `reach` over this stock as it is swept.
    pub fn watch<'a>(&self, reach: Reach<'a>) -> Watch<'a> {
        Watch {
            tops: Tops::of(&self.field),
            reach,
        }
    }

    /// Sweep `moves` as [`Stock::sweep`] does, and before each is swept,
    /// check what stands above the flutes against the material as it then
    /// stands: `seen` is told, of each move that meets some, its place in
    /// `moves` and the most each ring meets.
    ///
    /// The first move is where the tool already is, as in a sweep. A stock
    /// on a rotary is only swept.
    pub fn sweep_watching<T: SweptTool>(
        &mut self,
        moves: &[SimMove],
        tool: &T,
        watch: &mut Watch<'_>,
        seen: &mut dyn FnMut(usize, &[Option<Contact>]),
    ) -> Swept {
        // On a rotary the body stands out along a radius, not up the grid's
        // columns: it is not watched there.
        if watch.reach.is_empty() || self.round.is_some() {
            return self.sweep(moves, tool);
        }
        let mut report = Swept::default();
        let pitch = self.field.pitch();
        let mut found = vec![None; watch.reach.rings()];
        for (i, pair) in moves.windows(2).enumerate() {
            let [before, m] = pair else { continue };
            found.fill(None);
            let mut a = before.to;
            for b in m.chords(a, pitch * 0.1) {
                watch
                    .reach
                    .straight(&self.field, &mut watch.tops, a, b, &mut found);
                a = b;
            }
            if found.iter().any(Option::is_some) {
                seen(i + 1, &found);
            }
            let swept = self.sweep(pair, tool);
            if let Some(rect) = swept.touched {
                watch.tops.cut(rect);
            }
            report.rapid_contacts += swept.rapid_contacts;
            report.deepest_rapid_contact_mm = report
                .deepest_rapid_contact_mm
                .max(swept.deepest_rapid_contact_mm);
            report.touched = CellRect::join(report.touched, swept.touched);
        }
        report
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;
    use crate::stock::MoveClass;
    use glam::{dvec2, dvec3};

    /// A flat mill of radius r.
    struct Flat(f64);
    impl SweptTool for Flat {
        fn max_radius(&self) -> f64 {
            self.0
        }
        fn height_at(&self, d: f64) -> Option<f64> {
            (d <= self.0).then_some(0.0)
        }
    }

    fn cut(to: DVec3) -> SimMove {
        SimMove {
            to,
            class: MoveClass::MayCut,
            arc: None,
        }
    }

    /// A ring 20 mm across with its foot 12 mm up, on a 3 mm cutter.
    fn holder() -> Body {
        Body {
            rings: vec![Ring {
                from: 12.0,
                to: 30.0,
                radius_from: 10.0,
                radius_to: 10.0,
            }],
        }
    }

    /// A block 20 mm tall: a cut 15 mm down it meets the ring 3 mm into it
    /// at its first move, from where the tool already is, and every move
    /// after, until the tool comes out of the block by the ring's radius.
    #[test]
    fn a_ring_meets_the_material_it_comes_down_beside() {
        let mut stock =
            Stock::block(dvec2(-20.0, -20.0), dvec2(20.0, 20.0), 20.0, 0.0, 0.25).unwrap();
        let body = holder();
        let mut watch = stock.watch(Reach::new(&body, 0.01, None));
        let moves = [
            cut(dvec3(-30.0, 0.0, 30.0)),
            cut(dvec3(-30.0, 0.0, 5.0)),
            cut(dvec3(-15.0, 0.0, 5.0)),
            cut(dvec3(-29.0, 0.0, 5.0)),
            cut(dvec3(-31.0, 0.0, 5.0)),
            cut(dvec3(-32.0, 0.0, 5.0)),
        ];
        let mut seen = Vec::new();
        stock.sweep_watching(&moves, &Flat(1.5), &mut watch, &mut |i, c| {
            seen.push((i, c[0].unwrap()));
        });
        let at: Vec<usize> = seen.iter().map(|(i, _)| *i).collect();
        // Down beside the block, 10 mm off it, the ring's edge is on it, by
        // less than the margin. Along into it, back out to 9 mm off it, and
        // on from there, but not from 11 mm off.
        assert_eq!(at, [2, 3, 4], "{seen:?}");
        for (_, c) in &seen {
            assert!((c.excess - 3.0).abs() < 1e-4, "{c:?}");
            assert!(!c.part);
        }
    }

    /// A cone is checked as no narrower than it is, and a ring beyond its
    /// reach or wholly above the material finds nothing.
    #[test]
    fn rings_find_nothing_above_or_beside_the_material() {
        let field = HeightMap::covering(dvec2(-5.0, -5.0), dvec2(5.0, 5.0), 0.1, 10.0).unwrap();
        let mut tops = Tops::of(&field);
        let cone = Body {
            rings: vec![Ring {
                from: 4.0,
                to: 8.0,
                radius_from: 2.0,
                radius_to: 6.0,
            }],
        };
        let reach = Reach::new(&cone, 0.01, Some(&field));
        let mut found = [None];
        // 5.5 mm off the field's edge, the cone is that wide 7.5 mm up; it
        // is checked as its top quarter, as wide as its top from 7 mm up.
        let p = dvec3(-10.5, 0.0, 0.0);
        reach.straight(&field, &mut tops, p, p, &mut found);
        let c = found[0].unwrap();
        assert!(c.part);
        assert!((c.excess - 3.0).abs() < 1e-6, "{c:?}");
        // 12 mm off it, nothing.
        found = [None];
        let p = dvec3(-17.5, 0.0, 0.0);
        reach.straight(&field, &mut tops, p, p, &mut found);
        assert_eq!(found, [None]);
        // Above it, nothing.
        let p = dvec3(0.0, 0.0, 6.5);
        reach.straight(&field, &mut tops, p, p, &mut found);
        assert_eq!(found, [None]);
    }
}
