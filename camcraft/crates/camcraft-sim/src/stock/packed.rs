//! Stock kept small, for the keyframes and checkpoints a replay restores.

use std::sync::Arc;

use camcraft_geom::heightmap::HeightMap;
use glam::DVec2;

use super::{Axis, CellRect, Round, Stock, StockError};

/// A stock kept small: its heights run-length encoded, since machined material
/// is mostly flat (the untouched top, cleared floors, terraces), or kept as they
/// are when that would be larger. Bit for bit the stock it was made from.
#[derive(Debug, Clone, PartialEq)]
pub struct PackedStock {
    origin: DVec2,
    pitch: f64,
    nx: u32,
    ny: u32,
    floor: f64,
    cells: Packed,
    /// The stock's underside, shared with it: it never changes as it is cut.
    under: Option<Arc<[f32]>>,
    /// How its grid goes round a rotary, when it is on one.
    round: Option<Round>,
}

/// A stock's heights, packed.
#[derive(Debug, Clone, PartialEq)]
enum Packed {
    /// Runs of equal heights, as the bits of the height and how many.
    Runs(Vec<(u32, u32)>),
    /// Every height, when runs would take more.
    Raw(Vec<u32>),
}

impl PackedStock {
    /// `stock`, packed.
    pub fn pack(stock: &Stock) -> Self {
        let f = &stock.field;
        Self {
            origin: f.origin(),
            pitch: f.pitch(),
            nx: f.nx(),
            ny: f.ny(),
            floor: stock.floor,
            cells: Packed::of(f.data()),
            under: stock.under.clone(),
            round: stock.round,
        }
    }

    /// Bytes it holds.
    pub fn bytes(&self) -> usize {
        match &self.cells {
            Packed::Runs(r) => r.len() * 8,
            Packed::Raw(r) => r.len() * 4,
        }
    }

    /// The stock, unpacked.
    pub fn unpack(&self) -> Result<Stock, StockError> {
        let mut field = HeightMap::filled(self.origin, self.pitch, self.nx, self.ny, 0.0)?;
        for (slot, v) in field.data_mut().iter_mut().zip(self.cells.heights()) {
            *slot = v;
        }
        Ok(Stock {
            field,
            floor: self.floor,
            under: self.under.clone(),
            round: self.round,
        })
    }

