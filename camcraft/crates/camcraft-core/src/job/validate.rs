//! Checking a job before anything is computed.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    Depths, FixedBox, FixedCylinder, JOB_VERSION, Job, JobError, LAST_WCS, Operation, Origin,
    Retraction, Setup, StockSpec, StockToLeave, StrategySpec, ToolSpec,
};
use crate::is_positive;

/// What a length, a feed or a count must be.
const POSITIVE: &str = "finite and greater than zero";

/// What a size that may be nothing must be.
const AT_LEAST_ZERO: &str = "finite and at least zero";

/// What a coordinate, an angle or an offset either way must be.
const FINITE: &str = "finite";

impl Job {
    /// Check the whole job is runnable before anything is computed: everything
    /// [`check_common`](Self::check_common) checks, and every setup.
    ///
    /// Everything catchable is caught here, so a job fails on its own terms rather
    /// than part-way through an operation with a half-built path in hand.
    pub fn validate(&self) -> Result<(), JobError> {
        self.check_common()?;
        if self.setups.is_empty() {
            return Err(JobError::NoSetups);
        }
        (0..self.setups.len()).try_for_each(|i| self.check_setup(i))
    }

    /// Check what every setup depends on: the format, the tolerance, the tools,
    /// the names, and the stocks. A setup still being written does not fail this,
    /// so another setup can run.
    pub fn check_common(&self) -> Result<(), JobError> {
        if self.version != JOB_VERSION {
            return Err(JobError::UnsupportedVersion(self.version));
        }
        positive("tolerance", self.tolerance)?;

        for (name, spec) in &self.tools {
            // Its cutter builds, and turns, among the rest.
            if let Some(source) = spec.check().into_iter().next() {
                return Err(JobError::BadToolField {
                    name: name.clone(),
                    source,
                });
            }
        }

        unique("model", self.models.iter().map(|m| m.name.as_str()))?;
        for model in &self.models {
            let t = &model.transform;
            let field = |f: &str| format!("model '{}'.transform.{f}", model.name);
            let values = [
                ("position", &t.position),
                ("rotation", &t.rotation),
                ("scale", &t.scale),
            ];
            for (f, vs) in values {
                for &v in vs {
                    // A scale of nothing flattens the model to a plane, and one
                    // below it mirrors the part: a left hand cut for a right.
                    if f == "scale" && !(v.is_finite() && v >= 1e-6) {
                        return Err(bad(field(f), v, "finite and at least 0.000001"));
                    }
                    finite(&field(f), v)?;
                }
            }
        }
        unique("stock", self.stocks.iter().map(|s| s.name.as_str()))?;
        unique("setup", self.setups.iter().map(|s| s.name.as_str()))?;

        for stock in &self.stocks {
            let field = |f: &str| format!("stock '{}'.{f}", stock.name);
            for m in stock.spec.models() {
                if self.model(m).is_none() {
                    return Err(JobError::UnknownModel {
                        owner: format!("stock '{}'", stock.name),
                        model: m.clone(),
                    });
                }
            }
            match &stock.spec {
                StockSpec::AroundModels { margin, below, .. } => {
                    for (i, v) in margin.iter().chain(std::iter::once(below)).enumerate() {
                        at_least_zero(&field(if i < 3 { "margin" } else { "below" }), *v)?;
                    }
                }
                StockSpec::FixedBox(FixedBox { size, place, .. }) => {
                    for v in place.iter().filter_map(|p| p.value()) {
                        finite(&field("place"), v)?;
                    }
                    for v in size {
                        positive(&field("size"), *v)?;
                    }
                }
                StockSpec::CylinderAroundModels { margin, ends, .. } => {
                    at_least_zero(&field("margin"), *margin)?;
                    at_least_zero(&field("ends"), *ends)?;
                }
                StockSpec::FixedCylinder(FixedCylinder {
                    diameter,
                    length,
                    place,
                    ..
                }) => {
                    if let Some(v) = place.value() {
                        finite(&field("place"), v)?;
                    }
                    positive(&field("diameter"), *diameter)?;
                    positive(&field("length"), *length)?;
                }
                // Its model is checked above, with every stock's.
                StockSpec::Solid { .. } => {}
            }
        }

        // Only tools the operations use reach the machine, so only they must not
        // share a number. A table may keep a spare with any number it likes. The
        // machine's tool table is one, so this is across every setup.
        let mut used: BTreeMap<u32, &str> = BTreeMap::new();
        for op in self.setups.iter().flat_map(|s| &s.operations) {
            let Some((name, spec)) = op.tool.as_deref().and_then(|t| self.tools.get_key_value(t))
            else {
                continue;
            };
            match used.get(&spec.number) {
                Some(first) if *first != name.as_str() => {
                    return Err(JobError::DuplicateToolNumber {
                        number: spec.number,
                        first: (*first).to_string(),
                        second: name.clone(),
                    });
                }
                _ => {
                    used.insert(spec.number, name);
                }
            }
        }
        Ok(())
    }

