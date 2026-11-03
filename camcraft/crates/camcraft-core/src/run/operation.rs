//! Running one operation: its strategy's paths, ordered, linked over the material
//! as it stands, and cut into it.

use std::cell::OnceCell;

use camcraft_geom::contour::Region;
use camcraft_geom::topology::EdgeKind;
use camcraft_sim::stock::{Block, SimReport, Stock};
use glam::{DVec2, DVec3, dvec2};

use super::error::in_op;
use super::{Made, OperationResult, PLUNGE_RATIO, RunError, StrategyDetail};
use crate::cutter::allowance::{Allowance, EffectiveTool};
use crate::cutter::dropcutter::DropCutter;
use crate::cutter::tool::{Flat, ToolProfile};
use crate::job::{
    ChamferSide, ContourOrder, CutDirection, Depths, Entry, FaceDirection, HeightContext, Job,
    Links, Operation, PassOrder, Picks, RadialDirection, Retraction, SlotPattern, SpindleDirection,
    StrategySpec, ToolSpec,
};
use crate::path::linker::{
    ClearanceField, EntryKind, LinkParams, LinkReport, Material, Ramp, StayDown, Surface,
    highest_under, link,
};
use crate::path::ordering::order_passes;
use crate::path::pattern::{each, overhang, repeat};
use crate::path::simadapt::sim_moves_of;
use crate::path::tabs::{TabParams, add_tabs};
use crate::path::toolpath::{Move, Toolpath};
use crate::setup::features::{Chain, FeatureError, Picking, picked_chains};
use crate::setup::{Bodies, Prepared};
use crate::strategy::StrategyError;
use crate::strategy::adaptive::{AdaptiveParams, adaptive_rough, flat_heights};
use crate::strategy::chamfer::{Centres, Place, centres, material_side};
use crate::strategy::drilling::{DrillParams, DrillReport, Hole, Tap, drill, holes, nearest_first};
use crate::strategy::engrave::{EngraveParams, EngraveReport, Levels, Trace, engrave, in_order};
use crate::strategy::face::{FaceParams, LevelsAsked, face};
use crate::strategy::flat::{FlatParams, flat_finish};
use crate::strategy::morph::{MorphParams, MorphedSpiralReport, morphed_spiral};
use crate::strategy::parallel::{ParallelParams, ParallelReport, follow, parallel_finish_within};
use crate::strategy::patterns::{Centred, radial, reaching, spiral};
use crate::strategy::planar::{
    Area, LeadArc, PlanarParams, PlanarReport, Spiral, WallFinish, areas, centre_offset, pocket,
    profile,
};
use crate::strategy::pocket_clearing::{ClearingParams, pocket_clearing};
use crate::strategy::projected::{Curve, ProjectParams, ProjectReport, project};
use crate::strategy::reach::LevelReach;
use crate::strategy::scallop::{ScallopParams, contact_region, scallop_finish};
use crate::strategy::vcarve::{VCarveParams, vcarve};
use crate::strategy::waterline::{WaterlineParams, waterline};

mod carving;
mod clearing;
mod finishing;
mod link;

pub(super) use carving::chamfer_curves;
use carving::{chamfer_paths, engrave_paths, slot_paths, vcarve_paths};
use clearing::{adaptive_paths, drill_paths, face_paths, planar_paths, pocket_clearing_paths};
pub(super) use clearing::{drill_holes, heights_of, planar_areas};
use finishing::{
    contour_paths, flat_paths, morphed_spiral_paths, parallel_paths, radial_paths, scallop_paths,
    spiral_paths,
};
use link::{Cutting, OnPart, link_params};

/// One operation, and everything its strategy works from.
pub(super) struct Work<'a> {
    /// The job it is in.
    pub(super) job: &'a Job,
    /// The operation.
    pub(super) op: &'a Operation,
    /// Its tool, as the job describes it.
    pub(super) spec: &'a ToolSpec,
    /// Its tool as it cuts: grown by the stock to leave.
    pub(super) tool: &'a EffectiveTool,
    /// What its heights resolve against.
    pub(super) heights: &'a HeightContext,
    /// The setup, laid out: its part, and what is found from it once.
    pub(super) prepared: &'a Prepared,
    /// The drop-cutter over the part.
    pub(super) cutter: &'a DropCutter<'a>,
    /// The stock's footprint in XY, lower and upper corner.
    pub(super) bounds: (DVec2, DVec2),
    /// The blocks of material the setup starts from.
    pub(super) blocks: &'a [Block],
    /// The heights of the part's flat faces, found for the setup when an
    /// operation first needs them.
    pub(super) flats: &'a OnceCell<Vec<f64>>,
}

