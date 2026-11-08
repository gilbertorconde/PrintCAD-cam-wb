//! Material on a rotary: a round bar turned about its axis under a tool that
//! points at the axis, held as the bar unrolled.
//!
//! The grid's one axis runs along the bar and the other round it, a
//! millimetre of it a millimetre round at [`Round::radius`], one turn
//! exactly; a column's height is how far the material reaches out from the
//! axis there. A path is in the same unrolled frame (along, round, out from
//! the axis), which is how the machine moves it: X, the rotary's angle and Z
//! each straight from one end of a move to the other.
//!
//! The tool stands on a radius at its tip's angle. A column at an angle
//! `dθ` round from it is a ray out from the axis, which crosses the tool's
//! side `ρ sin dθ` across from its axis at `ρ cos dθ` up it: the tool takes
//! the ray from the first `ρ` its underside reaches. A column the tool's side
//! leaves before its top, as beside a wall it cuts, keeps its material: one
//! height a column cannot hold the air the tool leaves inside it, so less is
//! taken there than was, never more.

use std::f64::consts::{FRAC_PI_2, TAU};

use camcraft_geom::heightmap::{cell_index, narrow};
use glam::DVec3;

use rayon::prelude::*;

use super::{Axis, CellRect, Stock, StockError, SweptTool};

/// Cells a stamp's footprint must span before its rows are worked out in
/// parallel.
const PARALLEL_CELLS: usize = 20_000;

/// Where one stamp of a tool is, on a rotary: along the bar, round it and
/// out from its axis, and the first and last cell along it reaches.
struct Stamp {
    along: f64,
    at: f64,
    r0: f64,
    a0: u32,
    a1: u32,
}

/// The cells of a row round the bar that stamps lowered: each one's place
/// along it, what is left, and the most one stamp took.
type Lowered = Vec<(u32, f64, f64)>;

/// How a stock's grid goes round a rotary's axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Round {
    /// The grid's axis that goes round: Y for a bar lying along X, which an
    /// A axis turns, and X for one along Y, which a B axis turns.
    pub round: Axis,
    /// The radius at which a millimetre of the grid round is a millimetre
    /// round the bar: the radius the paths are unrolled at.
    pub radius: f64,
}

impl Round {
    /// A whole turn, in millimetres of the grid.
    pub fn period(&self) -> f64 {
        TAU * self.radius
    }

    /// `p`'s coordinates along the bar, round it and out from its axis.
    pub fn split(&self, p: DVec3) -> (f64, f64, f64) {
        match self.round {
            Axis::X => (p.y, p.x, p.z),
            _ => (p.x, p.y, p.z),
        }
    }
}

/// Cells in a turn of a grid at `pitch`, if a whole number of them make one
/// to within rounding.
fn cells_round(round: &Round, pitch: f64) -> Option<u32> {
    let n = (round.period() / pitch).round();
    let exact = (n * pitch - round.period()).abs() <= 1e-6 * round.period();
    // Bounded and whole, so the narrowing is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (exact && n >= 3.0 && n < f64::from(u32::MAX)).then_some(n as u32)
}

/// The pitch nearest `wanted`, and no coarser, that makes a whole turn of
/// `round` a whole number of cells.
pub fn round_pitch(round: &Round, wanted: f64) -> f64 {
    let n = (round.period() / wanted).ceil().max(3.0);
    round.period() / n
}

impl Stock {
    /// The stock on a rotary, its grid's `round.round` axis going once round
    /// it from the grid's first cell: refused unless a whole number of cells
    /// make a turn, and the grid holds at least one.
    pub fn rounded(mut self, round: Round) -> Result<Self, StockError> {
        let pitch = self.field.pitch();
        let across = match round.round {
            Axis::X => self.field.nx(),
            _ => self.field.ny(),
        };
        match cells_round(&round, pitch) {
            Some(n) if across >= n && self.under.is_none() => {
                self.round = Some(round);
                Ok(self)
            }
            _ => Err(StockError::Unreadable(
                "a rotary's stock must go once round in whole cells",
            )),
        }
    }

