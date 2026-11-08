//! How an operation is linked: its linking settings as the linker takes them,
//! and the material and part the linker measures its links against.

use super::holder::Watching;
use super::*;

/// How `work`'s operation links: its own settings, with the strategy's defaults
/// where it gives none, as Fusion's.
pub(super) fn link_params(work: &Work<'_>, clearance_z: f64) -> LinkParams {
    let (op, linking) = (work.op, &work.op.linking);
    let diameter = work.spec.diameter;
    // A profile's vertical lead is part of its leads; a finishing pass's is its
    // own setting.
    let leads = op.leads(diameter);
    let uses = op.strategy.uses();
    let vertical = if uses.lead_radius {
        op.lead_radius(diameter)
    } else {
        leads.map_or(0.0, |l| l.vertical)
    };
    // A slot one way lifts at the end of each level and goes back to its
    // start: staying down, it would drag back along what it just cut.
    let one_way = matches!(
        op.strategy,
        StrategySpec::Slot {
            pattern: SlotPattern::Directional,
            ..
        }
    );
    // Rotary finishing follows the part round the bar, which the linker
    // would measure a link against as a flat tool over the part unrolled,
    // not as the tool on a radius meets it: a link staying down, a lead or a
    // helix in could cut the part. So each of its links lifts clear and
    // comes straight down onto the next pass.
    let rotary =
        work.prepared.frame.wrap.is_some() && matches!(op.strategy, StrategySpec::Parallel { .. });
    let (stay_down, lead_radius, ramp) = match uses.links {
        _ if rotary => (StayDown::Never, 0.0, None),
        // Adaptive clearing stays down itself, at the level, only where it has
        // cleared the way, and goes down its own helixes: a link it leaves is
        // one to lift for, into what it has cleared, straight down.
        Links::Own => (StayDown::Never, 0.0, None),
        Links::OverPart => (
            StayDown::OverPart {
                max: op.stay_down(diameter),
            },
            vertical,
            None,
        ),
        Links::Level => (
            StayDown::Level {
                max: op.stay_down(diameter),
            },
            vertical,
            // Contour's ramp is its own, down the wall between levels: its
            // leads come down beside the wall, where the linker's test for
            // material, under the tool's whole width, would see the wall's
            // stock and go down a helix for nothing.
            (uses.ramp && !matches!(op.strategy, StrategySpec::Contour { .. })).then(|| Ramp {
                angle: linking.ramp_angle.to_radians(),
                // A ramp moves mostly sideways, so it goes nearer the cutting
                // feed than a straight plunge's.
                feed: op.ramp_feed(),
            }),
        ),
    };
    // A profile's ramp is its own path, a spiral down each ring
    // (`planar_paths`): the linker only brings the tool onto its lead.
    let profile = matches!(op.strategy, StrategySpec::Profile { .. });
    let entry = match op.entry() {
        _ if rotary => EntryKind::Plunge,
        Some(Entry::Helix) => EntryKind::Helix,
        Some(Entry::Plunge) => EntryKind::Plunge,
        Some(Entry::Ramp) if profile => EntryKind::Plunge,
        Some(Entry::Ramp) | None => EntryKind::Ramp,
    };
    let corner = work.spec.corner_radius();
    LinkParams {
        retract_feed: linking.retract_feed,
        plunge_feed: linking.plunge_feed,
        tool_radius: work.tool.cutter_radius(),
        safe_distance: linking.safe_distance,
        stay_down: if one_way { StayDown::Never } else { stay_down },
        lead_radius,
        ramp,
        entry,
        helix_radius: op.helix_diameter(diameter, corner) / 2.0,
        helix_min_radius: op.least_helix_diameter(diameter, corner) / 2.0,
        // A pocket with no room for a helix ramps along its ring, as Fusion's
        // helix ramp type allows; a profile goes straight down beside its lead.
        ramp_where_no_helix: !profile,
        // A helix climbs as the cut does.
        helix_ccw: work.kept_on_right(),
        ramp_stepdown: Some(op.ramp_stepdown(diameter)),
        lead_in_material: leads.is_some(),
        max_cut_step: op.passes.max_cut_step,
        tolerance: work.job.tolerance,
        end_z: Some(clearance_z),
        start_z: Some(clearance_z),
    }
}