    /// It as bytes, to keep in a file: [`PackedStock::from_bytes`] reads them
    /// back, bit for bit, on any machine.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.bytes() + 64);
        out.extend_from_slice(MAGIC);
        for v in [self.origin.x, self.origin.y, self.pitch, self.floor] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&self.nx.to_le_bytes());
        out.extend_from_slice(&self.ny.to_le_bytes());
        self.cells.write(&mut out);
        match &self.under {
            None => out.push(0),
            Some(under) => {
                out.push(1);
                Packed::of(under).write(&mut out);
            }
        }
        // Last, so a stock kept before rotaries were is read as it was.
        if let Some(round) = self.round {
            out.push(match round.round {
                Axis::X => 1,
                _ => 2,
            });
            out.extend_from_slice(&round.radius.to_le_bytes());
        }
        out
    }

    /// A stock kept by [`PackedStock::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, StockError> {
        let mut r = Reader(bytes);
        if r.take(MAGIC.len())? != MAGIC {
            return Err(StockError::Unreadable("not a kept stock"));
        }
        let (x, y, pitch, floor) = (r.f64()?, r.f64()?, r.f64()?, r.f64()?);
        let (nx, ny) = (r.u32()?, r.u32()?);
        let cells = Packed::read(&mut r)?;
        let count = nx as usize * ny as usize;
        if cells.len() != count {
            return Err(StockError::Unreadable("its cells are not its grid's"));
        }
        let under = match r.take(1)? {
            [0] => None,
            [1] => {
                let under = Packed::read(&mut r)?;
                if under.len() != count {
                    return Err(StockError::Unreadable("its underside is not its grid's"));
                }
                Some(under.heights().collect::<Arc<[f32]>>())
            }
            _ => {
                return Err(StockError::Unreadable(
                    "its underside is neither there nor not",
                ));
            }
        };
        let round = match r.0 {
            [] => None,
            [axis @ (1 | 2), ..] => {
                let round = if *axis == 1 { Axis::X } else { Axis::Y };
                r.take(1)?;
                Some(Round {
                    round,
                    radius: r.f64()?,
                })
            }
            _ => return Err(StockError::Unreadable("it goes on past its end")),
        };
        if !r.0.is_empty() {
            return Err(StockError::Unreadable("it goes on past its end"));
        }
        Ok(Self {
            origin: DVec2::new(x, y),
            pitch,
            nx,
            ny,
            floor,
            cells,
            under,
            round,
        })
    }

    /// Make `stock`, on the same grid, this one again, in place. Returns the
    /// cells that differed, or `None` when the grids are not the same.
    ///
    /// Row by row, and a row run by run: only the stretch of each that differs
    /// is written, and the cells changed are joined a row at a time.
    pub fn restore(&self, stock: &mut Stock) -> Option<Option<CellRect>> {
        let f = &stock.field;
        if self.nx != f.nx()
            || self.ny != f.ny()
            || self.pitch.to_bits() != f.pitch().to_bits()
            || self.origin != f.origin()
        {
            return None;
        }
        stock.floor = self.floor;
        stock.under.clone_from(&self.under);
        let nx = self.nx as usize;
        let rows = stock.field.data_mut().chunks_exact_mut(nx).zip(0u32..);
        let mut changed: Option<CellRect> = None;
        let mut mark = |iy: u32, (x0, x1): (usize, usize)| {
            // Within a row of a grid `nx` wide, so both fit.
            #[allow(clippy::cast_possible_truncation)]
            let r = CellRect {
                x0: x0 as u32,
                y0: iy,
                x1: x1 as u32,
                y1: iy,
            };
            changed = CellRect::join(changed, Some(r));
        };
        match &self.cells {
            Packed::Raw(raw) => {
                for ((row, iy), packed) in rows.zip(raw.chunks_exact(nx)) {
                    if let Some(span) = put_row(row, packed) {
                        mark(iy, span);
                    }
                }
            }
            Packed::Runs(runs) => {
                let mut runs = runs.iter().copied();
                let (mut run, mut left) = (0u32, 0usize);
                for (row, iy) in rows {
                    let mut span: Option<(usize, usize)> = None;
                    let mut at = 0;
                    while at < nx {
                        if left == 0 {
                            let Some((b, n)) = runs.next() else { break };
                            (run, left) = (b, n as usize);
                            continue;
                        }
                        let n = left.min(nx - at);
                        if let Some(cells) = row.get_mut(at..at + n)
                            && let Some((x0, x1)) = put_run(cells, run)
                        {
                            let (x0, x1) = (at + x0, at + x1);
                            span = Some(span.map_or((x0, x1), |(lo, _)| (lo, x1)));
                        }
                        at += n;
                        left -= n;
                    }
                    if let Some(span) = span {
                        mark(iy, span);
                    }
                }
            }
        }
        Some(changed)
    }
}

/// What a kept stock starts with.
const MAGIC: &[u8] = b"camcraft-stock 1\n";

impl Packed {
    /// `data`, as runs, or raw where runs would be larger.
    fn of(data: &[f32]) -> Self {
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for v in data {
            let bits = v.to_bits();
            match runs.last_mut() {
                Some((b, n)) if *b == bits && *n < u32::MAX => *n += 1,
                _ => runs.push((bits, 1)),
            }
            // Past half as many runs as cells, runs are larger than the cells.
            if runs.len() > data.len() / 2 + 1 {
                return Packed::Raw(data.iter().map(|v| v.to_bits()).collect());
            }
        }
        runs.shrink_to_fit();
        Packed::Runs(runs)
    }

    /// How many heights it holds.
    fn len(&self) -> usize {
        match self {
            Packed::Runs(r) => r.iter().map(|(_, n)| *n as usize).sum(),
            Packed::Raw(r) => r.len(),
        }
    }