impl Work<'_> {
    /// Whether its tool's spindle turns clockwise, seen from above: a pass
    /// climbs with what it cuts on its right.
    fn clockwise(&self) -> bool {
        self.spec.spindle == SpindleDirection::Clockwise
    }

    /// Whether the material being cut is on the tool's right: climbing with a
    /// clockwise spindle, or conventional with the other.
    fn kept_on_right(&self) -> bool {
        self.clockwise() != self.op.strategy.conventional()
    }

    /// The heights of the part's flat faces, raised by the stock to leave down,
    /// where flat area detection adds a level; none unless `on`.
    fn flat_levels(&self, on: bool) -> Vec<f64> {
        if !on {
            return Vec::new();
        }
        let axial = self.op.stock_to_leave().axial;
        self.flats
            .get_or_init(|| flat_heights(&self.prepared.part, self.job.tolerance))
            .iter()
            .map(|z| z + axial)
            .collect()
    }

    /// The range of slopes its strategy keeps to, in radians from level: every
    /// slope where it has none.
    fn slopes(&self) -> (f64, f64) {
        let (from, to) = self.op.strategy.slopes().unwrap_or((0.0, 90.0));
        (from.to_radians(), to.to_radians())
    }

    /// Which way a spiral cut in `order` goes: outward or not, and turning
    /// anticlockwise or not. Going outward, what is still to finish is outside:
    /// on the tool's right going anticlockwise.
    fn turning(&self, order: PassOrder) -> (bool, bool) {
        let outward = order == PassOrder::InsideOut;
        (outward, outward == self.kept_on_right())
    }

    /// Where the tool's centre may sweep: the stock's footprint, plus the tool,
    /// plus whatever extra was asked for.
    fn sweep(&self) -> (DVec2, DVec2) {
        let (lo, hi) = self.bounds;
        let reach = self.tool.max_radius() + self.op.boundary.margin;
        (
            dvec2(lo.x - reach, lo.y - reach),
            dvec2(hi.x + reach, hi.y + reach),
        )
    }
}

/// A strategy's paths, before ordering and linking: the path, how many passes it
/// has, and what the strategy reported.
type Paths = (Toolpath, u64, StrategyDetail);

/// What an operation is cut to, found before its strategy runs.
pub(super) enum Target {
    /// The machining boundary its tool's centre is kept to, if any.
    Limit(Option<Region>),
    /// A drilling operation's holes, in order.
    Holes(Vec<Hole>),
    /// A 2D operation's areas, each with its floor.
    Areas(Vec<Area>),
    /// The curves an engraving follows.
    Traces(Vec<Trace>),
    /// The area a V-carving carves.
    Carve(Region),
}

