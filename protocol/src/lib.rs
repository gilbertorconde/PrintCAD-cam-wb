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
pub const VERSION: u32 = 9;

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
    /// The earlier setup this one carries on from, as it is run on its own:
    /// this one's stock is the material that one leaves, which the helper
    /// keeps from when it ran it, or runs it again for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Box<Request>>,
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
    /// The box around the stock, as the setup sees it, by its eight corners:
    /// corner `i` is at the box's high X where `i & 1` is set, its high Y
    /// where `i & 2` is and its high Z where `i & 4` is. Square to the
    /// document's axes unless the setup is tilted.
    pub stock: [[f64; 3]; 8],
    /// The stock, when it is a round bar: drawn as one, inside its box.
    #[serde(default)]
    pub cylinder: Option<Cylinder>,
    /// The work zero.
    pub origin: [f64; 3],
    /// Each operation, in the setup's order.
    pub operations: Vec<OperationRun>,
    /// The material left when the setup is done, coarsely, for drawing.
    pub material: Option<Material>,
    /// The material and the tool along the run, for playing it back.
    #[serde(default)]
    pub playback: Option<Playback>,
    /// The setup's program, every operation in it, when the setup is posted
    /// as one; then no operation has a program of its own.
    #[serde(default)]
    pub program: Option<Program>,
    /// How long it took, in seconds.
    pub seconds: f64,
    /// When each move ends, as one number ([`timing`]): a playback session
    /// says the same of its run, so the package knows the session's moves
    /// are the ones it shows.
    #[serde(default)]
    pub timing: u64,
    /// How long it machines, as its machine profile times it.
    #[serde(default)]
    pub machining: Machining,
    /// The tools its operations cut with, in the order they first cut.
    #[serde(default)]
    pub tools: Vec<ToolUse>,
    /// Where the part and its stock are, for the setup sheet.
    #[serde(default)]
    pub layout: Option<Layout>,
}

/// How long a setup machines: its moves at their feeds and the machine's
/// rapid rates, and its tool changes. Nothing speeds up or slows down, so a
/// real machine takes longer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Machining {
    /// Its moves', every operation's [`OperationRun::seconds`] together.
    pub moves: f64,
    /// How many times its programs change the tool, the first included.
    pub tool_changes: u32,
    /// How long a change takes, in seconds; `None` when the profile does
    /// not say, and the changes are left out.
    pub tool_change: Option<f64>,
    /// The rapid rates it was timed at, X, Y and Z, in mm/min.
    pub rapid: [f64; 3],
    /// Whether they are the profile's own; else assumed.
    pub rapid_given: bool,
}

impl Machining {
    /// The whole: its moves and its tool changes, in seconds.
    pub fn total(&self) -> f64 {
        self.moves + f64::from(self.tool_changes) * self.tool_change.unwrap_or(0.0)
    }
}

/// A tool a setup cuts with, as its setup sheet lists it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolUse {
    /// Its name in the setup.
    pub name: String,
    /// Its number in the program.
    pub number: u32,
    /// Its kind, as a job file names it: `flat`, `ball`, `bullnose`,
    /// `tapered`, `face-mill`, `lollipop`, `slot-cutter`, `cone`,
    /// `spot-drill`, `countersink`, `drill`, `tap`, `thread-mill` or
    /// `probe`.
    pub kind: String,
    /// Its shape past its kind and diameter, in words: a corner, angles, a
    /// neck, a tap's pitch and hand. Empty where it has none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub shape: String,
    /// Its diameter, and a bull-nose's corner radius, in millimetres.
    pub diameter: f64,
    pub corner: f64,
    /// A cone's or a drill's included angle, in degrees.
    pub angle: Option<f64>,
    /// The pitches a thread mill cuts, finest and coarsest, in millimetres.
    pub pitch_range: Option<[f64; 2]>,
    pub flutes: Option<u32>,
    /// How long its cutting edges are, how far it sticks out of its
    /// holder, and its overall length, in millimetres, where known.
    pub flute_length: Option<f64>,
    pub stickout: Option<f64>,
    pub overall_length: Option<f64>,
    /// What holds it, in words, if it has a holder.
    pub holder: Option<String>,
    /// Its description.
    pub comment: String,
    /// The operations that cut with it, by their place in the setup.
    pub operations: Vec<usize>,
    /// The lowest its tip goes, from the work zero along the tool's axis.
    pub lowest: Option<f64>,
    /// The fastest it is fed, mm/min, and turned, rev/min.
    pub fastest_feed: f64,
    pub fastest_rpm: u32,
    /// How long it machines, in seconds.
    pub seconds: f64,
}