    /// Check setup `index` can run, given [`check_common`](Self::check_common)
    /// passes. An index past the end is [`JobError::NoSetups`].
    pub fn check_setup(&self, index: usize) -> Result<(), JobError> {
        self.check_setup_where(index, &|_| true)
    }

    /// As [`check_setup`](Self::check_setup), checking only the operations that
    /// will run: those turned on for which `runs` holds, by their place. The
    /// rest are left out of the run, and may be unfinished.
    pub fn check_setup_where(
        &self,
        index: usize,
        runs: &dyn Fn(usize) -> bool,
    ) -> Result<(), JobError> {
        let setup = self.setups.get(index).ok_or(JobError::NoSetups)?;
        let running = || {
            setup
                .operations
                .iter()
                .enumerate()
                .filter(|(i, op)| op.enabled && runs(*i))
                .map(|(_, op)| op)
        };
        let name = &setup.name;
        setup
            .orientation
            .check()
            .map_err(|source| JobError::BadOrientation {
                setup: name.clone(),
                source,
            })?;
        if !(54..=LAST_WCS).contains(&setup.wcs) {
            return Err(JobError::BadWorkOffset {
                setup: name.clone(),
                wcs: setup.wcs,
            });
        }
        if setup
            .work_offset
            .is_some_and(|at| !at.iter().all(|v| v.is_finite()))
        {
            return Err(JobError::BadWorkOffsetPosition(name.clone()));
        }
        if let Some(z) = &setup.z_correction {
            let bad_map = |why: String| JobError::BadProbeMap {
                setup: name.clone(),
                why,
            };
            if !is_positive(z.segment) {
                return Err(bad(
                    format!("{name}.z_correction.segment"),
                    z.segment,
                    "greater than zero",
                ));
            }
            match (z.map(), &z.file) {
                (Some(Ok(_)), _) => {}
                (Some(Err(e)), _) => return Err(bad_map(format!("are no map: {e}"))),
                (None, Some(file)) => {
                    return Err(bad_map(format!("were never read from {file}")));
                }
                (None, None) => {
                    return Err(bad_map(
                        "are missing: probe the stock and load the file the controller wrote"
                            .into(),
                    ));
                }
            }
        }
        if let Some(wrap) = &setup.wrap {
            self.check_wrap(setup, wrap, &mut running())?;
        }
        if setup.models.is_empty() {
            return Err(JobError::NoModels(name.clone()));
        }
        for m in &setup.models {
            if self.model(m).is_none() {
                return Err(JobError::UnknownModel {
                    owner: format!("setup '{name}'"),
                    model: m.clone(),
                });
            }
        }
        if setup.stocks.is_empty() {
            return Err(JobError::NoStock(name.clone()));
        }
        for s in &setup.stocks {
            let Some(def) = self.stock(s) else {
                return Err(JobError::UnknownStock {
                    setup: name.clone(),
                    stock: s.clone(),
                });
            };
            // A solid's columns are its own: another block beside it would
            // be one height a column with it, and the part cut from itself
            // is no part.
            if let StockSpec::Solid { model } = &def.spec {
                let why = if setup.stocks.len() > 1 {
                    Some("it must be the setup's only stock")
                } else if setup.models.contains(model) {
                    Some("it is one of the models the setup machines")
                } else {
                    None
                };
                if let Some(why) = why {
                    return Err(JobError::SolidStock {
                        setup: name.clone(),
                        model: model.clone(),
                        why,
                    });
                }
            }
        }
        if setup.operations.is_empty() {
            return Err(JobError::NoOperations(name.clone()));
        }
        let pick = |owner: String, model: &str, at: &[f64; 3]| {
            if !setup.models.iter().any(|m| m == model) {
                return Err(JobError::PickedFromOtherModel {
                    setup: name.clone(),
                    owner,
                    model: model.to_string(),
                });
            }
            if !at.iter().all(|v| v.is_finite()) {
                return Err(JobError::BadPick { owner, at: *at });
            }
            Ok(())
        };
        if let Origin::Point { point, .. } = &setup.origin {
            pick(format!("setup '{name}' work zero"), &point.model, &point.at)?;
        }
        // Only picks something reads: ones left over from a boundary or an
        // outline since changed are kept, and not found, so cannot stop it.
        for op in running().filter(|op| op.cuts_selection() || op.heights.measured_from_selection())
        {
            for r in op.selection.iter() {
                pick(format!("operation '{name}/{}'", op.name), &r.model, &r.at)?;
            }
            // A curve is on no model: only its points are checked.
            for at in op.selection.curves.iter().flat_map(|c| c.points()) {
                if !at.iter().all(|v| v.is_finite()) {
                    let owner = format!("operation '{name}/{}'", op.name);
                    return Err(JobError::BadPick { owner, at });
                }
            }
        }

        running().try_for_each(|op| op.check(name, &self.tools))
    }
}

