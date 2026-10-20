//! What the CAM package asks its native helper, and what the helper answers.
//!
//! The package (WebAssembly, in printCAD's sandbox) writes a [`Request`] as JSON
//! on the helper's standard input; the helper runs the setup with camcraft on
//! every core and writes a [`Response`] as JSON on its standard output. Every
//! point is in the document's coordinates, millimetres, so the package draws
//! what it gets without knowing how the setup turned the part.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The helper's file name in the package's `helpers/<os>-<arch>/` folder
/// (`.exe` on Windows, which printCAD adds).
pub const HELPER: &str = "camcraft-helper";

/// Bumped whenever a request or response changes shape, so a package never
/// reads an answer from a helper of another build.
pub const VERSION: u32 = 1;

/// One setup to run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// [`VERSION`], as the package was built with.
    pub version: u32,
    /// A camcraft job holding one setup, as TOML (`camcraft_session::job_text`).
    /// Its models' paths are not read: their shapes are in [`Request::shapes`].
    pub job: String,
    /// Each model's solid, by the model's name in the job.
    pub shapes: BTreeMap<String, Shape>,
    /// The most points to send back for drawing one operation's toolpath; a
    /// denser one is thinned to it. Zero sends none.
    pub max_points: usize,
    /// The machine profile to post with, as camcraft's TOML, when it is not
    /// one camcraft has built in: the setup carries its own copy.
    #[serde(default)]
    pub post: Option<String>,
    /// Instead of running a setup, read this file and answer with its text
    /// ([`Response::Read`]): what a package in printCAD's sandbox cannot do.
    #[serde(default)]
    pub read: Option<String>,
}

/// A body's solid, as printCAD keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    /// The B-rep in ogeom's native format, in the body's own frame.
    pub text: String,
    /// Where the body sits: a row-major rigid 4x4 matrix, as
    /// `printcad_bench_sdk::api::Body::placement` has it.
    pub placement: [f64; 16],
}

/// The placement that leaves a body where it is.
pub const IDENTITY: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0,
];

/// What came of a [`Request`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum Response {
    /// The setup ran: its toolpaths, checks and programs.
    Ran(Box<Run>),
    /// It could not run, and why, as a person should read it.
    Refused {
        /// Why.
        why: String,
    },
    /// The file [`Request::read`] named, as text.
    Read { text: String },
}

/// A setup, run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Run {
    /// [`VERSION`], as the helper was built with.
    pub version: u32,
    /// Whether it may be posted: it verified against the material and no check
    /// fails. camcraft refuses to post otherwise.
    pub passed: bool,
    /// What the checks found, failures first.
    pub findings: Vec<Finding>,
    /// What happened, in order, for a log.
    pub notes: Vec<String>,
    /// Everything worth a person's attention.
    pub warnings: Vec<String>,
    /// The box around the stock, low and high corners.
    pub stock: [[f64; 3]; 2],
    /// The work zero.
    pub origin: [f64; 3],
    /// Each operation, in the setup's order.
    pub operations: Vec<OperationRun>,
    /// The material left when the setup is done, coarsely, for drawing.
    pub material: Option<Material>,
    /// The material and the tool along the run, for playing it back.
    #[serde(default)]
    pub playback: Option<Playback>,
    /// How long it took, in seconds.
    pub seconds: f64,
}

/// The material and the tool at moments along a setup's run, coarsely: a
/// grid over the stock whose heights change from moment to moment.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Playback {
    /// Each point of the grid at height zero, in the model's coordinates.
    pub base: Vec<[f32; 3]>,
    /// One unit up the tool's axis, in the model's coordinates.
    pub axis: [f32; 3],
    /// The grid's triangles.
    pub indices: Vec<u32>,
    /// A height is `low + step * h`, for an `h` of a moment's.
    pub low: f32,
    pub step: f32,
    pub moments: Vec<Moment>,
}

/// One moment of a [`Playback`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Moment {
    /// Which operation is cutting, from zero.
    pub op: usize,
    /// How many of its moves it has made, and of how many.
    pub moves: usize,
    pub of: usize,
    /// Where the tool's tip is, in the model's coordinates.
    pub tool: [f32; 3],
    /// How many of the operation's drawn points it has gone through.
    pub drawn: usize,
    /// When, in machining seconds from the setup's start.
    #[serde(default)]
    pub seconds: f64,
    /// The material's height at each point of the grid.
    pub heights: Vec<u16>,
}

/// How bad a finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Level {
    /// Refuses posting.
    Fail,
    /// Worth reading before cutting.
    Warn,
    /// For information.
    Note,
    /// A check that passed.
    Pass,
}

/// One thing a check found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// How bad.
    pub level: Level,
    /// camcraft's code, `CC107` say.
    pub code: String,
    /// The line, as camcraft prints it.
    pub text: String,
    /// The operation it is about, by its place in the setup.
    pub operation: Option<usize>,
    /// How many of that operation's moves had been made when it happened,
    /// for one found at a move.
    #[serde(default)]
    pub at_move: Option<usize>,
}

/// A place on an operation's timeline worth marking: a failed check, or a
/// warning or a steep descent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Mark {
    /// How many of the operation's moves had been made.
    pub moves: usize,
    /// When, in machining seconds from the setup's start.
    pub seconds: f64,
    /// A failure; a warning otherwise.
    pub fail: bool,
}

