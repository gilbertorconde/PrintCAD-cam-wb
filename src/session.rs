//! Playing a setup back at full resolution: its run kept in a helper that
//! stays running (printCAD's helper sessions), and the material drawn as a
//! mesh printCAD keeps, updated only where a seek changed it and coloured by
//! how it stands against the part.
//!
//! Until the session has run the setup again, and wherever it cannot, the
//! coarse moments of the run's own answer play instead.

use cam_protocol::{SessionIn, SessionOut, SessionReady, StockPatch, class, unbase64};
use printcad_bench_sdk::api::*;
use printcad_bench_sdk::{host, serde_json};

/// The material's colour, which tints the mesh: the stock's.
pub const TINT: Rgb = [0.78, 0.66, 0.5];

/// Each point's colour by [`class`], under the [`TINT`]: the stock as it is,
/// finished material towards the part's blue-grey, a gouge red. (printCAD
/// 0.11 draws a mesh's tint but not yet its vertex colours, printCAD #190,
/// so the tint alone shows until it does.)
const STOCK: Rgb = [1.0, 1.0, 1.0];
const FINISHED: Rgb = [0.72, 0.95, 1.0];
const GOUGE: Rgb = [1.0, 0.3, 0.3];

fn colour(c: u8) -> Rgb {
    match c {
        class::FINISHED => FINISHED,
        class::GOUGE => GOUGE,
        _ => STOCK,
    }
}

/// A setup's run, kept in a helper to play back.
pub struct Session {
    handle: u64,
    /// The setup, and the run it plays: a digest of what it was run from.
    pub setup: String,
    pub key: u64,
    ready: Option<(SessionReady, u64)>,
    /// A seek asked for and not yet answered.
    waiting: bool,
    /// The playhead the material was last asked for.
    asked: Option<f64>,
    /// Where the last answer stands.
    pub at: Option<StockPatch>,
    /// Why it stopped, when it did.
    pub failed: Option<String>,
}

impl Session {
    /// Start one for `setup`'s run `key`, made from `request`, the run's
    /// own request.
    pub fn open(setup: &str, key: u64, request: &str) -> Result<Self, String> {
        let handle = host::helper_open(cam_protocol::HELPER)?;
        // The request is already JSON: wrapped, it is the opening line.
        if let Err(e) = host::helper_send(handle, &format!("{{\"open\":{request}}}")) {
            host::helper_close(handle);
            return Err(e);
        }
        Ok(Self {
            handle,
            setup: setup.into(),
            key,
            ready: None,
            waiting: true,
            asked: None,
            at: None,
            failed: None,
        })
    }

    /// The grid's timing, once the session has run: when each move ends.
    pub fn times(&self) -> Option<&[Vec<f32>]> {
        self.ready.as_ref().map(|(r, _)| r.times.as_slice())
    }

    /// The kept mesh of the material, once it has heights.
    pub fn mesh(&self) -> Option<u64> {
        self.ready
            .as_ref()
            .filter(|_| self.at.is_some())
            .map(|(_, m)| *m)
    }

    /// Take what the helper said, and ask for the material at `playhead`
    /// when it is free and the playhead moved. Never waits.
    pub fn pump(&mut self, playhead: f64) {
        if self.failed.is_some() {
            return;
        }
        loop {
            match host::helper_receive(self.handle, 0) {
                Ok(Some(line)) => self.take(&line),
                Ok(None) => break,
                Err(e) => {
                    self.failed = Some(e);
                    return;
                }
            }
        }
        if self.ready.is_some() && !self.waiting && self.asked != Some(playhead) {
            let line =
                serde_json::to_string(&SessionIn::Seek { seconds: playhead }).unwrap_or_default();
            match host::helper_send(self.handle, &line) {
                Ok(()) => {
                    self.waiting = true;
                    self.asked = Some(playhead);
                }
                Err(e) => self.failed = Some(e),
            }
        }
    }

    fn take(&mut self, line: &str) {
        match serde_json::from_str::<SessionOut>(line) {
            Ok(SessionOut::Ready(ready)) => {
                self.waiting = false;
                match grid(&ready) {
                    Ok(mesh) => self.ready = Some((*ready, mesh)),
                    Err(e) => self.failed = Some(e),
                }
            }
            Ok(SessionOut::Stock(patch)) => {
                self.waiting = false;
                if let Some((ready, mesh)) = &self.ready
                    && let Err(e) = apply(ready, *mesh, &patch)
                {
                    self.failed = Some(e);
                }
                self.at = Some(*patch);
            }
            Ok(SessionOut::Refused { why }) => self.failed = Some(why),
            Err(e) => self.failed = Some(format!("the session's answer: {e}")),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        host::helper_close(self.handle);
        if let Some((_, mesh)) = &self.ready {
            host::mesh_remove(*mesh);
        }
    }
}

fn point(r: &SessionReady, col: u32, row: u32, h: u16) -> [f32; 3] {
    let z = r.low + r.step * f32::from(h);
    let (c, w) = (col as f32, row as f32);
    std::array::from_fn(|i| r.origin[i] + c * r.across[i] + w * r.along[i] + z * r.up[i])
}

/// The material's mesh for `ready`'s grid, flat at the stock's foot until
/// the first answer gives it heights.
fn grid(ready: &SessionReady) -> Result<u64, String> {
    let (cols, rows) = (ready.cols, ready.rows);
    if cols < 2 || rows < 2 {
        return Err("the material's grid is too small".into());
    }
    let positions: Vec<[f32; 3]> = (0..rows)
        .flat_map(|r| (0..cols).map(move |c| (c, r)))
        .map(|(c, r)| point(ready, c, r, 0))
        .collect();
    let colours = vec![STOCK; positions.len()];
    let mut indices = Vec::with_capacity(((cols - 1) * (rows - 1) * 6) as usize);
    for r in 0..rows - 1 {
        for c in 0..cols - 1 {
            let at = r * cols + c;
            indices.extend([at, at + 1, at + cols + 1, at, at + cols + 1, at + cols]);
        }
    }
    host::mesh_create(&positions, &indices, &colours)
}

/// The rectangle a seek changed, into the kept mesh.
fn apply(ready: &SessionReady, mesh: u64, patch: &StockPatch) -> Result<(), String> {
    if patch.width == 0 || patch.height == 0 {
        return Ok(());
    }
    let heights: Vec<u16> = unbase64(&patch.heights)
        .ok_or("the session's heights are not base64")?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_le_bytes(*b))
        .collect();
    let classes = unbase64(&patch.classes).ok_or("the session's classes are not base64")?;
    let (w, h) = (patch.width as usize, patch.height as usize);
    if heights.len() != w * h || classes.len() != w * h {
        return Err("the session's rectangle does not hold its points".into());
    }
    for r in 0..h {
        let row = patch.row + r as u32;
        let line = r * w..(r + 1) * w;
        let positions: Vec<[f32; 3]> = heights[line.clone()]
            .iter()
            .enumerate()
            .map(|(c, h)| point(ready, patch.col + c as u32, row, *h))
            .collect();
        let colours: Vec<Rgb> = classes[line].iter().map(|c| colour(*c)).collect();
        host::mesh_update(mesh, row * ready.cols + patch.col, &positions, &colours)?;
    }
    Ok(())
}