    /// How the grid goes round a rotary, when the stock is on one.
    pub fn round(&self) -> Option<Round> {
        self.round
    }

    /// Lower the columns a tool with its tip at `tip`, in the unrolled frame,
    /// cuts on a rotary: how far it cut the deepest, and which cells it
    /// lowered. The grid's cells past a whole turn are the first ones again,
    /// and are lowered with them.
    pub(super) fn stamp_round<T: SweptTool>(
        &mut self,
        tip: DVec3,
        tool: &T,
        round: Round,
    ) -> (f64, Option<CellRect>) {
        self.stamp_round_all(&[tip], tool, round)
    }

    /// [`Stock::stamp_round`] at each of `tips` in turn, as one: the same
    /// cells lowered as far, how far the deepest was cut by any one of them.
    ///
    /// Each column is cut by the tips in order, and by nothing but its own
    /// height and theirs, so the rows round the bar are worked out apart: in
    /// parallel, where a move's stamps cover many cells between them, as a
    /// long pass's do. Stamped one at a time, each covers too few to share
    /// out, and a pass on a rotary is all stamps.
    pub(super) fn stamp_round_all<T: SweptTool>(
        &mut self,
        tips: &[DVec3],
        tool: &T,
        round: Round,
    ) -> (f64, Option<CellRect>) {
        let rt = tool.max_radius();
        let pitch = self.field.pitch();
        let Some(turn) = cells_round(&round, pitch) else {
            return (0.0, None);
        };
        if !rt.is_finite() || rt <= 0.0 {
            return (0.0, None);
        }
        let origin = self.field.origin();
        let na = match round.round {
            Axis::X => self.field.ny(),
            _ => self.field.nx(),
        };
        let (oa, or) = match round.round {
            Axis::X => (origin.y, origin.x),
            _ => (origin.x, origin.y),
        };
        // Each tip's place, and the cells along and the rows round it reaches.
        let mut stamps: Vec<Stamp> = Vec::with_capacity(tips.len());
        // Each row of a turn: the stamps that reach it, in order, and the row
        // round from the first each is, unwrapped.
        let mut rows: Vec<Vec<(u32, f64)>> = Vec::new();
        let mut cells = 0usize;
        for &tip in tips {
            if !tip.is_finite() {
                continue;
            }
            let (along, at, r0) = round.split(tip);
            // How far round a column can be and still meet the tool: its side
            // is `rt` across, and nothing it touches is nearer the axis than
            // its tip.
            let reach = if r0 > rt * 1e-3 {
                (rt / r0).atan().min(FRAC_PI_2)
            } else {
                FRAC_PI_2
            };
            let a0 = cell_index(((along - rt - oa) / pitch).floor(), na);
            let a1 = cell_index(((along + rt - oa) / pitch).ceil(), na);
            let span = reach * round.radius;
            let k0 = ((at - span - or) / pitch).floor();
            let k1 = ((at + span - or) / pitch).ceil();
            if !k0.is_finite() || !k1.is_finite() {
                continue;
            }
            // Never more than a turn: a column is one ray, whichever way round.
            let k1 = k1.min(k0 + f64::from(turn) - 1.0);
            let Ok(n) = u32::try_from(stamps.len()) else {
                break;
            };
            if rows.is_empty() {
                rows = vec![Vec::new(); turn as usize];
            }
            let mut k = k0;
            while k <= k1 {
                // Bounded above by a turn, and wrapped into it, so the
                // narrowing is exact.
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let ir = k.rem_euclid(f64::from(turn)) as usize;
                if let Some(row) = rows.get_mut(ir) {
                    row.push((n, k));
                    cells += (a1 - a0 + 1) as usize;
                }
                k += 1.0;
            }
            stamps.push(Stamp {
                along,
                at,
                r0,
                a0,
                a1,
            });
        }
        let floor = self.floor;
        let field = &self.field;
        // One row round: each cell along it cut by the stamps that reach it,
        // in order, each from where the one before left it; what is left in
        // each cell it lowered, and the most one stamp took there.
        let row = |(ir, reaching): (u32, &Vec<(u32, f64)>)| -> Lowered {
            let mut out: Lowered = Vec::new();
            let (Some(lo), Some(hi)) = (
                reaching
                    .iter()
                    .filter_map(|(n, _)| stamps.get(*n as usize))
                    .map(|s| s.a0)
                    .min(),
                reaching
                    .iter()
                    .filter_map(|(n, _)| stamps.get(*n as usize))
                    .map(|s| s.a1)
                    .max(),
            ) else {
                return out;
            };
            // Each cell's height as the stamps before left it, and the most
            // one took; NaN until one cuts it.
            let mut now = vec![(f64::NAN, 0.0f64); (hi - lo + 1) as usize];
            for &(n, k) in reaching {
                let Some(st) = stamps.get(n as usize) else {
                    continue;
                };
                let s = or + k * pitch;
                let dth = (s - st.at) / round.radius;
                let (sin, cos) = dth.sin_cos();
                if cos <= 1e-9 {
                    continue;
                }
                for ia in st.a0..=st.a1 {
                    let dx = oa + f64::from(ia) * pitch - st.along;
                    if dx.abs() > rt {
                        continue;
                    }
                    let Some(slot) = now.get_mut((ia - lo) as usize) else {
                        continue;
                    };
                    let cur = if slot.0.is_nan() {
                        let (ix, iy) = match round.round {
                            Axis::X => (ir, ia),
                            _ => (ia, ir),
                        };
                        match field.get(ix, iy) {
                            Some(h) => h,
                            None => continue,
                        }
                    } else {
                        slot.0
                    };
                    // The first point of the ray in the tool: from its tip's
                    // height up, the tool's underside rising as the ray goes
                    // out across it. Each step is no further out than the
                    // answer, so it is never past it.
                    let mut rho = st.r0 / cos;
                    let mut inside = true;
                    for _ in 0..12 {
                        let d = dx.hypot(rho * sin);
                        let Some(h) = tool.height_at(d) else {
                            inside = false;
                            break;
                        };
                        let next = (st.r0 + h) / cos;
                        let done = next - rho < 1e-7;
                        rho = next;
                        if done {
                            break;
                        }
                    }
                    if !inside || rho >= cur {
                        continue;
                    }
                    // Out past where the ray leaves the tool's side, the
                    // material is beside the tool, not under it.
                    if sin.abs() > 1e-12 && (rt * rt - dx * dx).max(0.0).sqrt() / sin.abs() < cur {
                        continue;
                    }
                    let left = f64::from(narrow(rho.max(floor)));
                    *slot = (left, slot.1.max(cur - left));
                }
            }
            for (ia, (left, took)) in (lo..).zip(now) {
                if !left.is_nan() {
                    out.push((ia, left, took));
                }
            }
            out
        };
        let busy = (0..turn).zip(&rows).filter(|(_, r)| !r.is_empty());
        let cuts: Vec<(u32, Lowered)> = if cells >= PARALLEL_CELLS {
            busy.collect::<Vec<_>>()
                .into_par_iter()
                .map(|r| (r.0, row(r)))
                .collect()
        } else {
            busy.map(|r| (r.0, row(r))).collect()
        };
        let mut deepest = 0.0f64;
        let mut touched: Option<CellRect> = None;
        let at = |ir: u32, ia: u32| match round.round {
            Axis::X => (ir, ia),
            _ => (ia, ir),
        };
        for (ir, cut) in cuts {
            for (ia, left, took) in cut {
                let (ix, iy) = at(ir, ia);
                self.field.set(ix, iy, left);
                deepest = deepest.max(took);
                touched = CellRect::join(touched, Some(CellRect::cell(ix, iy)));
                // The same column again past a whole turn.
                let (jx, jy) = at(ir + turn, ia);
                if self.field.get(jx, jy).is_some() {
                    self.field.set(jx, jy, left);
                    touched = CellRect::join(touched, Some(CellRect::cell(jx, jy)));
                }
            }
        }
        (deepest, touched)
    }

