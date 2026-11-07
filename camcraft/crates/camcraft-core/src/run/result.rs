//! What a run produced: each operation's paths and reports, and the setup's
//! material, verified.

use std::sync::Arc;

use camcraft_sim::stock::{Block, PackedStock, PartSurface, SimReport, StockError, VerifyReport};
use glam::DVec2;

use crate::job::{Coolant, HeightContext, Piece, SpindleDirection};
use crate::path::linker::LinkReport;
use crate::path::toolpath::Toolpath;
use crate::setup::Carried;
use crate::setup::frame::Frame;
use crate::strategy::adaptive::AdaptiveReport;
use crate::strategy::drilling::DrillReport;
use crate::strategy::engrave::EngraveReport;
use crate::strategy::flat::FlatReport;
use crate::strategy::holes::HoleReport;
use crate::strategy::morph::MorphedSpiralReport;
use crate::strategy::parallel::ParallelReport;
use crate::strategy::planar::PlanarReport;
use crate::strategy::pocket_clearing::ClearingReport;
use crate::strategy::projected::ProjectReport;
use crate::strategy::scallop::ScallopReport;
use crate::strategy::vcarve::VCarveReport;
use crate::strategy::waterline::WaterlineReport;

/// What a strategy reported, in its own terms.
///
/// Carried through rather than flattened into warnings, so that anything looking
/// at a result -- the app, the command line and the tests alike -- sees the same
/// numbers the run did.
#[derive(Debug, Clone, PartialEq)]
pub enum StrategyDetail {
    /// Raster finishing.
    Parallel(ParallelReport),
    /// Scallop finishing.
    Scallop(ScallopReport),
    /// Contour finishing: a waterline.
    Contour(WaterlineReport),
    /// Flat finishing.
    Flat(FlatReport),
    /// Spiral finishing.
    Spiral(ProjectReport),
    /// Radial finishing.
    Radial(ProjectReport),
    /// Morphed spiral finishing.
    MorphedSpiral(MorphedSpiralReport),
    /// Z-level clearing.
    PocketClearing(ClearingReport),
    /// Adaptive clearing.
    Adaptive(AdaptiveReport),
    /// A 2D profile.
    Profile(PlanarReport),
    /// A 2D pocket.
    Pocket(PlanarReport),
    /// Facing.
    Face(PlanarReport),
    /// Drilling.
    Drill(DrillReport),
    /// Boring, circular milling or threading.
    Holes(HoleReport),
    /// Engraving along curves.
    Engrave(EngraveReport),
    /// V-carving down the middle of areas.
    VCarve(VCarveReport),
    /// Custom G-code: only lines for the program.
    CustomGcode,
}

/// How an operation's paths came to be in a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Made {
    /// Worked out in this run.
    Generated,
    /// Kept from an earlier run, as it was, and cut again into this run's
    /// material.
    Kept,
    /// Not worked out: no paths, and it cuts nothing.
    Skipped,
    /// Turned off in the job: no paths, cuts nothing, and is not posted.
    Disabled,
    /// Custom G-code: no paths, cuts nothing, and its lines are written into
    /// the program beside it.
    Text,
}

impl Made {
    /// Whether it has paths: worked out now or kept from before.
    pub fn has_paths(self) -> bool {
        matches!(self, Made::Generated | Made::Kept)
    }
}

/// What a run is to do with one operation.
#[derive(Debug, Clone)]
pub enum Plan {
    /// Work it out.
    Generate,
    /// Keep what an earlier run made of it, unchanged since.
    Keep(Box<OperationResult>),
    /// Leave it out.
    Skip,
}

/// One piece of the tool above its flutes meeting material.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clash {
    /// Which piece.
    pub piece: Piece,
    /// How wide it is, at its widest, in millimetres.
    pub diameter: f64,
    /// The first move it meets material on, as an index into the path.
    pub first_move: usize,
    /// How many moves it meets material on.
    pub moves: u64,
    /// The most material stands into it, in millimetres.
    pub deepest: f64,
    /// Whether the part itself is in its way somewhere, not only material
    /// left above it.
    pub part: bool,
    /// Whether a rapid is among the moves.
    pub rapid: bool,
}