/// Work out one operation's paths, link them over the material as it stands, and
/// cut them into it, to `target`.
pub(super) fn run_operation(
    work: &Work<'_>,
    target: &Target,
    stock: &mut Stock,
) -> Result<OperationResult, RunError> {
    let op = work.op;
    let ctx = work.heights;
    let mut warnings = Vec::new();

    let (mut limit, mut holes, mut found, mut traces, mut carved) =
        (None, &[][..], &[][..], &[][..], None);
    match target {
        Target::Limit(l) => limit = l.as_ref(),
        Target::Holes(h) => holes = h.as_slice(),
        Target::Areas(a) => found = a.as_slice(),
        Target::Traces(t) => traces = t.as_slice(),
        Target::Carve(area) => carved = Some(area),
    }
    let (raw, passes, detail) = match op.strategy {
        StrategySpec::Parallel { angle } => parallel_paths(work, angle, limit, &mut warnings)?,
        StrategySpec::Scallop {
            direction, order, ..
        } => scallop_paths(work, (direction, order), limit, &mut warnings)?,
        StrategySpec::Contour {
            direction, order, ..
        } => contour_paths(work, (direction, order), limit, &mut warnings)?,
        StrategySpec::Flat { order, .. } => flat_paths(work, order, limit, &mut warnings)?,
        StrategySpec::Spiral {
            centre,
            inner_radius,
            outer_radius,
            order,
            ..
        } => spiral_paths(
            work,
            (centre, inner_radius, outer_radius),
            order,
            limit,
            &mut warnings,
        )?,
        StrategySpec::Radial {
            centre,
            inner_radius,
            outer_radius,
            angle_from,
            angle_to,
            direction,
            ..
        } => radial_paths(
            work,
            (centre, inner_radius, outer_radius),
            (angle_from, angle_to, direction),
            limit,
            &mut warnings,
        )?,
        StrategySpec::MorphedSpiral { centre, order, .. } => {
            morphed_spiral_paths(work, centre, order, limit, &mut warnings)?
        }
        StrategySpec::PocketClearing {
            min_ring_length,
            flat_areas,
            ..
        } => pocket_clearing_paths(
            work,
            min_ring_length,
            flat_areas,
            limit,
            stock,
            &mut warnings,
        )?,
        StrategySpec::Adaptive {
            direction,
            flat_areas,
        } => adaptive_paths(work, direction, flat_areas, limit, &mut warnings)?,
        StrategySpec::Profile { .. } | StrategySpec::Pocket { .. } => {
            planar_paths(work, found, &mut warnings)?
        }
        StrategySpec::Face {
            angle,
            direction,
            pass_extension,
            stock_offset,
        } => face_paths(
            work,
            (angle, direction),
            (pass_extension, stock_offset),
            &mut warnings,
        )?,
        StrategySpec::Drill {
            peck,
            dwell,
            chip_break,
            ..
        } => drill_paths(
            work,
            (peck, chip_break, dwell),
            op.strategy.tap(),
            holes,
            &mut warnings,
        )?,
        StrategySpec::Engrave {
            depth,
            direction,
            reverse,
        } => engrave_paths(
            work,
            (depth, direction, reverse),
            Levels::Ramped,
            traces,
            &mut warnings,
        )?,
        StrategySpec::Slot {
            depth,
            pattern,
            reverse,
        } => slot_paths(work, (depth, pattern, reverse), traces, &mut warnings)?,
        StrategySpec::Chamfer { .. } => chamfer_paths(work, traces, &mut warnings)?,
        StrategySpec::VCarve { max_depth } => match carved {
            Some(area) => vcarve_paths(work, max_depth, area, &mut warnings)?,
            None => return Err(in_op(op)(FeatureError::NothingToCarve)),
        },
    };

    // A pattern repeats the strategy's path before anything else is done to
    // it, so each copy is ordered, tabbed, linked into the material and
    // simulated as the original is.
    let (raw, passes) = if op.transform.repeats() {
        let (path, copies) = patterned(work, &raw, &mut warnings)?;
        (path, passes.saturating_mul(u64::from(copies)))
    } else {
        (raw, passes)
    };

    // Ordering is a Toolpath -> Toolpath transform, applied before linking so the
    // retracts it needs are planned around the order actually used. Holes are
    // already in order, and a hole reversed would be drilled from the bottom up.
    // A 2D operation's order is its own: holes before the outline that frees a
    // part, a pocket's middle before its walls.
    let (ordered, order) = if op.linking.reorder && op.strategy.uses().reorder {
        let by_level = op.strategy.uses().depths != Depths::None;
        order_passes(&raw, op.linking.allow_reverse, by_level)?
    } else {
        (raw, Default::default())
    };

    // Tabs are another Toolpath -> Toolpath transform, on the order used, before
    // linking: the linker cuts the passes into the material as it links them,
    // so it plans round the tabs left standing. Only a strategy that cuts loops
    // has them.
    let ordered = match op.passes.tabs {
        Some(tabs) if op.strategy.uses().tabs => {
            let params = TabParams {
                tabs,
                tool_radius: work.tool.cutter_radius(),
                plunge_feed: op.linking.plunge_feed,
            };
            let (tabbed, report) =
                add_tabs(&ordered, &params).map_err(|e| in_op(op)(StrategyError::from(e)))?;
            if report.tabs == 0 {
                warnings
                    .push("no tabs: no closed loop at the deepest level has room for one".into());
            } else if report.too_short > 0 {
                warnings.push(format!(
                    "{} loop(s) at the deepest level too short for a tab",
                    report.too_short
                ));
            }
            tabbed
        }
        _ => ordered,
    };

    // Traverses clear the material as it stood before the operation: never lower
    // than it will stand, so they are safe whatever the operation cuts first.
    // Between cuts, the safe distance over it, as Fusion's minimum retraction;
    // with a full one, the retract height or higher. The clearance height is
    // where the tool arrives and leaves.
    let clearance_z = op.heights.clearance.resolve(ctx);
    let retract_z = op.heights.retract.resolve(ctx);
    let clearance =
        ClearanceField::new(stock.field().clone(), op.linking.safe_distance, clearance_z)
            .map_err(in_op(op))?;
    let clearance = match op.linking.retraction {
        Retraction::Minimum => clearance,
        Retraction::Full => clearance.at_least(retract_z),
    };

    // Linked over the material as each pass leaves it, and cut into it as linked.
    let before = stock.volume();
    let surface = OnPart::new(work.cutter, work.tool, work.job.tolerance);
    let mut cutting = Cutting {
        stock,
        tool: work.tool,
        rapid_contacts: 0,
        deepest: 0.0,
    };
    let (path, mut links) = link(
        &ordered,
        &clearance,
        &mut cutting,
        &surface,
        &link_params(work, clearance_z),
    )
    .map_err(in_op(op))?;
    // Adaptive clearing makes its own links at the level, and its own helixes:
    // they are the operation's as much as the linker's are.
    if let StrategyDetail::Adaptive(r) = &detail {
        links.links_resolved += r.stayed_down;
        links.stayed_down += r.stayed_down;
        links.helixes += r.helixes;
    }
    let sim = SimReport {
        removed_mm3: (before - cutting.stock.volume()).max(0.0),
        rapid_contacts: cutting.rapid_contacts,
        deepest_rapid_contact_mm: cutting.deepest,
    };
    if links.suspicious_plunges > 0 {
        warnings.push(format!(
            "{} plunge(s) do not start above the surface",
            links.suspicious_plunges
        ));
    }
    if path.has_unplanned_links() {
        warnings.push("path still contains unresolved links and is NOT machinable".into());
    }
    if sim.rapid_contacts > 0 {
        warnings.push(format!(
            "{} rapid move(s) pass through material, deepest {:.2} mm",
            sim.rapid_contacts, sim.deepest_rapid_contact_mm
        ));
    }
    let start = path.moves().first().map_or(glam::DVec3::ZERO, |m| m.to);
    let (steep, deepest) = path.steep_cut_descents(start, PLUNGE_RATIO);
    if deepest > op.passes.max_cut_step {
        warnings.push(format!(
            "a steep cutting descent of {deepest:.2} mm exceeds the {:.2} mm \
             allowance: a plunge at cutting feed ({steep} steep descent(s))",
            op.passes.max_cut_step
        ));
    }

    Ok(OperationResult {
        path,
        sim,
        passes,
        travel_saved: order.saved(),
        detail,
        links,
        clearance_z,
        steep_cut_descents: steep,
        deepest_cut_descent_mm: deepest,
        warnings,
        // The rest is the operation's own: its name, strategy and tool.
        ..not_made(op, Some(work.spec), Made::Generated)
    })
}