/// One operation, run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OperationRun {
    /// Its name.
    pub name: String,
    /// Whether it made a toolpath: `generated`, `skipped` or `disabled`.
    pub made: String,
    /// A line saying what it did.
    pub summary: String,
    /// Its own warnings.
    pub warnings: Vec<String>,
    /// Its toolpath for drawing, thinned to the request's budget.
    pub strokes: Vec<Stroke>,
    /// How many points the whole toolpath has, before thinning.
    pub points: usize,
    /// How far it cuts, in millimetres.
    pub cut_mm: f64,
    /// How far it moves at rapid, in millimetres.
    pub rapid_mm: f64,
    /// Its program, when it made a toolpath.
    pub program: Option<Program>,
    /// Its heights as camcraft resolved them, each a plane across the stock.
    #[serde(default)]
    pub heights: Vec<HeightPlane>,
    /// How long it machines, in seconds: its moves at their feeds, rapids at
    /// an assumed 5000 mm/min.
    #[serde(default)]
    pub seconds: f64,
    /// Where its checks found something, in order.
    #[serde(default)]
    pub marks: Vec<Mark>,
}

/// One of an operation's heights: clearance, retract, top or bottom.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HeightPlane {
    /// Which height: `Clearance`, `Retract`, `Top` or `Bottom`.
    pub name: String,
    /// How high it is above the work zero, along the tool's axis.
    pub z: f64,
    /// A rectangle across the stock at that height, a little bigger than it,
    /// in the model's coordinates.
    pub corners: [[f32; 3]; 4],
}

/// A run of moves of one kind.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    /// Rapid moves, or moves at a feed.
    pub rapid: bool,
    /// Its points, the first being where the moves start.
    pub points: Vec<[f32; 3]>,
}

/// A G-code program.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Program {
    /// Its file name, without the extension, as camcraft names it.
    pub name: String,
    /// The post's extension, `nc` or `tap` say.
    pub extension: String,
    /// The post it was written for.
    pub post: String,
    /// The program.
    pub text: String,
    /// How many lines it has.
    pub lines: usize,
}

/// The material's top, sampled coarsely, as triangles in the document's
/// coordinates.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Material {
    /// Corners.
    pub positions: Vec<[f32; 3]>,
    /// Three corners a triangle.
    pub indices: Vec<u32>,
}

// ------------------------------------------------------------- session

/// What the package says to a helper kept running for playback, one line
/// each: the setup to play back, then where to stand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionIn {
    /// Run this setup and keep it, to play back.
    Open(Box<Request>),
    /// The material, tool and toolpath at this many machining seconds from
    /// the setup's start.
    Seek { seconds: f64 },
}

/// What a playback session answers, one line each.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionOut {
    /// The setup ran and is kept: the material's grid, and when each move
    /// ends.
    Ready(Box<SessionReady>),
    /// It could not run, and why.
    Refused { why: String },
    /// Where a seek stands, and the material that changed.
    Stock(Box<StockPatch>),
}

/// A playback session's material grid and timing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionReady {
    /// Points across and along.
    pub cols: u32,
    pub rows: u32,
    /// Point `(c, r)` at height `h` sits at `origin + c * across + r *
    /// along + h * up`, in the model's coordinates.
    pub origin: [f32; 3],
    pub across: [f32; 3],
    pub along: [f32; 3],
    pub up: [f32; 3],
    /// A height is `low + step * h` for a packed `h`.
    pub low: f32,
    pub step: f32,
    /// When each move of each operation ends, in machining seconds from
    /// the setup's start.
    pub times: Vec<Vec<f32>>,
}

/// How a point of the material stands against the part.
pub mod class {
    /// Stock still on it.
    pub const STOCK: u8 = 0;
    /// Cut to the part, within the tolerance.
    pub const FINISHED: u8 = 1;
    /// Cut below the part.
    pub const GOUGE: u8 = 2;
    /// No part under it.
    pub const CLEAR: u8 = 3;
}

/// A seek's answer: where it stands, and a rectangle of the material grid
/// that changed (none when nothing did).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StockPatch {
    /// The machining seconds it stands at.
    pub seconds: f64,
    /// The operation under way, and how many of its moves are made, of how
    /// many.
    pub op: usize,
    pub moves: usize,
    pub of: usize,
    /// Where the tool's tip is, in the model's coordinates.
    pub tool: [f32; 3],
    /// How many of the operation's drawn points come before the tool.
    pub drawn: usize,
    pub col: u32,
    pub row: u32,
    pub width: u32,
    pub height: u32,
    /// The rectangle's heights, row by row, 16 bits each, little-endian,
    /// in base64.
    pub heights: String,
    /// Each point's [`class`], a byte each, in base64.
    pub classes: String,
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// `bytes` in base64, padded.
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(BASE64[(n >> (18 - 6 * i) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The bytes of base64 `text`; `None` when it is not base64.
pub fn unbase64(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| BASE64.iter().position(|&x| x == c).map(|v| v as u32);
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        let mut n = 0u32;
        for &c in &chunk[..4 - pad] {
            n = (n << 6) | value(c)?;
        }
        n <<= 6 * pad as u32;
        let got = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&got[..3 - pad]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_reads_back() {
        for n in 0..8 {
            let bytes: Vec<u8> = (0..n).map(|i| (i * 37 + 5) as u8).collect();
            assert_eq!(unbase64(&base64(&bytes)).as_deref(), Some(&bytes[..]));
        }
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(unbase64("TWE"), None);
    }

    #[test]
    fn a_session_line_reads_back() {
        let line = serde_json::to_string(&SessionIn::Seek { seconds: 1.5 }).unwrap();
        assert_eq!(line, r#"{"seek":{"seconds":1.5}}"#);
        assert_eq!(
            serde_json::from_str::<SessionIn>(&line).unwrap(),
            SessionIn::Seek { seconds: 1.5 }
        );
    }
}