impl Job {
    /// What a setup machined on a rotary cannot have: a stock that is not a
    /// round bar, depths corrected to a probed surface, an operation that
    /// is not planned on the bar unrolled, or picks on the part, which are
    /// found where the part is, not where it is unrolled to.
    fn check_wrap(
        &self,
        setup: &Setup,
        wrap: &super::Wrap,
        running: &mut dyn Iterator<Item = &super::Operation>,
    ) -> Result<(), JobError> {
        let refuse = |why: String| JobError::Wrap {
            setup: setup.name.clone(),
            why,
        };
        if let Some(d) = wrap.diameter {
            positive(&format!("{}.wrap.diameter", setup.name), d)?;
        }
        if setup.z_correction.is_some() {
            return Err(refuse(
                "its depths cannot be corrected to a probed surface".into(),
            ));
        }
        for s in &setup.stocks {
            let round = self.stock(s).is_none_or(|def| {
                matches!(
                    def.spec,
                    StockSpec::CylinderAroundModels { .. } | StockSpec::FixedCylinder(_)
                )
            });
            if !round || setup.stocks.len() > 1 {
                return Err(refuse(
                    "its stock must be one cylinder, lying across the tool".into(),
                ));
            }
        }
        for op in running {
            let wraps = matches!(
                op.strategy,
                StrategySpec::Engrave { .. }
                    | StrategySpec::Slot { .. }
                    | StrategySpec::Profile { .. }
                    | StrategySpec::Pocket { .. }
                    | StrategySpec::VCarve { .. }
                    | StrategySpec::Parallel { .. }
                    | StrategySpec::CustomGcode {}
            );
            if !wraps {
                return Err(refuse(format!(
                    "operation '{}' is {}, which is not planned on a rotary: engraving, slots, \
                     2D profiles and pockets and V-carving are wrapped round it, and parallel \
                     finishing goes along or round it",
                    op.name,
                    op.strategy.name()
                )));
            }
            // A head under an overhang, unrolled, would reach in under the
            // bar's own surface round it: no undercut goes round a bar.
            if self.tool_for(op).is_some_and(|t| t.kind.undercuts()) {
                return Err(refuse(format!(
                    "operation '{}' cuts with a lollipop or slot cutter, whose head undercuts: \
                     on a rotary, use a tool that cuts from its end",
                    op.name
                )));
            }
            if (op.cuts_selection() || op.heights.measured_from_selection())
                && !(op.selection.faces.is_empty() && op.selection.edges.is_empty())
            {
                return Err(refuse(format!(
                    "operation '{}' picks faces or edges of the part, which are not found on it \
                     unrolled: give it a sketch, drawn flat as the bar unrolled",
                    op.name
                )));
            }
        }
        Ok(())
    }
}