/// `raw` repeated as `work`'s operation's pattern says, and how many copies
/// that is, the original among them; a copy that goes further outside the stock than the
/// original, or passes of mirrored copies cut the other way where that
/// matters, said.
fn patterned(
    work: &Work<'_>,
    raw: &Toolpath,
    warnings: &mut Vec<String>,
) -> Result<(Toolpath, u32), RunError> {
    let op = work.op;
    let pattern = &op.transform;
    let (lo, hi) = work.bounds;
    let tolerance = work.job.tolerance;
    let own = overhang(raw.moves(), lo, hi);
    let copies = each(raw, pattern);
    let outside: Vec<f64> = copies
        .iter()
        .map(|c| overhang(c, lo, hi))
        .filter(|o| *o > own + tolerance)
        .collect();
    if !outside.is_empty() {
        let worst = outside.iter().copied().fold(0.0, f64::max);
        warnings.push(format!(
            "{} of the pattern's {} copies go outside the stock, up to {worst:.1} mm \
             further than the original does",
            outside.len(),
            copies.len()
        ));
    }
    // Adaptive clearing steers each pass by the material ahead of it, and a
    // drill's holes have no way round.
    let steered = op.strategy.uses().links == Links::Own;
    let r = repeat(raw, pattern, steered).map_err(|e| in_op(op)(StrategyError::from(e)))?;
    // Where direction means nothing, or both ways is the strategy's own.
    let either_way = matches!(
        op.strategy,
        StrategySpec::Parallel { .. }
            | StrategySpec::Radial { .. }
            | StrategySpec::Face { .. }
            | StrategySpec::Drill { .. }
            | StrategySpec::Slot { .. }
            | StrategySpec::VCarve { .. }
    );
    if r.unturned > 0 && !either_way {
        warnings.push(format!(
            "{} pass(es) of the mirrored copies could not be turned round, so they cut the \
             other way: conventional where the original climbs",
            r.unturned
        ));
    }
    Ok((r.path, r.copies))
}

