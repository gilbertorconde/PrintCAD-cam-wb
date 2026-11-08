//! A pattern of an operation the material steers: adaptive clearing, and
//! pocket clearing, which cuts only where material stands. Moved whole, a
//! copy would cut as if the material where it is were the original's: air
//! where an earlier operation, or an earlier copy, already took it, and,
//! where more stands than at the original, the whole width of the tool at
//! once where the original's passes came in from the side.
//!
//! So each copy is worked out where it is. The material there is brought to
//! the original's place through the copy undone, cleared as the original's
//! is, from the same levels and the same part, and the passes moved out to the
//! copy again. A mirrored copy is cleared the other way round, so moved out
//! it cuts the way the original does. And each copy's cut is taken from the
//! material before a copy it reaches is worked out.
//!
//! Copies that reach none of the ones before them that are not yet cut are
//! worked out together, so copies apart cost no more than one each on every
//! core; ones that overlap are worked out in turn. The result is the same on
//! any number of cores.

use camcraft_geom::heightmap::{HeightMap, cell_index};
use camcraft_sim::stock::StockError;
use rayon::prelude::*;

use super::*;
use crate::path::pattern::{Copy, copies, push_copy};
use crate::path::toolpath::MoveRole;
use crate::strategy::MATERIAL_THICKNESS;

/// An operation the material steers, worked out as far as the model takes it.
enum Planned {
    Adaptive(Adaptive),
    Clearing(Clearing),
}

/// One copy's passes, where it is, and what clearing it reported.
struct Worked {
    moves: Vec<Move>,
    report: Report,
}

#[derive(Clone, Copy)]
enum Report {
    Adaptive(AdaptiveReport),
    Clearing(ClearingReport),
}

impl Report {
    fn passes(&self) -> u64 {
        match self {
            Report::Adaptive(r) => r.passes,
            Report::Clearing(r) => r.rings,
        }
    }
}