fn unique<'a>(what: &'static str, names: impl Iterator<Item = &'a str>) -> Result<(), JobError> {
    let mut seen = BTreeSet::new();
    for n in names {
        if n.trim().is_empty() {
            return Err(JobError::NoName { what });
        }
        if !seen.insert(n) {
            return Err(JobError::DuplicateName {
                what,
                name: n.to_string(),
            });
        }
    }
    Ok(())
}

/// `field`'s `value`, which is not `range`.
fn bad(field: impl Into<String>, value: f64, range: &'static str) -> JobError {
    JobError::BadValue {
        field: field.into(),
        value,
        range,
    }
}

/// Check `field`'s `value` is finite and greater than zero.
fn positive(field: &str, value: f64) -> Result<(), JobError> {
    is_positive(value)
        .then_some(())
        .ok_or_else(|| bad(field, value, POSITIVE))
}

/// Check `field`'s `value` is finite and not below zero.
fn at_least_zero(field: &str, value: f64) -> Result<(), JobError> {
    (value.is_finite() && value >= 0.0)
        .then_some(())
        .ok_or_else(|| bad(field, value, AT_LEAST_ZERO))
}

/// Check `field`'s `value` is finite.
fn finite(field: &str, value: f64) -> Result<(), JobError> {
    value
        .is_finite()
        .then_some(())
        .ok_or_else(|| bad(field, value, FINITE))
}

impl StockToLeave {
    /// Check both are numbers, naming `field` if not.
    fn check(&self, field: &str) -> Result<(), JobError> {
        match self.unusable() {
            Some(value) => Err(bad(field, value, FINITE)),
            None => Ok(()),
        }
    }
}

impl Operation {
    /// Check this operation of setup `setup` can run with the job's `tools`: it
    /// names one the job has, the tool can do what it asks, and every value its
    /// strategy uses ([`StrategySpec::uses`]) is usable. One it does not use is
    /// not read, so is not checked. Its picks are checked with the setup's.
    pub(super) fn check(
        &self,
        setup: &str,
        tools: &BTreeMap<String, ToolSpec>,
    ) -> Result<(), JobError> {
        let at = format!("{setup}/{}", self.name);
        let field = |f: &str| format!("{at}.{f}");
        // It has no tool and nothing it cuts; what its lines say is checked
        // with the run's checks, where a person can see why.
        if matches!(self.strategy, StrategySpec::CustomGcode {}) {
            return Ok(());
        }
        if let Some(why) = self.cannot_measure_from_picks() {
            return Err(JobError::BadHeight { op: at, why });
        }
        let Some(tool) = &self.tool else {
            return Err(JobError::NoTool { op: at });
        };
        let Some(spec) = tools.get(tool) else {
            return Err(JobError::UnknownTool {
                op: at,
                tool: tool.clone(),
            });
        };
        if let Some(why) = self.cannot_use(spec) {
            return Err(JobError::WrongTool {
                op: at,
                tool: tool.clone(),
                why: why.into(),
            });
        }
        let uses = self.strategy.uses();
        self.check_heights(&field)?;
        self.check_passes(&field)?;
        self.check_linking(&field)?;
        self.check_pattern(&field)?;
        if uses.boundary {
            finite(&field("boundary.margin"), self.boundary.margin)?;
        }
        if let Some(stepdown) = self.stepdown() {
            positive(&field("passes.stepdown"), stepdown)?;
        } else if uses.depths == Depths::Required {
            return Err(JobError::NoStepdown { op: at });
        }
        self.check_strategy(&field)
    }