/// What a setup sheet says of how an operation cuts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Cutting {
    /// Its strategy, as a job file names it: `adaptive`.
    pub strategy: String,
    /// Its tool's name in the setup, and number.
    pub tool: String,
    pub tool_number: u32,
    /// The spindle's speed, rev/min.
    pub rpm: u32,
    /// Its cutting and plunge feeds, and the fastest any move is fed, in
    /// mm/min.
    pub feed: f64,
    pub plunge_feed: f64,
    pub fastest_feed: f64,
    /// The distance between its passes, where it has one.
    pub stepover: Option<f64>,
    /// The deepest each of its levels may be, where it cuts in levels.
    pub stepdown: Option<f64>,
    /// The stock it leaves beside and below, where it leaves any.
    pub stock_to_leave: Option<[f64; 2]>,
    /// The lowest and highest its tip goes, from the work zero along the
    /// tool's axis.
    pub z: Option<[f64; 2]>,
    /// Its strategy's own settings worth reading on the shop floor, in
    /// words: a bore's or a thread's pitch, a tap, rest machining, corner
    /// relief, a pattern's copies.
    pub notes: Vec<String>,
}

/// Where a setup's part and stock are, as its setup sheet says.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// The stock's lowest and highest corners, from the work zero along the
    /// setup's axes, in millimetres.
    pub stock: [[f64; 3]; 2],
    /// The bodies' the same way.
    pub part: [[f64; 3]; 2],
    /// The tool's axis, pointing up it, in the document's coordinates.
    pub tool_axis: [f64; 3],
    /// What its programs' headers say of a tilt: none for a part square up
    /// its Z.
    pub tilt: Vec<String>,
    /// The post's name, as its profile gives it.
    pub post: String,
    /// The day it ran, `2026-11-07`, in UTC.
    pub date: String,
    /// The probe results its depths are corrected to: their file, grid and
    /// heights; `None` when they are not.
    #[serde(default)]
    pub probe: Option<String>,
}

/// A round bar of stock, in the document's coordinates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Cylinder {
    /// The middles of its two ends.
    pub ends: [[f64; 3]; 2],
    pub radius: f64,
}

/// The material and the tool at moments along a setup's run, coarsely: a
/// grid over the stock whose heights change from moment to moment.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Playback {
    /// Each point of the grid at height zero, in the model's coordinates.
    pub base: Vec<[f32; 3]>,
    /// One unit up the tool's axis, in the model's coordinates.
    pub axis: [f32; 3],
    /// For a setup on a rotary, one unit out from its axis at each point
    /// of the grid, which then stands on the axis; empty for `axis` at
    /// every point.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub axes: Vec<[f32; 3]>,
    /// For a setup on a rotary, a point on its axis and the axis's
    /// direction, in the model's coordinates: the tool stands out from it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub about: Option<[[f32; 3]; 2]>,
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
    /// the machine profile's rates ([`Machining`]).
    #[serde(default)]
    pub seconds: f64,
    /// How it cuts, for the setup sheet: `None` for one with no tool.
    #[serde(default)]
    pub cutting: Option<Cutting>,
    /// Where its checks found something, in order.
    #[serde(default)]
    pub marks: Vec<Mark>,
    /// What it machines, to draw while it is edited, as camcraft's app
    /// draws it: none for an operation that keeps to no boundary.
    #[serde(default)]
    pub outlines: Option<Outlines>,
}

/// An operation's boundary as drawn over the stock, in the model's
/// coordinates: each ring closed, its last point back to its first, but for
/// the open curves an engraving follows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Outlines {
    /// The boundary as chosen: a 3D operation's machining boundary, a 2D
    /// operation's outline, the curves an engraving follows, the areas a
    /// V-carving carves.
    pub boundary: Vec<Vec<[f32; 3]>>,
    /// Where the tool's centre may go against it (3D).
    pub limit: Vec<Vec<[f32; 3]>>,
    /// What the tool cuts outside the part, or beside the outline, each
    /// piece an outer ring with its holes: drawn hatched.
    pub cut: Vec<CutPiece>,
}