/// The operation's stock, cut as its moves are linked: what every link decision
/// is measured against.
pub(super) struct Cutting<'a> {
    pub(super) stock: &'a mut Stock,
    pub(super) tool: &'a EffectiveTool,
    pub(super) rapid_contacts: u64,
    pub(super) deepest: f64,
    /// The tool above its flutes, watched as it cuts.
    pub(super) watching: Watching<'a>,
}

impl Material for Cutting<'_> {
    fn highest(&self, from: DVec2, to: DVec2, radius: f64) -> Option<f64> {
        // The simulator's own disc, down to its margin, so a rapid stopped above
        // this is one the simulator agrees touches nothing.
        let field = self.stock.field();
        let Some(round) = self.stock.round() else {
            return highest_under(field, from, to, radius * (1.0 + 1e-9)).top;
        };
        // On a rotary the grid is one turn round, and a path goes round and
        // round: the line is measured where it is on the grid, a turn either
        // side too, as it may cross where the grid starts again.
        let unit = match round.round {
            camcraft_sim::stock::Axis::X => DVec2::X,
            _ => DVec2::Y,
        };
        let period = round.period();
        let start = field.origin().dot(unit);
        let mid = 0.5 * (from + to).dot(unit);
        let turns = ((mid - start) / period).floor();
        let under = |reach: f64| {
            [-1.0, 0.0, 1.0]
                .into_iter()
                .filter_map(|k| {
                    let shift = unit * ((k - turns) * period);
                    highest_under(field, from + shift, to + shift, reach * (1.0 + 1e-9)).top
                })
                .reduce(f64::max)
        };
        let top = under(radius);
        // The tool spans more of the bar unrolled the nearer the axis it is:
        // `atan(r / ρ)` round, at the radius it is unrolled at. So the
        // material is measured again that wide, for the tool as near the axis
        // as it can be, on what was found (on the axis, where nothing was):
        // never less than it spans.
        let near = top.unwrap_or(0.0).max(1e-3);
        let wide = round.radius * (radius / near).atan();
        if wide > radius { under(wide) } else { top }
    }

    fn cut(&mut self, moves: &[Move]) {
        let swept = self
            .watching
            .cut(self.stock, &sim_moves_of(moves), self.tool);
        self.rapid_contacts += swept.rapid_contacts;
        self.deepest = self.deepest.max(swept.deepest_rapid_contact_mm);
    }
}

/// The part, as the operation's tool meets it.
pub(super) struct OnPart<'a> {
    cutter: &'a DropCutter<'a>,
    tool: &'a EffectiveTool,
    /// The tool a tolerance narrower: where one only grazing the part rests.
    slim: EffectiveTool,
    tolerance: f64,
}

impl<'a> OnPart<'a> {
    pub(super) fn new(cutter: &'a DropCutter<'a>, tool: &'a EffectiveTool, tolerance: f64) -> Self {
        let a = tool.allowance();
        let slim = Allowance::new(a.radial() - tolerance, a.axial())
            .ok()
            .and_then(|a| EffectiveTool::of(tool.base(), a).ok())
            .unwrap_or(*tool);
        Self {
            cutter,
            tool,
            slim,
            tolerance,
        }
    }
}

impl Surface for OnPart<'_> {
    fn at(&self, p: DVec2) -> Option<f64> {
        self.cutter.drop_at(self.tool, p.x, p.y)
    }

    fn along(&self, from: DVec2, to: DVec2) -> Vec<DVec3> {
        // Seeded at a quarter of the tool's width: nothing that narrow is passed over.
        let seed = (self.tool.max_radius() * 0.5).max(self.tolerance);
        follow(self.cutter, self.tool, from, to, self.tolerance, seed)
    }

    fn grazing_at(&self, p: DVec2) -> Option<f64> {
        self.cutter.drop_at(&self.slim, p.x, p.y)
    }

    fn grazing_along(&self, from: DVec2, to: DVec2) -> Vec<DVec3> {
        let seed = (self.slim.max_radius() * 0.5).max(self.tolerance);
        follow(self.cutter, &self.slim, from, to, self.tolerance, seed)
    }
}