    /// Check the heights its strategy uses are numbers, `field` naming each.
    fn check_heights(&self, field: &dyn Fn(&str) -> String) -> Result<(), JobError> {
        let (uses, h) = (self.strategy.uses(), &self.heights);
        // The retract height is where a full retraction goes, and where a
        // strategy that stays down goes no higher than.
        let retract = uses.stay_down
            || self.linking.retraction == Retraction::Full
            || matches!(self.strategy, StrategySpec::Probe { .. });
        let used = [
            ("clearance", Some(h.clearance)),
            ("retract", retract.then_some(h.retract)),
            ("top", h.top.filter(|_| uses.top.is_some())),
            ("bottom", h.bottom.filter(|_| uses.bottom.is_some())),
        ];
        for (name, height) in used {
            if let Some(height) = height {
                finite(&field(&format!("heights.{name}.offset")), height.offset)?;
            }
        }
        Ok(())
    }

    /// Check the passes' values its strategy uses, `field` naming each.
    fn check_passes(&self, field: &dyn Fn(&str) -> String) -> Result<(), JobError> {
        let uses = self.strategy.uses();
        if uses.stepover {
            positive(&field("passes.stepover"), self.passes.stepover)?;
        }
        if uses.feed {
            positive(&field("passes.feed"), self.passes.feed)?;
        }
        if self.passes.rpm == Some(0) {
            return Err(bad(field("passes.rpm"), 0.0, "greater than zero"));
        }
        positive(&field("passes.max_cut_step"), self.passes.max_cut_step)?;
        self.stock_to_leave()
            .check(&field("passes.stock_to_leave"))?;
        if let Some(t) = self.passes.tabs.filter(|_| uses.tabs) {
            positive(&field("passes.tabs.width"), t.width)?;
            positive(&field("passes.tabs.height"), t.height)?;
            positive(&field("passes.tabs.spacing"), t.spacing)?;
        }
        if let Some(a) = self.relief().and_then(|r| r.sharper_than)
            && !(a > 0.0 && a <= 180.0)
        {
            return Err(bad(
                field("passes.relief.sharper_than"),
                a,
                "above 0 and at most 180 degrees",
            ));
        }
        if let Some(f) = self.passes.finishing.filter(|_| uses.finishing) {
            if f.passes == Some(0) {
                return Err(bad(field("passes.finishing.passes"), 0.0, "at least one"));
            }
            if let Some(s) = f.stepover {
                positive(&field("passes.finishing.stepover"), s)?;
            }
            if let Some(v) = f.feed {
                positive(&field("passes.finishing.feed"), v)?;
            }
        }
        Ok(())
    }

    /// Check the linking values its strategy uses, `field` naming each.
    fn check_linking(&self, field: &dyn Fn(&str) -> String) -> Result<(), JobError> {
        let (uses, l) = (self.strategy.uses(), &self.linking);
        positive(&field("linking.retract_feed"), l.retract_feed)?;
        positive(&field("linking.plunge_feed"), l.plunge_feed)?;
        // Every strategy's links are made above the material.
        at_least_zero(&field("linking.safe_distance"), l.safe_distance)?;
        if uses.stay_down
            && let Some(d) = l.stay_down
        {
            at_least_zero(&field("linking.stay_down"), d)?;
        }
        if uses.lead_radius
            && let Some(r) = l.lead_radius
        {
            at_least_zero(&field("linking.lead_radius"), r)?;
        }
        if uses.ramp {
            if !(l.ramp_angle > 0.0 && l.ramp_angle < 90.0) {
                return Err(bad(
                    field("linking.ramp_angle"),
                    l.ramp_angle,
                    "greater than 0 and less than 90 degrees",
                ));
            }
            if let Some(f) = l.ramp_feed {
                positive(&field("linking.ramp_feed"), f)?;
            }
            if let Some(d) = l.ramp_stepdown {
                positive(&field("linking.ramp_stepdown"), d)?;
            }
        }
        if let Some(leads) = l.leads.filter(|_| uses.leads) {
            if let Some(r) = leads.radius {
                positive(&field("linking.leads.radius"), r)?;
            }
            if !(leads.sweep > 0.0 && leads.sweep <= 180.0) {
                return Err(bad(
                    field("linking.leads.sweep"),
                    leads.sweep,
                    "greater than 0 and at most 180 degrees",
                ));
            }
            for (name, v) in [("straight", leads.straight), ("vertical", leads.vertical)] {
                if let Some(v) = v {
                    at_least_zero(&field(&format!("linking.leads.{name}")), v)?;
                }
            }
        }
        if let Some(e) = l.entry
            && !uses.entries.is_empty()
            && !uses.entries.contains(&e)
        {
            return Err(JobError::NotOffered {
                field: field("linking.entry"),
                value: e.name(),
                strategy: self.strategy.name(),
            });
        }
        if !uses.entries.is_empty()
            && let Some(d) = l.helix_diameter
        {
            positive(&field("linking.helix_diameter"), d)?;
        }
        Ok(())
    }