/// What stands for an operation left out of a run: no paths, nothing cut. `spec`
/// is its tool, if it names one the job has.
///
/// Its name, strategy and tool are what any result of the operation carries, so
/// a result worked out takes them from here too.
pub(super) fn not_made(op: &Operation, spec: Option<&ToolSpec>, made: Made) -> OperationResult {
    let detail = match op.strategy {
        StrategySpec::Parallel { .. } => StrategyDetail::Parallel(Default::default()),
        StrategySpec::Scallop { .. } => StrategyDetail::Scallop(Default::default()),
        StrategySpec::Contour { .. } => StrategyDetail::Contour(Default::default()),
        StrategySpec::Flat { .. } => StrategyDetail::Flat(Default::default()),
        StrategySpec::Spiral { .. } => StrategyDetail::Spiral(Default::default()),
        StrategySpec::Radial { .. } => StrategyDetail::Radial(Default::default()),
        StrategySpec::MorphedSpiral { .. } => StrategyDetail::MorphedSpiral(Default::default()),
        StrategySpec::PocketClearing { .. } => StrategyDetail::PocketClearing(Default::default()),
        StrategySpec::Adaptive { .. } => StrategyDetail::Adaptive(Default::default()),
        StrategySpec::Profile { .. } => StrategyDetail::Profile(Default::default()),
        StrategySpec::Pocket { .. } => StrategyDetail::Pocket(Default::default()),
        StrategySpec::Face { .. } => StrategyDetail::Face(Default::default()),
        StrategySpec::Engrave { .. } | StrategySpec::Slot { .. } | StrategySpec::Chamfer { .. } => {
            StrategyDetail::Engrave(Default::default())
        }
        StrategySpec::VCarve { .. } => StrategyDetail::VCarve(Default::default()),
        StrategySpec::Drill {
            peck,
            dwell,
            chip_break,
            ..
        } => StrategyDetail::Drill(DrillReport {
            holes: Vec::new(),
            peck,
            chip_break,
            dwell,
            tap: op.strategy.tap(),
            feed: op.linking.plunge_feed,
        }),
    };
    OperationResult {
        name: op.name.clone(),
        strategy: op.strategy.name(),
        path: Toolpath::new(),
        tool_number: spec.map_or(0, |t| t.number),
        rpm: op.passes.rpm.or(spec.map(|t| t.rpm)).unwrap_or(0),
        tool_comment: spec.map(|t| t.comment.clone()).unwrap_or_default(),
        // A tap turns the way its thread does, whichever way its tool says.
        spindle: match op.strategy.tap() {
            Some(t) => t.hand.spindle(),
            None => spec.map(|t| t.spindle).unwrap_or_default(),
        },
        coolant: spec.map(|t| t.coolant).unwrap_or_default(),
        sim: SimReport::default(),
        passes: 0,
        travel_saved: 0.0,
        detail,
        links: LinkReport::default(),
        clearance_z: 0.0,
        steep_cut_descents: 0,
        deepest_cut_descent_mm: 0.0,
        max_cut_step: op.passes.max_cut_step,
        warnings: Vec::new(),
        made,
    }
}
