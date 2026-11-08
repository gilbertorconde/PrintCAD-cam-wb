//! What an operation machines, drawn over the stock as camcraft's app draws
//! it: a 3D operation's machining boundary as chosen, where the tool's
//! centre may go against it, and what the tool cuts outside the part; a 2D
//! operation's outline at the height of what is picked in it, and what the
//! tool cuts beside it (a band for a profile, all inside for a pocket); the
//! curves an engraving or a slot follows at its top, and the band the tool
//! cuts along them; a chamfer's edges, and the band its cone covers beside
//! them; and the areas a V-carving carves, all of them cut.

use cam_protocol::{CutPiece, Outlines};
use camcraft_core::cutter::allowance::EffectiveTool;
use camcraft_core::job::{Boundary, Job, Operation, StrategySpec};
use camcraft_core::path::pattern::copies;
use camcraft_core::run::{chamfer_centres, cutting_tool};
use camcraft_core::setup::features::Picking;
use camcraft_core::setup::frame::Frame;
use camcraft_core::setup::{Bodies, Prepared, features};
use camcraft_core::strategy::engrave::Trace;
use camcraft_core::strategy::planar::{centre_offset, picked_pieces};
use camcraft_geom::contour::Region;
use camcraft_sim::stock::SweptTool;
use glam::{DVec2, DVec3};

use crate::to_f32;

/// What the outlines of a setup's operations are worked out from.
pub(crate) struct Ground<'a> {
    pub job: &'a Job,
    pub prepared: &'a Prepared,
    pub bodies: &'a Bodies<'a>,
    pub frame: &'a Frame,
    /// The stock's top, in the setup's frame.
    pub top: f64,
}