    /// Check its pattern, `field` naming each value: counts of one or more,
    /// no more copies in all than [`super::MOST_COPIES`], finite spacings and
    /// points, and a polar spread of more than nothing, no more than a turn.
    fn check_pattern(&self, field: &dyn Fn(&str) -> String) -> Result<(), JobError> {
        let p = &self.transform;
        if let Some(l) = p.linear {
            for (name, n) in [("x_count", l.x_count), ("y_count", l.y_count)] {
                if n == 0 {
                    return Err(bad(
                        field(&format!("transform.linear.{name}")),
                        0.0,
                        "1 or more",
                    ));
                }
            }
            finite(&field("transform.linear.x_spacing"), l.x_spacing)?;
            finite(&field("transform.linear.y_spacing"), l.y_spacing)?;
        }
        if let Some(r) = p.polar {
            if r.count == 0 {
                return Err(bad(field("transform.polar.count"), 0.0, "1 or more"));
            }
            if !(r.angle.is_finite() && r.angle > 0.0 && r.angle <= 360.0) {
                return Err(bad(
                    field("transform.polar.angle"),
                    r.angle,
                    "more than 0 and at most 360 degrees",
                ));
            }
            for v in r.centre {
                finite(&field("transform.polar.centre"), v)?;
            }
        }
        if let Some(m) = p.mirror {
            for v in m.through {
                finite(&field("transform.mirror.through"), v)?;
            }
        }
        if p.copies() > super::MOST_COPIES {
            return Err(bad(
                field("transform"),
                f64::from(p.copies()),
                "at most 1000 copies in all",
            ));
        }
        Ok(())
    }

