//! Fixtures: vise jaws, clamps, a sacrificial board, what holds the part.
//! They are never cut, so every move of every operation is checked against
//! them, the tool whole: its cutter, its body past the flutes, its shank and
//! its holder, cutting moves and rapids alike, as the holder is checked
//! against the material ([`camcraft_sim::stock::Reach`]).
//!
//! A tool comes down from above, its spindle above it, so a fixture is its
//! top: a column of it stands from wherever up to that, and the tool meets it
//! wherever it comes lower within its reach. Nothing cuts a fixture, so its
//! top is laid on a grid once for the setup, and each operation is checked
//! against it as it stands.

use camcraft_geom::heightmap::HeightMap;
use camcraft_geom::mesh::TriMesh;
use camcraft_sim::stock::{Body, Contact, PartSurface, Reach, Ring, StockError, Tops};
use glam::{DVec2, DVec3};

use crate::cutter::tool::ToolProfile;
use crate::job::{Piece, ToolSpec};
use crate::path::toolpath::{Move, MoveRole, Toolpath, ToolpathError};

/// A fixture, on the simulator's grid.
#[derive(Debug, Clone)]
pub struct Fixture {
    /// Its model's name.
    pub name: String,
    /// Its top, over the window of the grid it covers.
    pub top: HeightMap,
}

/// The coarsest grid a fixture is laid on, in millimetres: its top is
/// grown a cell all round, so a coarser grid only finds it nearer, never
/// further, and checking every move against it stays quick.
pub const FIXTURE_PITCH: f64 = 0.2;

impl Fixture {
    /// `mesh`, a fixture in the setup's frame, laid on a grid no finer than
    /// `pitch` and no coarser than [`FIXTURE_PITCH`] unless `pitch` is: each
    /// cell the highest it comes over that cell and the cells round it, so
    /// the tool is never let nearer it than it is.
    pub fn of(name: &str, mesh: &TriMesh, pitch: f64) -> Result<Self, StockError> {
        let pitch = pitch.max(FIXTURE_PITCH);
        let b = mesh.bounds().ok_or(StockError::NoBlocks)?;
        let pad = DVec2::splat(2.0 * pitch);
        let surface =
            PartSurface::from_mesh(mesh, b.min.truncate() - pad, b.max.truncate() + pad, pitch)?;
        let sampled = surface.field();
        let mut top = sampled.clone();
        let (nx, ny) = (sampled.nx(), sampled.ny());
        for iy in 0..ny {
            for ix in 0..nx {
                let mut high: Option<f64> = None;
                for jy in iy.saturating_sub(1)..=(iy + 1).min(ny - 1) {
                    for jx in ix.saturating_sub(1)..=(ix + 1).min(nx - 1) {
                        if let Some(h) = sampled.get(jx, jy) {
                            high = Some(high.map_or(h, |m: f64| m.max(h)));
                        }
                    }
                }
                if let Some(h) = high {
                    top.set(ix, iy, h);
                }
            }
        }
        Ok(Self {
            name: name.to_string(),
            top,
        })
    }
}

/// What of a tool is checked against a fixture: its cutter, or a piece above
/// its flutes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Part {
    /// The cutter, up to its flute length.
    Cutter,
    /// A piece above its flutes.
    Above(Piece),
}

/// One fixture met by an operation's tool.
#[derive(Debug, Clone, PartialEq)]
pub struct FixtureHit {
    /// The fixture, by its model's name.
    pub fixture: String,
    /// Which of the setup's placements of the part it was met at, from zero:
    /// the operation's path moved there ([`crate::job::Setup::placements`]).
    pub placement: usize,
    /// What of the tool met it first.
    pub part: Part,
    /// How wide that is, in millimetres.
    pub diameter: f64,
    /// The first move it meets it on, as an index into the path.
    pub first_move: usize,
    /// How many moves meet it.
    pub moves: u64,
    /// The most it stands into the tool, in millimetres.
    pub deepest: f64,
    /// Whether a rapid is among the moves.
    pub rapid: bool,
}

impl FixtureHit {
    /// What it is, in a sentence. `wcs`, when there are several placements,
    /// names the one it is at.
    pub fn line(&self, wcs: Option<&str>) -> String {
        let what = match self.part {
            Part::Cutter => format!("the cutter (Ø{:.2})", self.diameter),
            Part::Above(Piece::Body) => "the cutter above its flutes".to_string(),
            Part::Above(Piece::Shank) => format!("the shank (Ø{:.2})", self.diameter),
            Part::Above(Piece::Holder(i)) => {
                format!("holder segment {} (Ø{:.2})", i + 1, self.diameter)
            }
        };
        let at = wcs.map(|w| format!(" at {w}")).unwrap_or_default();
        let rapid = if self.rapid {
            ", a rapid among them"
        } else {
            ""
        };
        format!(
            "{what} meets the fixture '{}'{at} on {} move(s) from move {}{rapid}, {:.2} mm into it",
            self.fixture,
            self.moves,
            self.first_move + 1,
            self.deepest
        )
    }
}