impl Ground<'_> {
    /// A line through `points`, in the setup's paths' frame, in the
    /// model's: round a rotary, following it between them.
    fn drawn(&self, points: &[DVec3]) -> Vec<[f32; 3]> {
        let Some((&first, rest)) = points.split_first() else {
            return Vec::new();
        };
        std::iter::once(first)
            .chain(crate::rolled(self.frame, first, rest.to_vec()))
            .map(|p| to_f32(self.frame.shown(p)))
            .collect()
    }

    fn rings(&self, region: &Region, z: f64) -> Vec<Vec<[f32; 3]>> {
        region
            .rings()
            .iter()
            .map(|r| self.ring(r.points(), z))
            .collect()
    }

    fn ring(&self, points: &[DVec2], z: f64) -> Vec<[f32; 3]> {
        let mut ends: Vec<DVec3> = points.iter().map(|p| p.extend(z)).collect();
        if let Some(first) = ends.first().copied() {
            ends.push(first);
        }
        self.drawn(&ends)
    }

    /// `region`'s pieces at height `z`, each with its holes.
    fn pieces(&self, region: &Region, z: f64) -> Vec<CutPiece> {
        region
            .pieces()
            .iter()
            .filter_map(|piece| {
                let (outer, holes): (Vec<_>, Vec<_>) =
                    piece.rings().iter().partition(|r| r.signed_area() > 0.0);
                Some(CutPiece {
                    outline: self.ring(outer.first()?.points(), z),
                    holes: holes.iter().map(|h| self.ring(h.points(), z)).collect(),
                })
            })
            .collect()
    }

    /// What operation `op` machines, drawn: none for one with no boundary,
    /// or where it cannot be worked out.
    /// A pattern's copies are drawn too, each where its toolpath goes.
    pub fn outlines(&self, op: &Operation) -> Option<Outlines> {
        let uses = op.strategy.uses();
        let tool = cutting_tool(self.job, op).ok();
        let tolerance = self.job.tolerance;
        let one = if uses.outline {
            self.outline(op, tool.as_ref(), tolerance)
        } else if uses.boundary {
            self.boundary(op, tool.as_ref(), tolerance)
        } else if uses.traces {
            self.traces(op, tool.as_ref(), tolerance)
        } else if uses.carves {
            self.carved(op, tolerance)
        } else {
            None
        }?;
        if !op.transform.repeats() {
            return Some(one);
        }
        let mut out = Outlines::default();
        for c in copies(&op.transform) {
            let ring = |r: &Vec<[f32; 3]>| -> Vec<[f32; 3]> {
                r.iter()
                    .map(|p| {
                        let q = DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
                        to_f32(self.frame.shown(c.point(self.frame.unshown(q))))
                    })
                    .collect()
            };
            out.boundary.extend(one.boundary.iter().map(ring));
            out.limit.extend(one.limit.iter().map(ring));
            out.cut.extend(one.cut.iter().map(|p| CutPiece {
                outline: ring(&p.outline),
                holes: p.holes.iter().map(ring).collect(),
            }));
        }
        Some(out)
    }

    /// Where `op` cuts down from, its picks aside: its own top, or its
    /// strategy's.
    fn op_top(&self, op: &Operation) -> f64 {
        op.levels_3d(&self.prepared.heights()).0
    }

    /// An engraving's curves at its top, open ones left open, and the band
    /// the tool cuts along them. A chamfer's at their own heights, the
    /// edges', and the band its cone covers there, beside each edge away
    /// from the material.
    fn traces(
        &self,
        op: &Operation,
        tool: Option<&EffectiveTool>,
        tolerance: f64,
    ) -> Option<Outlines> {
        let picking = Picking::new(&op.selection, self.prepared, self.bodies, tolerance);
        let traces = picking.traces().ok()?;
        let chamfer = chamfer_centres(self.job, op, self.prepared, self.bodies);
        let top = self.op_top(op);
        let z = |t: &Trace| if chamfer.is_some() { t.start.z } else { top };
        let mut out = Outlines::default();
        let mut lines = Vec::new();
        for t in &traces {
            let points: Vec<DVec2> = t.points(tolerance).iter().map(|p| p.truncate()).collect();
            let ends: Vec<DVec3> = points.iter().map(|p| p.extend(z(t))).collect();
            out.boundary.push(self.drawn(&ends));
            lines.push(points);
        }
        if let Some((centres, place)) = &chamfer {
            let lines: Vec<Vec<DVec2>> = centres
                .iter()
                .map(|t| t.points(tolerance).iter().map(|p| p.truncate()).collect())
                .collect();
            let at = centres.iter().map(|t| t.start.z).fold(f64::MIN, f64::max);
            let band = Region::around_polylines(&lines, place.reach, tolerance / 2.0);
            out.cut = self.pieces(&band, at);
        } else if let Some(tool) = tool {
            let band = Region::around_polylines(&lines, tool.cutter_radius(), tolerance / 2.0);
            out.cut = self.pieces(&band, top);
        }
        Some(out)
    }

    /// A V-carving's areas at its top, all of them cut.
    fn carved(&self, op: &Operation, tolerance: f64) -> Option<Outlines> {
        let area = features::boundary_shape(
            Boundary::Selection,
            &op.selection,
            self.prepared,
            self.bodies,
            tolerance,
        )
        .ok()??;
        let z = self.op_top(op);
        Some(Outlines {
            boundary: self.rings(&area, z),
            limit: Vec::new(),
            cut: self.pieces(&area, z),
        })
    }

    fn boundary(
        &self,
        op: &Operation,
        tool: Option<&EffectiveTool>,
        tolerance: f64,
    ) -> Option<Outlines> {
        let b = &op.boundary;
        let part = self.prepared.shadow().ok()?.area().clone();
        let shape = match b.kind {
            Boundary::Silhouette => part.clone(),
            kind => {
                features::boundary_shape(kind, &op.selection, self.prepared, self.bodies, tolerance)
                    .ok()??
            }
        };
        let radius = tool.map_or(0.0, EffectiveTool::cutter_radius);
        let arc = tolerance / 2.0;
        let limit = shape.offset(features::containment_offset(b, radius), arc);
        let z = self.top;
        let mut out = Outlines {
            boundary: self.rings(&shape, z),
            limit: self.rings(&limit, z),
            cut: Vec::new(),
        };
        if let Some(tool) = tool
            && let Ok(centres) = limit.difference(&part.offset(tool.max_radius(), arc))
            && let Ok(cut) = centres.offset(radius, arc).difference(&part)
        {
            out.cut = self.pieces(&cut, z);
        }
        Some(out)
    }

    fn outline(
        &self,
        op: &Operation,
        tool: Option<&EffectiveTool>,
        tolerance: f64,
    ) -> Option<Outlines> {
        let (shape, picked) = features::outline(op, self.prepared, self.bodies, tolerance).ok()?;
        let shape = shape.unwrap_or_else(|| {
            let b = self.prepared.stock_bounds();
            Region::rectangle(b.min.truncate(), b.max.truncate())
        });
        let arc = tolerance / 2.0;
        let radius = tool.map_or(0.0, EffectiveTool::cutter_radius);
        let offset = centre_offset(op, radius);
        let pocket = matches!(op.strategy, StrategySpec::Pocket { .. });
        let mut out = Outlines::default();
        for (piece, picks) in picked_pieces(&shape, arc, &picked) {
            // On the picks in it, or at the stock's top with none.
            let z = picks.map_or(self.top, |p| p.lowest);
            out.boundary.extend(self.rings(&piece, z));
            if radius <= 0.0 {
                continue;
            }
            let centres = piece.offset(offset, arc);
            if centres.area() <= 1e-9 {
                continue;
            }
            let reach = centres.offset(radius, arc);
            let cut = if pocket {
                Some(reach)
            } else {
                reach.difference(&centres.offset(-radius, arc)).ok()
            };
            if let Some(cut) = cut {
                out.cut.extend(self.pieces(&cut, z));
            }
        }
        Some(out)
    }
}
