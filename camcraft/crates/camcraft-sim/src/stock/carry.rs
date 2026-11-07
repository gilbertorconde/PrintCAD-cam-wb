//! Material carried from one setup into another: what one left, seen down the
//! other's tool axis.
//!
//! A stock holds one stretch of material a column, from its bottom up to its
//! top. Carried into a frame whose tool comes down the same line, the other
//! way up as a part turned over is, or turned about it, a column stays one
//! column and is carried as it is, its top and bottom swapping ends when it
//! is turned over. A cell takes the material of every column of the earlier
//! grid whose cell overlaps its own, so where the two grids meet cell for
//! cell nothing is gained, and where they do not, nothing is lost.
//!
//! Carried into a frame whose tool comes in across the columns, each new
//! column is looked down from above and from below for the first material,
//! and is held as everything between the two: a pocket or a slot that the new
//! column passes through on its way, inside the material, is taken as
//! material, as are the edges of every stretch to a cell all round. That is
//! more material, never less: a rapid is never let through what is there, nor
//! a cut planned as air.

use camcraft_geom::heightmap::{HeightMap, narrow};
use camcraft_geom::mesh::Aabb;
use camcraft_geom::steps::bounded_count;
use glam::{DAffine3, DVec2, DVec3};
use rayon::prelude::*;

use super::{Stock, StockError, THIN};

/// How material in one frame is carried into another.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Carry {
    /// From the frame the material was cut in to the frame it is carried into:
    /// a turn and a move, nothing stretched.
    pub to: DAffine3,
    /// The box it is within, in the frame it was cut in: its stock's. The
    /// grid's cells past the box's edges, which a stock's blocks are taken to
    /// ([`Stock::from_blocks`]), are held at them.
    pub within: Aabb,
}

/// How far, as the sine of the angle between them, a tool axis may be from the
/// one the material was cut along and still be along it.
const ALONG: f64 = 1e-9;

impl Stock {
    /// It with an underside, each column's bottom where `bottom` says the
    /// material starts under it (`None`, or anything below the floor, is the
    /// floor), so it can be carried into a frame that sees it from another
    /// side. A column whose top is at or under its bottom is emptied. A stock
    /// that has an underside already is as it was.
    #[must_use]
    pub fn with_bottoms(mut self, bottom: impl Fn(DVec2) -> Option<f64> + Sync) -> Self {
        if self.under.is_some() {
            return self;
        }
        let floor = self.floor;
        let (origin, pitch, nx) = (self.field.origin(), self.field.pitch(), self.field.nx());
        let mut under = vec![narrow(floor); self.field.data().len()];
        self.field
            .data_mut()
            .par_chunks_mut(nx as usize)
            .zip(under.par_chunks_mut(nx as usize))
            .enumerate()
            .for_each(|(iy, (tops, bottoms))| {
                let y = origin.y + f64::from(u32::try_from(iy).unwrap_or(u32::MAX)) * pitch;
                for ((top, under), ix) in tops.iter_mut().zip(bottoms).zip(0u32..) {
                    let p = DVec2::new(origin.x + f64::from(ix) * pitch, y);
                    let at = bottom(p).unwrap_or(floor).max(floor);
                    if f64::from(*top) <= at + THIN {
                        *top = narrow(floor);
                    } else {
                        *under = narrow(at);
                    }
                }
            });
        self.under = Some(under.into());
        self
    }

    /// Cell `i`'s material within `w`'s heights, its bottom and top, or
    /// `None` where it has none.
    fn column(&self, i: usize, w: &Aabb) -> Option<(f64, f64)> {
        let top = f64::from(*self.field.data().get(i)?);
        if !(top.is_finite() && top > f64::from(narrow(self.floor))) {
            return None;
        }
        let bottom = self
            .under
            .as_deref()
            .and_then(|u| u.get(i))
            .map_or(self.floor, |b| f64::from(*b))
            .max(self.floor);
        let (bottom, top) = (bottom.max(w.min.z), top.min(w.max.z));
        (top > bottom + THIN).then_some((bottom, top))
    }