/// How many cylinders a cutter's tip is checked as, up from its point to
/// where it is its full width: each as wide as the tip gets over it, so
/// never less than the cutter, at most an eighth of its radius more.
const TIP_STEPS: u32 = 8;

/// A tool whole, as rings over a fixture: its cutter up to its flute
/// length (or, with none known, all the way up), as wide as its tip gets
/// over each of [`TIP_STEPS`] steps and then its full width, and what stands
/// above its flutes. Each ring's part and width, in the body's order.
pub fn whole_tool(spec: &ToolSpec) -> (Body, Vec<(Part, f64)>) {
    let mut body = Body::default();
    let mut parts = Vec::new();
    let Ok(cutter) = spec.build() else {
        return (body, parts);
    };
    let r = cutter.max_radius();
    let above = spec.above_flutes();
    // The cutter ends where the first piece above it starts.
    let top = spec
        .flute_length
        .or_else(|| above.body.rings.first().map(|g| g.from))
        .unwrap_or(f64::INFINITY);
    let mut from = 0.0;
    for k in 1..=TIP_STEPS {
        let d = r * f64::from(k) / f64::from(TIP_STEPS);
        let h = cutter.height_at(d).unwrap_or(from).max(from).min(top);
        if h > from {
            body.rings.push(Ring {
                from,
                to: h,
                radius_from: d,
                radius_to: d,
            });
            parts.push((Part::Cutter, 2.0 * d));
            from = h;
        }
    }
    if top > from {
        body.rings.push(Ring {
            from,
            to: top,
            radius_from: r,
            radius_to: r,
        });
        parts.push((Part::Cutter, 2.0 * r));
    }
    for (ring, piece) in above.body.rings.iter().zip(above.pieces) {
        body.rings.push(*ring);
        parts.push((
            Part::Above(piece),
            2.0 * ring.radius_from.max(ring.radius_to),
        ));
    }
    (body, parts)
}

/// Where `path`, moved by `by`, meets `fixture` with `spec`'s tool whole, to
/// the tolerance `margin`; `None` where it never does.
pub fn hit(
    path: &Toolpath,
    spec: &ToolSpec,
    fixture: &Fixture,
    by: DVec3,
    margin: f64,
) -> Option<(Part, f64, usize, u64, f64, bool)> {
    let (body, parts) = whole_tool(spec);
    let reach = Reach::new(&body, margin, None);
    if reach.is_empty() {
        return None;
    }
    let mut tops = Tops::of(&fixture.top);
    let mut found: Vec<Option<Contact>> = vec![None; reach.rings()];
    let mut first: Option<(usize, usize)> = None;
    let (mut moves, mut deepest, mut rapid) = (0u64, 0.0f64, false);
    let mut at: Option<DVec3> = None;
    for (i, m) in path.moves().iter().enumerate() {
        let Some(from) = at.replace(m.to) else {
            continue;
        };
        found.fill(None);
        let mut a = from + by;
        for b in m.chords(from, margin.max(1.0e-3)) {
            reach.straight(&fixture.top, &mut tops, a, b + by, &mut found);
            a = b + by;
        }
        let met = found
            .iter()
            .enumerate()
            .filter_map(|(k, c)| Some((k, c.as_ref()?)));
        let mut any = false;
        for (k, c) in met {
            any = true;
            deepest = deepest.max(c.excess);
            if first.is_none() {
                first = Some((k, i));
            }
        }
        if any {
            moves += 1;
            rapid |= m.feed.is_none();
        }
    }
    let (ring, index) = first?;
    let (part, diameter) = parts.get(ring).copied()?;
    Some((part, diameter, index, moves, deepest, rapid))
}

/// `raw` with every cut left out where `spec`'s tool whole would meet one of
/// `fixtures`, to the tolerance `margin`, and a link where the cuts go on, as
/// using the shank and holder leaves them out where they meet the part; how
/// many were left out.
pub fn trim(
    raw: &Toolpath,
    spec: &ToolSpec,
    fixtures: &[Fixture],
    margin: f64,
) -> Result<(Toolpath, u64), ToolpathError> {
    let (body, _) = whole_tool(spec);
    let reach = Reach::new(&body, margin, None);
    if reach.is_empty() || fixtures.is_empty() {
        return Ok((raw.clone(), 0));
    }
    let mut tops: Vec<Tops> = fixtures.iter().map(|f| Tops::of(&f.top)).collect();
    let mut found = vec![None; reach.rings()];
    let mut hits = |from: DVec3, m: &Move| {
        fixtures.iter().zip(&mut tops).any(|(f, tops)| {
            found.fill(None);
            let mut a = from;
            for b in m.chords(from, margin.max(1.0e-3)) {
                reach.straight(&f.top, tops, a, b, &mut found);
                a = b;
            }
            found.iter().any(Option::is_some)
        })
    };
    super::operation::leave_out(raw, |from, m| m.role == MoveRole::Cut && hits(from, m))
}