impl Clash {
    /// What it is, in a sentence: what met what, on how many moves from
    /// which, how far into it, and for the holder the stickout `needs` that
    /// would clear it.
    pub fn line(&self, needs: Option<f64>) -> String {
        let what = match self.piece {
            Piece::Body => "the cutter above its flutes".to_string(),
            Piece::Shank => format!("the shank (Ø{:.2})", self.diameter),
            Piece::Holder(i) => format!("holder segment {} (Ø{:.2})", i + 1, self.diameter),
        };
        let hit = if self.part { "the part" } else { "the stock" };
        let rapid = if self.rapid {
            ", a rapid among them"
        } else {
            ""
        };
        let mut out = format!(
            "{what} meets {hit} on {} move(s) from move {}{rapid}, {:.2} mm into it",
            self.moves,
            self.first_move + 1,
            self.deepest
        );
        if let (Piece::Holder(_), Some(n)) = (self.piece, needs) {
            // Rounded up, so the stickout said clears it.
            out.push_str(&format!(
                ": needs {:.1} mm stickout",
                (n * 10.0).ceil() / 10.0
            ));
        }
        out
    }
}

/// What the simulator found of the tool above its flutes: its body past the
/// flute length, its shank and its holder, against the material as each move
/// found it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Clashes {
    /// Each piece that met material, from the tip upward.
    pub found: Vec<Clash>,
    /// The stickout the holder clears everything at, when it met material:
    /// the tool's own and the most material stood into the holder.
    pub needs_stickout: Option<f64>,
    /// Whether the tool had anything above its flutes to check.
    pub checked: bool,
    /// What its lengths did not say enough to check.
    pub unchecked: Vec<String>,
    /// Cutting moves a 3D finish left out where they met the part.
    pub trimmed: u64,
}

/// What one operation produced.
#[derive(Debug, Clone)]
pub struct OperationResult {
    /// The operation's name.
    pub name: String,
    /// Which strategy produced it, for the program's section comment.
    pub strategy: &'static str,
    /// The machinable path.
    pub path: Toolpath,
    /// Tool number, for the post.
    pub tool_number: u32,
    /// How far the tool's tip is from the spindle's gauge line, in
    /// millimetres, where its lengths say ([`ToolSpec::gauge_length`]): for
    /// its travel in machine coordinates.
    ///
    /// [`ToolSpec::gauge_length`]: crate::job::ToolSpec::gauge_length
    pub tool_length: Option<f64>,
    /// Spindle speed.
    pub rpm: u32,
    /// Tool description.
    pub tool_comment: String,
    /// Which way the spindle turns.
    pub spindle: SpindleDirection,
    /// Coolant while cutting.
    pub coolant: Coolant,
    /// What the simulator saw while this operation ran.
    pub sim: SimReport,
    /// What it saw of the tool above its flutes.
    pub clashes: Clashes,
    /// Cutting passes produced.
    pub passes: u64,
    /// Inter-pass travel removed by reordering, as a fraction.
    pub travel_saved: f64,
    /// Where its corner reliefs take the tool's centre, seen from above: from
    /// each corner to the relief's end, in every copy of its pattern. What
    /// they cut past the part is no gouge.
    pub reliefs: Vec<[DVec2; 2]>,
    /// What the strategy reported.
    pub detail: StrategyDetail,
    /// What linking did.
    pub links: LinkReport,
    /// The clearance height, resolved: where the tool arrives and leaves. Between
    /// cuts it flies over the material by the safe distance (or at the retract
    /// height, with a full retraction), and never lower than this where the
    /// material is unknown.
    pub clearance_z: f64,
    /// Continuous cutting descents steeper than [`PLUNGE_RATIO`](super::PLUNGE_RATIO).
    pub steep_cut_descents: u64,
    /// Deepest of those, in millimetres.
    pub deepest_cut_descent_mm: f64,
    /// How deep a steep descent may be before it is a plunge, in millimetres: the
    /// operation's `max_cut_step`.
    pub max_cut_step: f64,
    /// Anything the caller should look at.
    pub warnings: Vec<String>,
    /// How its paths came to be in this run.
    pub made: Made,
}