    /// The box round its material carried by `carry`, in the frame it is
    /// carried into; `None` when it has no material.
    pub fn carried_bounds(&self, carry: &Carry) -> Option<Aabb> {
        let w = carry.within;
        let half = self.field.pitch() * 0.5;
        let (nx, ny) = (self.field.nx(), self.field.ny());
        let grow = |b: Option<Aabb>, p: DVec3| {
            Some(b.map_or(Aabb { min: p, max: p }, |b| Aabb {
                min: b.min.min(p),
                max: b.max.max(p),
            }))
        };
        (0..ny)
            .into_par_iter()
            .filter_map(|iy| {
                let mut b: Option<Aabb> = None;
                for ix in 0..nx {
                    let i = iy as usize * nx as usize + ix as usize;
                    let (Some((lo, hi)), Some(p)) =
                        (self.column(i, &w), self.field.position(ix, iy))
                    else {
                        continue;
                    };
                    for (dx, dy, z) in [
                        (-half, -half, lo),
                        (half, -half, lo),
                        (-half, half, lo),
                        (half, half, lo),
                        (-half, -half, hi),
                        (half, -half, hi),
                        (-half, half, hi),
                        (half, half, hi),
                    ] {
                        let q = DVec3::new(
                            (p.x + dx).clamp(w.min.x, w.max.x),
                            (p.y + dy).clamp(w.min.y, w.max.y),
                            z,
                        );
                        b = grow(b, carry.to.transform_point3(q));
                    }
                }
                b
            })
            .reduce_with(|a, b| a.union(&b))
    }

    /// Its material carried by `carry`, as a stock in the frame it is carried
    /// into, on a grid of `pitch` over `onto`'s columns, with `floor` under
    /// it. `onto` is the box the material is carried into, as
    /// [`Stock::carried_bounds`] gives it, or one round that; nothing is
    /// carried outside it. The stock has an underside: each column stands
    /// between its own bottom and top, and is empty where nothing is carried.
    pub fn carried(
        &self,
        carry: &Carry,
        onto: Aabb,
        floor: f64,
        pitch: f64,
    ) -> Result<Stock, StockError> {
        if onto.max.z <= floor || !onto.max.z.is_finite() || !floor.is_finite() {
            return Err(StockError::Inverted {
                top: onto.max.z,
                floor,
            });
        }
        let mut field =
            HeightMap::covering(onto.min.truncate(), onto.max.truncate(), pitch, floor)?;
        let (origin, nx) = (field.origin(), field.nx() as usize);
        let back = carry.to.inverse();
        // The tool's axis, and the grid's X, in the frame the material was cut in.
        let down = back.matrix3 * DVec3::Z;
        let across = back.matrix3 * DVec3::X;
        let along = down.truncate().length() < ALONG;
        let (lo, hi) = (onto.min.z, onto.max.z);
        let mut tops = vec![f64::NAN; field.data().len()];
        let mut bottoms = vec![f64::NAN; field.data().len()];
        tops.par_chunks_mut(nx)
            .zip(bottoms.par_chunks_mut(nx))
            .enumerate()
            .for_each(|(iy, (tops, bottoms))| {
                let y = origin.y + f64::from(u32::try_from(iy).unwrap_or(u32::MAX)) * pitch;
                for ((top, bottom), ix) in tops.iter_mut().zip(bottoms).zip(0u32..) {
                    let p = DVec2::new(origin.x + f64::from(ix) * pitch, y);
                    let found = if along {
                        self.down_along(carry, back.transform_point3(p.extend(0.0)), across, pitch)
                    } else {
                        self.down_across(
                            carry,
                            back.transform_point3(p.extend(0.0)),
                            down,
                            (lo, hi),
                            pitch,
                        )
                    };
                    if let Some((b, t)) = found {
                        (*bottom, *top) = (b.max(lo), t.min(hi));
                    }
                }
            });
        if !along {
            (tops, bottoms) = dilated(&tops, &bottoms, nx);
        }
        let mut under = vec![narrow(floor); tops.len()];
        for ((slot, under), (t, b)) in field
            .data_mut()
            .iter_mut()
            .zip(&mut under)
            .zip(tops.iter().zip(&bottoms))
        {
            if t > b {
                *slot = narrow(*t);
                *under = narrow(*b);
            }
        }
        Ok(Stock {
            field,
            floor,
            under: Some(under.into()),
        })
    }