    /// Every height, in order.
    fn heights(&self) -> impl Iterator<Item = f32> + '_ {
        let (runs, raw): (&[(u32, u32)], &[u32]) = match self {
            Packed::Runs(r) => (r, &[]),
            Packed::Raw(r) => (&[], r),
        };
        runs.iter()
            .flat_map(|&(bits, n)| std::iter::repeat_n(f32::from_bits(bits), n as usize))
            .chain(raw.iter().map(|b| f32::from_bits(*b)))
    }

    /// Its kind, its count and its words, little-endian.
    fn write(&self, out: &mut Vec<u8>) {
        let words: Vec<u32> = match self {
            Packed::Runs(r) => r.iter().flat_map(|&(b, n)| [b, n]).collect(),
            Packed::Raw(r) => r.clone(),
        };
        out.push(u8::from(matches!(self, Packed::Raw(_))));
        out.extend_from_slice(&(words.len() as u64).to_le_bytes());
        for w in words {
            out.extend_from_slice(&w.to_le_bytes());
        }
    }

    /// One [`Packed::write`] wrote.
    fn read(r: &mut Reader<'_>) -> Result<Self, StockError> {
        let raw = match r.take(1)? {
            [0] => false,
            [1] => true,
            _ => return Err(StockError::Unreadable("its cells are of no kind it knows")),
        };
        let count = usize::try_from(r.u64()?)
            .map_err(|_| StockError::Unreadable("it has more cells than fit"))?;
        // Each word four bytes, all of them there, before any is kept.
        if count.checked_mul(4).is_none_or(|b| b > r.0.len()) {
            return Err(StockError::Unreadable("it ends before its cells do"));
        }
        let mut words = Vec::with_capacity(count);
        for _ in 0..count {
            words.push(r.u32()?);
        }
        if raw {
            return Ok(Packed::Raw(words));
        }
        if words.len() % 2 != 0 {
            return Err(StockError::Unreadable("a run has no length"));
        }
        Ok(Packed::Runs(
            words
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&[b, n]| (b, n))
                .collect(),
        ))
    }
}

/// Bytes read from the front.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], StockError> {
        if n > self.0.len() {
            return Err(StockError::Unreadable("it ends too soon"));
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn u32(&mut self) -> Result<u32, StockError> {
        let mut b = [0; 4];
        b.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(b))
    }

    fn u64(&mut self) -> Result<u64, StockError> {
        let mut b = [0; 8];
        b.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(b))
    }

    fn f64(&mut self) -> Result<f64, StockError> {
        let mut b = [0; 8];
        b.copy_from_slice(self.take(8)?);
        Ok(f64::from_le_bytes(b))
    }
}

/// Set `cells` to the height whose bits are `run`, from the first that differs
/// to the last: which cells those were, or `None` when none differ.
fn put_run(cells: &mut [f32], run: u32) -> Option<(usize, usize)> {
    let first = cells.iter().position(|v| v.to_bits() != run)?;
    let last = cells
        .iter()
        .rposition(|v| v.to_bits() != run)
        .unwrap_or(first);
    if let Some(changed) = cells.get_mut(first..=last) {
        changed.fill(f32::from_bits(run));
    }
    Some((first, last))
}

/// Set `cells` to the heights whose bits are `packed`, from the first that
/// differs to the last: which cells those were, or `None` when none differ.
fn put_row(cells: &mut [f32], packed: &[u32]) -> Option<(usize, usize)> {
    let differs = |(v, b): (&f32, &u32)| v.to_bits() != *b;
    let first = cells.iter().zip(packed).position(differs)?;
    let last = cells.iter().zip(packed).rposition(differs).unwrap_or(first);
    if let (Some(changed), Some(packed)) = (cells.get_mut(first..=last), packed.get(first..=last)) {
        for (v, b) in changed.iter_mut().zip(packed) {
            *v = f32::from_bits(*b);
        }
    }
    Some((first, last))
}