/// `work`'s operation's pattern, each copy worked out against `stock` as it
/// stands where the copy is, and as the copies before it leave it: the path,
/// every copy one after another, how many passes, and what clearing
/// reported, all copies together. `None` for a strategy the material does
/// not steer, whose copies are its path moved.
pub(super) fn each_copy(
    work: &Work<'_>,
    limit: Option<&Region>,
    stock: &Stock,
    warnings: &mut Vec<String>,
) -> Result<Option<Paths>, RunError> {
    let op = work.op;
    let pitch = stock.field().pitch();
    let mut rest = None;
    let planned = match op.strategy {
        StrategySpec::Adaptive {
            direction,
            flat_areas,
            rest: asked,
        } => {
            rest = asked;
            Planned::Adaptive(Adaptive::new(work, direction, flat_areas, limit)?)
        }
        StrategySpec::PocketClearing {
            min_ring_length,
            flat_areas,
            ..
        } => Planned::Clearing(Clearing::new(
            work,
            min_ring_length,
            flat_areas,
            limit,
            pitch,
        )?),
        _ => return Ok(None),
    };
    let all = copies(&op.transform);
    let (lo, hi) = match &planned {
        Planned::Adaptive(a) => a.area,
        Planned::Clearing(c) => c.area,
    };
    // A copy's passes stay in its area, and a way down beside them within a
    // helix of it; what they cut, and what steers them, is within the tool
    // of them. Two copies each that far round apart neither cut nor see
    // what the other cuts.
    let diameter = work.spec.diameter;
    let reach = work.tool.max_radius()
        + op.helix_diameter(diameter, work.spec.corner_radius()) / 2.0
        + pitch;
    let footprints: Vec<(DVec2, DVec2)> = all.iter().map(|c| footprint(c, lo, hi, reach)).collect();
    let waves = waves(&footprints);

    // The material each wave sees, at every height asked, in the setup's
    // frame: cut by the waves before it.
    let heights = match &planned {
        Planned::Adaptive(a) => a.levels.heights(),
        Planned::Clearing(c) => c.levels.heights(),
    };
    let blocks = work.blocks;
    let tolerance = work.job.tolerance;
    let arc = tolerance * 0.5;
    // The material as the setup starts, on the same grid: where the grid is
    // lower now, something has cut it. For a setup that carries on from an
    // earlier one, what that one left, not its box.
    let fresh = match &planned {
        Planned::Adaptive(_) => Some(work.prepared.fresh_stock(stock.floor(), pitch)?),
        Planned::Clearing(_) => None,
    };
    // What the copies start from: the material as it stands, for pocket
    // clearing and rest machining; else the setup's blocks, so adaptive
    // clearing's copies see only what each other cut, and the original is
    // what it is alone.
    let start = match (&fresh, rest) {
        (Some(f), None) => f,
        _ => stock,
    };
    let adjustment = rest.map_or(0.0, |r| r.adjustment.max(0.0));
    // Where the tool reaches at each level, for the adjustment: what is
    // left there is the rest it would take.
    let reach: Vec<(f64, Region)> = match &planned {
        Planned::Adaptive(a) if adjustment > 0.0 => heights
            .iter()
            .filter_map(|&z| {
                let r = a.levels.allowed(z)?.offset(work.tool.cutter_radius(), arc);
                Some((z, r))
            })
            .collect(),
        _ => Vec::new(),
    };
    let mut cut: Option<Stock> = None;
    let mut made: Vec<Option<Worked>> = all.iter().map(|_| None).collect();
    for (w, wave) in waves.iter().enumerate() {
        // Only the grid about the wave's copies, a tool further round them:
        // the material anywhere else neither steers nor is cut by them.
        let (lo, hi) = wave.iter().filter_map(|&k| footprints.get(k)).fold(
            (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
            |(a, b), f| (a.min(f.0), b.max(f.1)),
        );
        let pad = DVec2::splat(work.tool.max_radius() + 2.0 * pitch);
        let (lo, hi) = (lo - pad, hi + pad);
        let now = cut.as_ref().unwrap_or(start);
        let field = &window(now.field(), lo, hi)?;
        let was = match &fresh {
            Some(f) => Some(window(f.field(), lo, hi)?),
            None => None,
        };
        let standing: Vec<(f64, Region)> = heights
            .par_iter()
            .map(|&z| {
                let r = match &planned {
                    // Adaptive clearing takes the blocks' own edges where
                    // nothing has cut them, less what has: where the grid
                    // stood and stands no longer. That is known to half a
                    // cell, and what is within the tolerance of it is taken
                    // as cut, or along the edge of every earlier cut a pass
                    // would go round taking nothing.
                    Planned::Adaptive(a) => {
                        let h = z + MATERIAL_THICKNESS;
                        let was = was
                            .as_ref()
                            .map(|f| contour_above(f, h))
                            .unwrap_or_default();
                        let gone = was
                            .difference(&contour_above(field, h))
                            .map_err(|e| in_op(op)(StrategyError::from(e)))?;
                        // Material of a shape of its own, a solid's or what
                        // an earlier setup left, is where it stood above the
                        // level; else the blocks, exactly.
                        let blocks = if work.prepared.starts_shaped() {
                            was.clone()
                        } else {
                            material(blocks, z, level_arc(&a.params)).map_err(in_op(op))?
                        };
                        if gone.is_empty() {
                            blocks
                        } else {
                            blocks
                                .difference(&gone.offset(tolerance, arc))
                                .map_err(|e| in_op(op)(StrategyError::from(e)))?
                        }
                    }
                    Planned::Clearing(_) => contour_above(field, z),
                };
                Ok((z, r))
            })
            .collect::<Result<_, RunError>>()?;
        let at = |z: f64| {
            standing
                .iter()
                .find(|(h, _)| (h - z).abs() < 1e-9)
                .map(|(_, r)| r)
        };
        let done: Vec<(usize, Worked)> = wave
            .par_iter()
            .map(|&k| {
                let c = all.get(k).copied().unwrap_or(Copy::SAME);
                let back = c.inverse();
                let here = |z: f64| at(z).map(|r| back.region(r)).unwrap_or_default();
                let turned = c.mirrored();
                let (path, report) = match &planned {
                    Planned::Adaptive(a) => {
                        let uncut = |z: f64| {
                            let m = here(z);
                            match reach.iter().find(|(h, _)| (h - z).abs() < 1e-9) {
                                Some((_, r)) if adjustment > 0.0 => {
                                    thin_left(m, r, adjustment, arc)
                                }
                                _ => Ok(m),
                            }
                        };
                        let (p, r) =
                            a.clear(op, &|z| uncut(z).map_err(StrategyError::from), turned)?;
                        (p, Report::Adaptive(r))
                    }
                    Planned::Clearing(cl) => {
                        let (p, r) = cl.clear(op, &here, turned)?;
                        (p, Report::Clearing(r))
                    }
                };
                Ok((
                    k,
                    Worked {
                        moves: c.moves(path.moves()),
                        report,
                    },
                ))
            })
            .collect::<Result<_, RunError>>()?;
        // What this wave cut, taken from the material, for the waves after it.
        let last = w + 1 == waves.len();
        for (k, m) in done {
            if !last {
                let stock = cut.get_or_insert_with(|| start.clone());
                cut_in(stock, &m.moves, work.tool);
            }
            if let Some(slot) = made.get_mut(k) {
                *slot = Some(m);
            }
        }
    }

    let made: Vec<Worked> = made.into_iter().flatten().collect();
    let mut path = Toolpath::new();
    let mut passes = 0u64;
    let mut empty = 0usize;
    let mut sum: Option<Report> = None;
    for m in &made {
        passes += m.report.passes();
        empty += usize::from(m.report.passes() == 0);
        sum = Some(match (sum, m.report) {
            (None, r) => r,
            (Some(Report::Adaptive(a)), Report::Adaptive(b)) => Report::Adaptive(a.and(b)),
            (Some(Report::Clearing(a)), Report::Clearing(b)) => Report::Clearing(a.and(b)),
            (Some(r), _) => r,
        });
    }
    // Where each copy would go were it the original moved: off the stock,
    // it is said, though it finds nothing there to cut.
    if let Some(first) = made.first() {
        let mut original = Toolpath::new();
        push_copy(&mut original, first.moves.clone())
            .map_err(|e| in_op(op)(StrategyError::from(e)))?;
        outside_said(work, &original, warnings);
    }
    if empty > 0 && empty < made.len() {
        warnings.push(format!(
            "{empty} of the pattern's {} copies found nothing to clear where they are",
            made.len()
        ));
    }
    for m in made {
        push_copy(&mut path, m.moves).map_err(|e| in_op(op)(StrategyError::from(e)))?;
    }
    let detail = match sum {
        Some(Report::Adaptive(r)) => {
            adaptive_said(&r, warnings);
            StrategyDetail::Adaptive(r)
        }
        Some(Report::Clearing(r)) => {
            clearing_said(&r, warnings);
            StrategyDetail::PocketClearing(r)
        }
        None => match &planned {
            Planned::Adaptive(_) => StrategyDetail::Adaptive(AdaptiveReport::default()),
            Planned::Clearing(_) => StrategyDetail::PocketClearing(ClearingReport::default()),
        },
    };
    Ok(Some((path, passes, detail)))
}

/// Where copy `c` of what keeps within `lo`..`hi` reaches, `reach` round it,
/// seen from above.
fn footprint(c: &Copy, lo: DVec2, hi: DVec2, reach: f64) -> (DVec2, DVec2) {
    let corners =
        [lo, dvec2(hi.x, lo.y), hi, dvec2(lo.x, hi.y)].map(|p| c.point(p.extend(0.0)).truncate());
    let (a, b) = corners
        .iter()
        .fold((corners[0], corners[0]), |(a, b), p| (a.min(*p), b.max(*p)));
    (a - DVec2::splat(reach), b + DVec2::splat(reach))
}

/// Which wave each copy is worked out in, the copies of each wave in order:
/// a copy after every earlier one whose footprint overlaps its own, so each
/// sees what those cut; those that overlap none of a wave's are in it.
fn waves(footprints: &[(DVec2, DVec2)]) -> Vec<Vec<usize>> {
    let overlap = |a: &(DVec2, DVec2), b: &(DVec2, DVec2)| {
        a.0.x < b.1.x && b.0.x < a.1.x && a.0.y < b.1.y && b.0.y < a.1.y
    };
    let mut wave_of: Vec<usize> = Vec::with_capacity(footprints.len());
    for (k, f) in footprints.iter().enumerate() {
        let w = footprints
            .iter()
            .take(k)
            .zip(&wave_of)
            .filter(|(g, _)| overlap(f, g))
            .map(|(_, w)| w + 1)
            .max()
            .unwrap_or(0);
        wave_of.push(w);
    }
    let count = wave_of.iter().max().map_or(0, |w| w + 1);
    let mut out = vec![Vec::new(); count];
    for (k, w) in wave_of.into_iter().enumerate() {
        if let Some(wave) = out.get_mut(w) {
            wave.push(k);
        }
    }
    out
}

/// `material` less each piece of what of it `reach` takes that is thinner
/// than `adjustment` everywhere: rest machining's adjustment, so a sliver
/// left on a wall is not gone round for nothing, while a corner thicker than
/// it is cleared whole, its load as it is.
fn thin_left(
    material: Region,
    reach: &Region,
    adjustment: f64,
    arc: f64,
) -> Result<Region, camcraft_geom::contour::RegionError> {
    let thin: Vec<Region> = material
        .intersection(reach)?
        .pieces()
        .into_iter()
        .filter(|p| p.offset(-adjustment / 2.0, arc).is_empty())
        .collect();
    if thin.is_empty() {
        return Ok(material);
    }
    material.difference(&Region::union_all(&thin)?)
}

/// The cells of `field` over `lo`..`hi`, at least one, on its own grid.
/// Its edges are an edge of the material to [`contour_above`], so what is
/// asked of it is well inside them.
fn window(field: &HeightMap, lo: DVec2, hi: DVec2) -> Result<HeightMap, RunError> {
    let (o, p) = (field.origin(), field.pitch());
    let index = |v: f64, n: u32| cell_index(v, n);
    let (x0, x1) = (
        index(((lo.x - o.x) / p).floor(), field.nx()),
        index(((hi.x - o.x) / p).ceil(), field.nx()),
    );
    let (y0, y1) = (
        index(((lo.y - o.y) / p).floor(), field.ny()),
        index(((hi.y - o.y) / p).ceil(), field.ny()),
    );
    let corner = field.position(x0, y0).unwrap_or(o);
    let mut out = HeightMap::filled(corner, p, x1 - x0 + 1, y1 - y0 + 1, f64::NAN)
        .map_err(StockError::from)?;
    for iy in y0..=y1 {
        for ix in x0..=x1 {
            if let Some(h) = field.get(ix, iy) {
                out.set(ix - x0, iy - y0, h);
            }
        }
    }
    Ok(out)
}

/// `moves`, a copy's passes before they are linked, cut into `stock` by
/// `tool`: each pass from where its link brings it, never along the link.
fn cut_in(stock: &mut Stock, moves: &[Move], tool: &EffectiveTool) {
    for pass in moves.chunk_by(|_, m| m.role != MoveRole::Link) {
        stock.sweep(&sim_moves_of(pass), tool);
    }
}