    /// Check its strategy's own values, `field` naming each.
    fn check_strategy(&self, field: &dyn Fn(&str) -> String) -> Result<(), JobError> {
        if let StrategySpec::Face {
            angle,
            pass_extension,
            stock_offset,
            ..
        } = self.strategy
        {
            finite(&field("angle"), angle)?;
            at_least_zero(&field("pass_extension"), pass_extension)?;
            finite(&field("stock_offset"), stock_offset)?;
        }
        if let Some((slope_from, slope_to)) = self.strategy.slopes() {
            if !(slope_from.is_finite() && (0.0..90.0).contains(&slope_from)) {
                return Err(bad(
                    field("slope_from"),
                    slope_from,
                    "at least 0 and less than 90 degrees",
                ));
            }
            if !(slope_to.is_finite() && slope_from < slope_to && slope_to <= 90.0) {
                return Err(bad(
                    field("slope_to"),
                    slope_to,
                    "greater than slope_from and at most 90 degrees",
                ));
            }
        }
        if let StrategySpec::Spiral {
            centre,
            inner_radius,
            outer_radius,
            ..
        }
        | StrategySpec::Radial {
            centre,
            inner_radius,
            outer_radius,
            ..
        } = self.strategy
        {
            check_centred(field, centre, inner_radius, outer_radius)?;
        }
        if let StrategySpec::MorphedSpiral { centre, .. } = self.strategy {
            // Its centre alone: it has no radii.
            check_centred(field, centre, 0.0, 0.0)?;
        }
        if let StrategySpec::Radial {
            angle_from,
            angle_to,
            ..
        } = self.strategy
        {
            finite(&field("angle_from"), angle_from)?;
            if !(angle_to.is_finite() && angle_from < angle_to && angle_to <= angle_from + 360.0) {
                return Err(bad(
                    field("angle_to"),
                    angle_to,
                    "greater than angle_from and at most a turn past it",
                ));
            }
        }
        if let StrategySpec::Engrave { depth, .. } | StrategySpec::Slot { depth, .. } =
            self.strategy
        {
            positive(&field("depth"), depth)?;
        }
        if let StrategySpec::Chamfer {
            width, tip_offset, ..
        } = self.strategy
        {
            positive(&field("width"), width)?;
            at_least_zero(&field("tip_offset"), tip_offset)?;
        }
        if let StrategySpec::Adaptive { rest: Some(r), .. } = self.strategy {
            at_least_zero(&field("rest.adjustment"), r.adjustment)?;
        }
        if let StrategySpec::Bore { pitch, .. } = self.strategy {
            positive(&field("pitch"), pitch)?;
        }
        if let StrategySpec::Probe {
            x_count,
            y_count,
            x_spacing,
            y_spacing,
            inset,
            probe_feed,
            max_depth,
        } = self.strategy
        {
            for (name, count) in [("x_count", x_count), ("y_count", y_count)] {
                if count == Some(0) {
                    return Err(bad(field(name), 0.0, "at least one"));
                }
            }
            positive(&field("x_spacing"), x_spacing)?;
            positive(&field("y_spacing"), y_spacing)?;
            at_least_zero(&field("inset"), inset)?;
            positive(&field("probe_feed"), probe_feed)?;
            positive(&field("max_depth"), max_depth)?;
        }
        if let StrategySpec::Circular { boss_width, .. } = self.strategy {
            at_least_zero(&field("boss_width"), boss_width)?;
        }
        if let StrategySpec::Thread {
            pitch,
            diameter,
            radial_passes,
            ..
        } = self.strategy
        {
            positive(&field("pitch"), pitch)?;
            if let Some(d) = diameter {
                positive(&field("diameter"), d)?;
            }
            if radial_passes == 0 {
                return Err(bad(field("radial_passes"), 0.0, "at least one"));
            }
        }
        if let StrategySpec::VCarve { max_depth: Some(d) } = self.strategy {
            positive(&field("max_depth"), d)?;
        }
        if let StrategySpec::Drill {
            peck,
            dwell,
            chip_break,
            pitch,
            ..
        } = self.strategy
        {
            if let Some(p) = peck {
                positive(&field("peck"), p)?;
            }
            if let Some(d) = dwell {
                at_least_zero(&field("dwell"), d)?;
            }
            if let Some(c) = chip_break {
                positive(&field("chip_break"), c)?;
                if peck.is_none() {
                    return Err(bad(field("chip_break"), c, "given only with a peck"));
                }
            }
            // A tap goes in and back out in one: a peck, or a pause at the
            // bottom with the spindle turning, would cut the thread again.
            if let Some(p) = pitch {
                positive(&field("pitch"), p)?;
                for (name, v) in [("peck", peck), ("chip_break", chip_break), ("dwell", dwell)] {
                    if let Some(v) = v {
                        return Err(bad(field(name), v, "left out when tapping"));
                    }
                }
            }
        }
        Ok(())
    }
}

/// Check a pattern about a centre's centre and radii, `field` naming each: a
/// finite centre, and radii from zero with the inner inside the outer where the
/// outer is given.
fn check_centred(
    field: &dyn Fn(&str) -> String,
    centre: Option<[f64; 2]>,
    inner: f64,
    outer: f64,
) -> Result<(), JobError> {
    for v in centre.into_iter().flatten() {
        finite(&field("centre"), v)?;
    }
    at_least_zero(&field("inner_radius"), inner)?;
    if !(outer.is_finite() && outer >= 0.0 && (outer == 0.0 || inner < outer)) {
        return Err(bad(
            field("outer_radius"),
            outer,
            "zero, or finite and greater than inner_radius",
        ));
    }
    Ok(())
}