impl OperationResult {
    /// Whether this operation is safe to run as it stands.
    ///
    /// A plunge at cutting feed is as unsafe as a rapid through material, so a
    /// steep descent deeper than the allowance fails the operation, not just warns.
    pub fn is_clean(&self) -> bool {
        self.sim.is_clean()
            && self.clashes.found.is_empty()
            && !self.path.has_unplanned_links()
            && self.deepest_cut_descent_mm <= self.max_cut_step
    }
}

/// What one setup produced. Everything in it is in the setup's frame.
#[derive(Debug, Clone)]
pub struct SetupResult {
    /// Which setup.
    pub setup: String,
    /// How model coordinates became the setup's.
    pub frame: Frame,
    /// Each operation, in order.
    pub operations: Vec<OperationResult>,
    /// The part the setup was supposed to produce, shared with whatever replays
    /// the run.
    pub part: Arc<PartSurface>,
    /// How the finished stock compares with the part.
    pub verify: VerifyReport,
    /// Tolerance the comparison allowed.
    pub verify_budget: f64,
    /// Each block of stock.
    pub stocks: Vec<Block>,
    /// Footprint of all the stock in XY, lower corner.
    pub stock_min: DVec2,
    /// Footprint of all the stock in XY, upper corner.
    pub stock_max: DVec2,
    /// The heights symbolic references resolved against.
    pub heights: HeightContext,
    /// The material before the first operation, then after each: one more than
    /// there are operations, the last being the finished material. The keyframes a replay
    /// starts from, packed (a full sheet's raw heights are hundreds of megabytes,
    /// and machined material packs small) and shared with it rather than copied.
    pub keyframes: Vec<Arc<PackedStock>>,
}

impl SetupResult {
    /// Whether the whole setup verified.
    pub fn is_clean(&self) -> bool {
        self.verify.is_clean() && self.operations.iter().all(OperationResult::is_clean)
    }

    /// The material it left, for a setup that carries on from it: the finished
    /// material, each column standing on its stock's underside.
    pub fn left(&self) -> Result<Carried, StockError> {
        let last = self
            .keyframes
            .last()
            .ok_or(StockError::NoBlocks)?
            .unpack()?;
        // A cell's material reaches to the cell's edges either way, as a
        // block's top is taken to them.
        let grow = last.field().pitch();
        let blocks = &self.stocks;
        let stock = last.with_bottoms(|p| {
            blocks
                .iter()
                .filter_map(|b| b.bottom_near(p, grow))
                .reduce(f64::min)
        });
        let within = crate::setup::bounds_all(blocks).ok_or(StockError::NoBlocks)?;
        let frame = self.frame;
        Ok(Carried {
            stock: Arc::new(stock),
            within,
            to_model: crate::setup::affine_of(|q| frame.to_model(q)),
        })
    }

    /// Everything worth a person's attention, across every operation.
    pub fn warnings(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .operations
            .iter()
            .flat_map(|o| o.warnings.iter().map(|w| format!("{}: {w}", o.name)))
            .collect();
        if self.verify.gouged > 0 {
            out.push(format!(
                "finished stock is below the part in {} column(s), deepest {:.3} mm \
                 against a {:.3} mm budget",
                self.verify.gouged, self.verify.deepest_gouge_mm, self.verify_budget
            ));
        }
        out
    }
}

/// Where a run has got to, as it goes: what it is doing now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Laying the models and stock out in the setup's frame.
    Preparing,
    /// Running operation `i`, from 0: its paths, then cutting them.
    Operation(usize),
    /// Measuring the finished material against the part.
    Verifying,
    /// Running the setup's checks on what it made.
    Checking,
}