/// A piece of what a tool cuts: an outer ring and the holes inside it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CutPiece {
    pub outline: Vec<[f32; 3]>,
    pub holes: Vec<Vec<[f32; 3]>>,
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
    /// For each line, how many of the operation's moves are made once it
    /// has run, as [`Moment::moves`] counts them: never fewer than the line
    /// before's. Empty when it is not known.
    #[serde(default)]
    pub moves: Vec<u32>,
    /// For a setup's program, each line's operation, by its index among the
    /// run's, whose moves [`Program::moves`] counts: never one before the
    /// line before's. Empty for an operation's own program.
    #[serde(default)]
    pub operations: Vec<u32>,
}

impl Program {
    /// The line the tool stands on once `made` moves are made: the first
    /// that makes that many, from zero, the first line before any. `None`
    /// when the lines' moves are not known.
    pub fn line_of(&self, made: usize) -> Option<usize> {
        let last = self.moves.len().checked_sub(1)?;
        Some(
            self.moves
                .partition_point(|m| (*m as usize) < made)
                .min(last),
        )
    }

    /// How many moves are made once line `line` has run.
    pub fn made_by(&self, line: usize) -> Option<usize> {
        self.moves.get(line).map(|m| *m as usize)
    }

    /// Whether it is a setup's program, of every operation.
    pub fn is_setups(&self) -> bool {
        !self.operations.is_empty()
    }

    /// The line the tool stands on once `made` moves of operation `op` are
    /// made, in a setup's program; in an operation's own, `op` is its own.
    /// `None` when the lines' moves are not known.
    pub fn line_at(&self, op: usize, made: usize) -> Option<usize> {
        if !self.is_setups() {
            return self.line_of(made);
        }
        let last = self.moves.len().min(self.operations.len()).checked_sub(1)?;
        // The first line at or past it, by halves: a program's lines are in
        // order of operation and move.
        let at = Some((op, made));
        let (mut low, mut high) = (0, last + 1);
        while low < high {
            let mid = low + (high - low) / 2;
            if self.moment(mid) < at {
                low = mid + 1;
            } else {
                high = mid;
            }
        }
        Some(low.min(last))
    }

    /// The operation, in a setup's program, and how many of its moves are
    /// made once line `line` has run; `None` past the end, or for an
    /// operation's own program.
    pub fn moment(&self, line: usize) -> Option<(usize, usize)> {
        Some((
            *self.operations.get(line)? as usize,
            *self.moves.get(line)? as usize,
        ))
    }
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

/// How a playback session's grid goes round a rotary's axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct GridRound {
    /// Out from the axis a quarter turn on from `up`.
    pub side: [f32; 3],
    /// The angle at the grid's first point, and on for each column and
    /// row, in radians.
    pub first: f32,
    pub per_col: f32,
    pub per_row: f32,
}

impl GridRound {
    /// One unit out from the axis at point `(c, r)`, `up` being the way out
    /// at angle zero.
    pub fn out(&self, up: [f32; 3], c: f32, r: f32) -> [f32; 3] {
        let (sin, cos) = (self.first + c * self.per_col + r * self.per_row).sin_cos();
        std::array::from_fn(|i| up[i] * cos + self.side[i] * sin)
    }
}