    /// The material a column carried along the tool's axis takes, its bottom
    /// and top in the frame it is carried into: every earlier column whose
    /// cell overlaps its own, its cell in that frame centred on `at`, `across`
    /// that frame's X there, `pitch` wide.
    fn down_along(
        &self,
        carry: &Carry,
        at: DVec3,
        across: DVec3,
        pitch: f64,
    ) -> Option<(f64, f64)> {
        let (o, p) = (self.field.origin(), self.field.pitch());
        let (nx, ny) = (self.field.nx(), self.field.ny());
        // Half the cell's width along the earlier X and Y, turned as it may
        // be, and half an earlier cell, a hair short so a cell only touching
        // is not taken.
        let reach = pitch * 0.5 * (across.x.abs() + across.y.abs()) + p * (0.5 - 1e-6);
        let span = |c: f64, origin: f64, n: u32| -> Option<(u32, u32)> {
            let first = ((c - reach - origin) / p).ceil().max(0.0);
            let last = ((c + reach - origin) / p).floor().min(f64::from(n) - 1.0);
            // Within the grid, so the narrowing is exact.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            (first.is_finite() && last.is_finite() && first <= last)
                .then_some((first as u32, last as u32))
        };
        let (x0, x1) = span(at.x, o.x, nx)?;
        let (y0, y1) = span(at.y, o.y, ny)?;
        let mut out: Option<(f64, f64)> = None;
        for iy in y0..=y1 {
            for ix in x0..=x1 {
                let i = iy as usize * nx as usize + ix as usize;
                let Some((b, t)) = self.column(i, &carry.within) else {
                    continue;
                };
                let z = |z: f64| carry.to.transform_point3(DVec3::new(at.x, at.y, z)).z;
                let (zb, zt) = (z(b), z(t));
                let (b, t) = (zb.min(zt), zb.max(zt));
                out = Some(out.map_or((b, t), |(ob, ot)| (ob.min(b), ot.max(t))));
            }
        }
        out
    }

    /// The material a column carried across the earlier ones takes, its
    /// bottom and top in the frame it is carried into, between `lo` and `hi`
    /// there: looked down from above and up from below, in steps of half the
    /// finer of the two grids, for the first material each way, from `at`
    /// along `down`, both in the earlier frame. A step's worth more each way,
    /// for what lies between two looks.
    fn down_across(
        &self,
        carry: &Carry,
        at: DVec3,
        down: DVec3,
        (lo, hi): (f64, f64),
        pitch: f64,
    ) -> Option<(f64, f64)> {
        let (o, p) = (self.field.origin(), self.field.pitch());
        let (nx, ny) = (self.field.nx(), self.field.ny());
        let step = 0.5 * p.min(pitch);
        let steps = bounded_count(((hi - lo) / step).ceil());
        let solid = |s: f64| {
            let q = at + down * s;
            let (fx, fy) = (((q.x - o.x) / p).round(), ((q.y - o.y) / p).round());
            if !(fx >= 0.0 && fy >= 0.0 && fx < f64::from(nx) && fy < f64::from(ny)) {
                return false;
            }
            // Within the grid, so the narrowing is exact.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let (ix, iy) = (fx as usize, fy as usize);
            self.column(iy * nx as usize + ix, &carry.within)
                .is_some_and(|(b, t)| q.z >= b - step && q.z <= t + step)
        };
        let at_step = |k: u32| f64::from(k) * step;
        let top = (0..=steps)
            .map(|k| (hi - at_step(k)).max(lo))
            .find(|s| solid(*s))?;
        let bottom = (0..=steps)
            .map(|k| (lo + at_step(k)).min(hi))
            .find(|s| solid(*s))
            .unwrap_or(top);
        Some((bottom - step, top + step))
    }
}

/// Each column's material grown to its neighbours': a cell takes the highest
/// top and lowest bottom of the material in it and the eight round it.
/// `tops` and `bottoms` are rows `nx` long, NaN where there is none.
fn dilated(tops: &[f64], bottoms: &[f64], nx: usize) -> (Vec<f64>, Vec<f64>) {
    let ny = tops.len() / nx.max(1);
    let mut out_t = vec![f64::NAN; tops.len()];
    let mut out_b = vec![f64::NAN; tops.len()];
    out_t
        .par_chunks_mut(nx)
        .zip(out_b.par_chunks_mut(nx))
        .enumerate()
        .for_each(|(iy, (row_t, row_b))| {
            for (ix, (t, b)) in row_t.iter_mut().zip(row_b).enumerate() {
                for jy in iy.saturating_sub(1)..=(iy + 1).min(ny - 1) {
                    for jx in ix.saturating_sub(1)..=(ix + 1).min(nx - 1) {
                        let j = jy * nx + jx;
                        let (Some(&nt), Some(&nb)) = (tops.get(j), bottoms.get(j)) else {
                            continue;
                        };
                        if nt > nb {
                            *t = if t.is_nan() { nt } else { t.max(nt) };
                            *b = if b.is_nan() { nb } else { b.min(nb) };
                        }
                    }
                }
            }
        });
    (out_t, out_b)
}