    /// How far apart a move's stamps go on a rotary, for a tool `radius`
    /// across, on a grid at `pitch` going round at `round`'s radius: a cell
    /// half way between two stamps is cut, by a ball or a flat, under a
    /// quarter of a cell less deep than by the nearest stamp, as its ray is
    /// that much further round from the tool, so stamps closer than that
    /// cut no more than a cell can show.
    pub(super) fn round_step(pitch: f64, radius: f64, round: &Round) -> f64 {
        let bend = 1.0 / round.radius.max(1e-6) + 1.0 / radius.max(1e-6);
        (2.0 * (pitch / (2.0 * bend)).sqrt()).clamp(pitch * 0.5, (radius * 0.5).max(pitch * 0.5))
    }

    /// The material's volume on a rotary, in cubic millimetres: each column
    /// a wedge from the axis out to its height.
    pub(super) fn round_volume(&self, round: Round) -> f64 {
        let pitch = self.field.pitch();
        let Some(turn) = cells_round(&round, pitch) else {
            return 0.0;
        };
        let (nx, ny) = (self.field.nx(), self.field.ny());
        let mut sum = 0.0;
        for iy in 0..ny {
            for ix in 0..nx {
                let ir = match round.round {
                    Axis::X => ix,
                    _ => iy,
                };
                if ir >= turn {
                    continue;
                }
                if let Some(h) = self.field.get(ix, iy).filter(|h| h.is_finite()) {
                    let h = h.max(0.0);
                    sum += 0.5 * h * h;
                }
            }
        }
        // A cell is `pitch` along and `pitch / radius` radians round.
        sum * pitch * pitch / round.radius
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::panic,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
mod tests {
    use glam::{DVec2, dvec3};

    use super::*;
    use crate::stock::{MoveClass, SimMove};

    /// A flat end mill: level across its radius.
    struct Flat(f64);

    impl SweptTool for Flat {
        fn max_radius(&self) -> f64 {
            self.0
        }
        fn height_at(&self, d: f64) -> Option<f64> {
            (d <= self.0).then_some(0.0)
        }
    }

    /// A bar 20 long and 10 in radius along X, unrolled at its own radius.
    fn bar() -> Stock {
        let round = Round {
            round: Axis::Y,
            radius: 10.0,
        };
        let pitch = round_pitch(&round, 0.25);
        let half = round.period() / 2.0;
        Stock::block(
            DVec2::new(0.0, -half),
            DVec2::new(20.0, half),
            10.0,
            -1.0,
            pitch,
        )
        .unwrap()
        .rounded(round)
        .unwrap()
    }

    #[test]
    fn a_turn_must_be_whole_cells() {
        let round = Round {
            round: Axis::Y,
            radius: 10.0,
        };
        let stock = Stock::block(DVec2::ZERO, DVec2::new(20.0, 70.0), 10.0, -1.0, 0.3).unwrap();
        assert!(stock.rounded(round).is_err());
        let p = round_pitch(&round, 0.3);
        assert!(p <= 0.3 && ((round.period() / p).round() * p - round.period()).abs() < 1e-9);
    }

    #[test]
    fn its_volume_is_a_cylinders() {
        let stock = bar();
        let want = std::f64::consts::PI * 100.0 * 20.0;
        // The grid's columns go a cell past each end.
        let got = stock.volume();
        assert!((got / want - 1.0).abs() < 0.03, "{got} {want}");
    }

    /// A flat end mill down at radius 8 over the top takes the column under
    /// its axis to 8, and one round from it only as low as its flat bottom
    /// comes on that ray: 8 over the cosine.
    #[test]
    fn a_flat_cuts_a_chord_across_the_bar() {
        let mut stock = bar();
        let tool = Flat(3.0);
        let path = [
            SimMove {
                to: dvec3(10.0, 0.0, 12.0),
                class: MoveClass::Rapid,
                arc: None,
            },
            SimMove {
                to: dvec3(10.0, 0.0, 8.0),
                class: MoveClass::MayCut,
                arc: None,
            },
        ];
        let report = stock.apply(&path, &tool);
        assert_eq!(report.rapid_contacts, 0);
        let field = stock.field();
        let at = |s: f64| {
            let iy = ((s - field.origin().y) / field.pitch()).round() as u32;
            let ix = ((10.0 - field.origin().x) / field.pitch()).round() as u32;
            let s = field.origin().y + f64::from(iy) * field.pitch();
            (field.get(ix, iy).unwrap(), s)
        };
        let (h, _) = at(0.0);
        assert!((h - 8.0).abs() < 1e-4, "{h}");
        let (h, s) = at(2.0);
        let want = 8.0 / (s / 10.0).cos();
        assert!((h - want).abs() < 1e-4, "{h} {want}");
        // Past where its side is, the bar is whole.
        let (h, _) = at(5.0);
        assert_eq!(h, 10.0);
        assert!(report.removed_mm3 > 0.0);
    }

    /// A pass along the bar at a constant radius, and one round it across the
    /// seam, cut where they go: the seam is no edge.
    #[test]
    fn a_cut_across_the_seam_is_one_cut() {
        let mut stock = bar();
        let tool = Flat(1.0);
        let half = std::f64::consts::PI * 10.0;
        let path = [
            SimMove {
                to: dvec3(5.0, half - 3.0, 12.0),
                class: MoveClass::Rapid,
                arc: None,
            },
            SimMove {
                to: dvec3(5.0, half - 3.0, 9.0),
                class: MoveClass::MayCut,
                arc: None,
            },
            SimMove {
                to: dvec3(5.0, half + 3.0, 9.0),
                class: MoveClass::MayCut,
                arc: None,
            },
        ];
        stock.apply(&path, &tool);
        let f = stock.field();
        let ix = ((5.0 - f.origin().x) / f.pitch()).round() as u32;
        // Either side of the seam, at the first row and the last.
        for iy in [0, 1, f.ny() - 2, f.ny() - 1] {
            let h = f.get(ix, iy).unwrap();
            // A stamp's flat floor, unrolled, rises round from it: by a
            // fraction of a cell between two.
            assert!((h - 9.0).abs() < 0.02, "{iy}: {h}");
        }
        // A rapid back through it is a contact.
        let back = [
            SimMove {
                to: dvec3(5.0, half + 3.0, 9.0),
                class: MoveClass::Rapid,
                arc: None,
            },
            SimMove {
                to: dvec3(5.0, 0.0, 9.0),
                class: MoveClass::Rapid,
                arc: None,
            },
        ];
        assert_eq!(stock.apply(&back, &tool).rapid_contacts, 1);
    }

    /// A move's stamps cut as one leave every cell as they do one by one,
    /// back over cells an earlier stamp lowered, and say as deep and where,
    /// whether shared out over many cells or not.
    #[test]
    fn stamps_as_one_are_stamps_one_by_one() {
        let tool = Flat(1.5);
        let tips: Vec<DVec3> = (0..400)
            .map(|i| {
                let t = f64::from(i) / 399.0;
                dvec3(4.0 + 12.0 * t, 25.0 + 30.0 * t, 9.5 - 1.5 * (t * 7.0).sin())
            })
            .collect();
        let round = bar().round().unwrap();
        // Hundreds of stamps, shared out; a few, not.
        for count in [tips.len(), 5] {
            let mut all = bar();
            let mut each = bar();
            let (mut deepest, mut touched) = (0.0f64, None);
            for &tip in tips.iter().take(count) {
                let (d, t) = each.stamp_round(tip, &tool, round);
                deepest = deepest.max(d);
                touched = CellRect::join(touched, t);
            }
            let some: Vec<DVec3> = tips.iter().take(count).copied().collect();
            let (d, t) = all.stamp_round_all(&some, &tool, round);
            assert_eq!(all.field().data(), each.field().data(), "{count}");
            assert_eq!((d, t), (deepest, touched), "{count}");
            assert!(deepest > 0.5 && touched.is_some(), "{count}: {deepest}");
        }
    }

    #[test]
    fn it_is_kept_round() {
        let stock = bar();
        let packed = crate::stock::PackedStock::pack(&stock);
        let again = crate::stock::PackedStock::from_bytes(&packed.to_bytes())
            .unwrap()
            .unpack()
            .unwrap();
        assert_eq!(again.round(), stock.round());
        assert_eq!(again, stock);
    }
}