/// A playback session's material grid and timing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionReady {
    /// Points across and along.
    pub cols: u32,
    pub rows: u32,
    /// Point `(c, r)` at height `h` sits at `origin + c * across + r *
    /// along + h * up`, in the model's coordinates; on a rotary, with
    /// `round`, `up` turned round it.
    pub origin: [f32; 3],
    pub across: [f32; 3],
    pub along: [f32; 3],
    pub up: [f32; 3],
    /// For a setup on a rotary: `up` turned about its axis, towards
    /// `side`, by `first + c * per_col + r * per_row` radians. The grid
    /// then stands on the axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round: Option<GridRound>,
    /// A height is `low + step * h` for a packed `h`.
    pub low: f32,
    pub step: f32,
    /// When each move of each operation ends, in machining seconds from
    /// the setup's start.
    pub times: Vec<Vec<f32>>,
    /// Its run's [`timing`], as [`Run::timing`] has it.
    #[serde(default)]
    pub timing: u64,
    /// The grid's squares (from point `(c, r)` to `(c + 1, r + 1)`, row by
    /// row) with no stock over them at the start, as round stock leaves
    /// its box's corners: a bit each, the lowest first, in base64. Empty
    /// when there are none.
    #[serde(default)]
    pub bare: String,
}

/// A digest of when each move of each operation ends, in machining seconds
/// from the operation's start: two runs time their moves alike only if they
/// have the same moves at the same feeds. FNV-1a over each operation's move
/// count and each time's bits, so it is the same on every system.
pub fn timing<'a>(operations: impl IntoIterator<Item = &'a [f64]>) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |word: u64| {
        for b in word.to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for times in operations {
        eat(times.len() as u64);
        for t in times {
            eat(t.to_bits());
        }
    }
    h
}

/// Whether square `i` is one of `bare`'s, as [`SessionReady::bare`] packs
/// them.
pub fn is_bare(bare: &[u8], i: usize) -> bool {
    bare.get(i / 8).is_some_and(|b| b & (1 << (i % 8)) != 0)
}

/// Squares, each bare or not, packed as [`SessionReady::bare`] has them.
pub fn pack_bare(squares: impl IntoIterator<Item = bool>) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, bare) in squares.into_iter().enumerate() {
        if i % 8 == 0 {
            out.push(0);
        }
        if bare && let Some(last) = out.last_mut() {
            *last |= 1 << (i % 8);
        }
    }
    out
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
    fn a_line_and_its_move_find_each_other() {
        // A header, three moves (the second in two lines), a footer.
        let program = Program {
            moves: vec![0, 0, 1, 2, 2, 3, 3],
            ..Program::default()
        };
        assert_eq!(program.line_of(0), Some(0));
        assert_eq!(program.line_of(1), Some(2));
        assert_eq!(program.line_of(2), Some(3), "the first of its lines");
        assert_eq!(program.line_of(3), Some(5));
        assert_eq!(program.line_of(9), Some(6), "past the end, the last");
        assert_eq!(program.made_by(4), Some(2));
        assert_eq!(program.made_by(7), None);
        assert_eq!(Program::default().line_of(0), None, "not known");
    }

    #[test]
    fn a_setups_line_and_its_operations_move_find_each_other() {
        // Operation 0's header and two moves, operation 2's tool change and
        // three moves (operation 1 made none), and the footer.
        let program = Program {
            operations: vec![0, 0, 0, 2, 2, 2, 2, 2],
            moves: vec![0, 1, 2, 0, 1, 2, 3, 3],
            ..Program::default()
        };
        assert!(program.is_setups());
        assert_eq!(program.line_at(0, 0), Some(0));
        assert_eq!(program.line_at(0, 2), Some(2));
        assert_eq!(program.line_at(1, 0), Some(3), "none of its own: the next");
        assert_eq!(program.line_at(2, 0), Some(3));
        assert_eq!(program.line_at(2, 3), Some(6), "the first of its lines");
        assert_eq!(program.line_at(9, 0), Some(7), "past the end, the last");
        assert_eq!(program.moment(4), Some((2, 1)));
        assert_eq!(program.moment(8), None);
        let own = Program {
            moves: vec![0, 1, 1],
            ..Program::default()
        };
        assert!(!own.is_setups());
        assert_eq!(own.line_at(5, 1), Some(1), "its own operation's");
        assert_eq!(own.moment(1), None);
    }

    #[test]
    fn bare_squares_read_back() {
        let squares = [
            true, false, false, true, false, false, false, false, true, true,
        ];
        let packed = pack_bare(squares);
        assert_eq!(packed.len(), 2);
        for (i, bare) in squares.iter().enumerate() {
            assert_eq!(is_bare(&packed, i), *bare, "{i}");
        }
        assert!(!is_bare(&packed, 40), "past the end, not bare");
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
