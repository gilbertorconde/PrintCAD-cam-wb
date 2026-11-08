//! A setup made ready to run: its models merged and turned the way the part sits,
//! its stock laid out, and everything measured from its work zero.
//!
//! Past this point everything is in the setup frame. Strategies, the linker, the
//! simulator and the post never see a rotation or a model coordinate. The one way
//! back is [`Prepared::frame`]: a pick is found on its own model, where its point
//! is in model coordinates, and brought into the frame ([`crate::setup::features`]).
//! [`Prepared::pieces`] names the models the part was merged from.
//!
//! A setup square to the model's axes lays its stock out in model coordinates
//! and turns it, so two setups of one block see the same material. A tilted one
//! ([`frame::Turn::Tilted`]) lays it out in its own axes instead, round the
//! models as it sees them: a box turned by a tilt is not a box along the
//! setup's axes, and the simulator's stock is.
//!
//! What every operation of the setup reads from the part alone is worked out
//! once, when first asked for, and kept with it: its shadow from above.
//!
//! A setup may carry on from an earlier one ([`Carried`]), as Fusion's take
//! their stock from the setup before: its stock is then what that one left,
//! the same material turned the way this one sees it, and its box the box
//! round that.

pub mod features;
pub mod frame;
mod round;

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

use camcraft_geom::contour::RegionError;
use camcraft_geom::mesh::{Aabb, MeshError, TriMesh};
use camcraft_geom::shadow::{Shadow, shadow_above};
use camcraft_geom::topology::Body;
use camcraft_sim::stock::{Axis as FrameAxis, Bar, Block, Carry, Stock};
use glam::{DAffine3, DMat3, DVec3};

use crate::job::{BoxOf, HeightContext, Job, JobError, Origin, Setup, StockSpec};
use features::FeatureError;
use frame::{BadOrientation, Frame, Turn};

/// A setup's models, by name, in model coordinates, with their topology.
pub type Bodies<'a> = BTreeMap<&'a str, &'a Body>;

/// A setup that could not be prepared.
#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    /// The job did not describe it fully.
    #[error(transparent)]
    Job(#[from] JobError),
    /// A model it needs was not given.
    #[error("model '{0}' is not loaded")]
    NotLoaded(String),
    /// A model it needs has no geometry.
    #[error("model '{0}' has no geometry")]
    EmptyModel(String),
    /// Its orientation cannot be worked out.
    #[error(transparent)]
    Orientation(#[from] BadOrientation),
    /// The models could not be merged.
    #[error(transparent)]
    Mesh(#[from] MeshError),
    /// The work zero was picked on something that is no longer there.
    #[error("work zero: {0}")]
    Feature(#[from] FeatureError),
    /// The setup it carries on from left no material.
    #[error("the setup it carries on from left no material")]
    NothingCarried,
    /// It is on a rotary, and cannot be laid out as it is.
    #[error("on a rotary: {0}")]
    Wrap(String),
}

/// The material an earlier setup left, for a setup that carries on from it.
#[derive(Debug, Clone)]
pub struct Carried {
    /// What it left, in its own frame, with an underside
    /// ([`Stock::with_bottoms`]): the material the other way up is its bottom.
    pub stock: Arc<Stock>,
    /// Its stock's box, in its own frame.
    pub within: Aabb,
    /// From its frame to model coordinates.
    pub to_model: DAffine3,
}

/// `f`, a turn and a move, as an affine map.
pub fn affine_of(f: impl Fn(DVec3) -> DVec3) -> DAffine3 {
    let o = f(DVec3::ZERO);
    DAffine3 {
        matrix3: DMat3::from_cols(f(DVec3::X) - o, f(DVec3::Y) - o, f(DVec3::Z) - o),
        translation: o,
    }
}

/// One of the models merged into the part.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    /// The model, by name.
    pub model: String,
}

/// A setup in its own frame.
///
/// Its part is not to be changed once laid out: what is worked out from it is
/// kept.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// Every model the setup machines, as one mesh.
    pub part: TriMesh,
    /// The models it was merged from, in the setup's order.
    pub pieces: Vec<Piece>,
    /// Its bounds.
    pub model_bounds: Aabb,
    /// Each stock block.
    pub stocks: Vec<Block>,
    /// How model coordinates became these.
    pub frame: Frame,
    /// The material it starts from, when it carries on from an earlier setup:
    /// that one's, and how to carry it into this frame.
    pub carried: Option<(Arc<Stock>, Carry)>,
    /// The part's shadow, once cast ([`Prepared::shadow`]).
    shadow: OnceLock<Shadow>,
}

impl Prepared {
    /// Model `model`, if the part was merged from it.
    pub fn piece(&self, model: &str) -> Option<&Piece> {
        self.pieces.iter().find(|p| p.model == model)
    }

    /// The box around every stock block.
    pub fn stock_bounds(&self) -> Aabb {
        bounds_all(&self.stocks).unwrap_or(self.model_bounds)
    }

    /// What a height resolves against in this setup: its stock's top and
    /// bottom, and its models'. No picks: an operation's are its own.
    pub fn heights(&self) -> HeightContext {
        let all = self.stock_bounds();
        HeightContext {
            stock_top: all.max.z,
            stock_bottom: all.min.z,
            model_top: self.model_bounds.max.z,
            model_bottom: self.model_bounds.min.z,
            picked: None,
        }
    }

    /// The part's shadow from straight above, cast onto a level below it:
    /// everything it covers, seen from the top. Its silhouette, and where a
    /// tool can touch it, are grown from this.
    ///
    /// Cast once, when first asked for, and kept for every operation after. A
    /// shadow that cannot be cast is not kept, since its error cannot be: it
    /// is cast again, and fails again, each time it is asked for.
    pub fn shadow(&self) -> Result<&Shadow, RegionError> {
        if let Some(cast) = self.shadow.get() {
            return Ok(cast);
        }
        let cast = shadow_above(&self.part, self.model_bounds.min.z - 1.0)?;
        Ok(self.shadow.get_or_init(|| cast))
    }
}

/// The box around the named models, in model coordinates, or with `view` in
/// the coordinates it turns them into.
fn bounds_of(
    names: &[String],
    bodies: &Bodies<'_>,
    view: Option<&Turn>,
) -> Result<Aabb, PrepareError> {
    let mut out: Option<Aabb> = None;
    for n in names {
        let body = bodies
            .get(n.as_str())
            .ok_or_else(|| PrepareError::NotLoaded(n.clone()))?;
        let b = match view {
            None => body.mesh().bounds(),
            Some(t) => body.mesh().vertices().iter().map(|v| t.apply(*v)).fold(
                None,
                |b: Option<Aabb>, p| {
                    Some(b.map_or(Aabb { min: p, max: p }, |b| Aabb {
                        min: b.min.min(p),
                        max: b.max.max(p),
                    }))
                },
            ),
        }
        .ok_or_else(|| PrepareError::EmptyModel(n.clone()))?;
        out = Some(out.map_or(b, |o| o.union(&b)));
    }
    out.ok_or_else(|| PrepareError::Job(JobError::NoModels(String::new())))
}

/// The box around every one of `blocks`.
pub fn bounds_all(blocks: &[Block]) -> Option<Aabb> {
    let boxes: Vec<Aabb> = blocks.iter().map(Block::bounds).collect();
    Aabb::union_all(&boxes)
}

/// A stock block in model coordinates. One around no named models is around
/// `setup_models`, the models of the setup using it.
///
/// With `view`, a tilted setup's turn, it is laid out in the coordinates that
/// turns the models into instead: its sides, margins and bar along the
/// setup's axes, round the models as the setup sees them.
fn block(
    spec: &StockSpec,
    setup_models: &[String],
    bodies: &Bodies<'_>,
    view: Option<&Turn>,
) -> Result<Block, PrepareError> {
    let named = spec.models();
    let models = if named.is_empty() {
        setup_models
    } else {
        named
    };
    let around = bounds_of(models, bodies, view)?;
    Ok(match spec {
        StockSpec::AroundModels { margin, below, .. } => Block::Box(Aabb {
            min: DVec3::new(
                around.min.x - margin[0],
                around.min.y - margin[1],
                around.min.z - below,
            ),
            max: DVec3::new(
                around.max.x + margin[0],
                around.max.y + margin[1],
                around.max.z + margin[2],
            ),
        }),
        StockSpec::FixedBox(b) => {
            let ([px, py, pz], [sx, sy, sz]) = (b.place, b.size);
            let (lo, hi) = (around.min, around.max);
            let min = DVec3::new(
                px.low(lo.x, hi.x, sx),
                py.low(lo.y, hi.y, sy),
                pz.low(lo.z, hi.z, sz),
            );
            Block::Box(Aabb {
                min,
                max: min + DVec3::from_array(b.size),
            })
        }
        StockSpec::CylinderAroundModels {
            axis, margin, ends, ..
        } => {
            let a = axis.vector();
            let centre = around.center();
            // The farthest any of them reaches from the axis through the
            // middle of their box.
            let mut reach: f64 = 0.0;
            for n in models {
                let body = bodies
                    .get(n.as_str())
                    .ok_or_else(|| PrepareError::NotLoaded(n.clone()))?;
                for v in body.mesh().vertices() {
                    let off = view.map_or(*v, |t| t.apply(*v)) - centre;
                    reach = reach.max((off - a * off.dot(a)).length());
                }
            }
            let along = (around.max - around.min).dot(a);
            Block::Bar(Bar {
                axis: FrameAxis::of(axis.index()),
                centre,
                radius: reach + margin,
                length: along + 2.0 * ends,
            })
        }
        StockSpec::FixedCylinder(c) => {
            let a = c.axis.vector();
            let (lo, hi) = (around.min.dot(a), around.max.dot(a));
            let middle = c.place.low(lo, hi, c.length) + c.length * 0.5;
            let centre = around.center();
            Block::Bar(Bar {
                axis: FrameAxis::of(c.axis.index()),
                centre: centre + a * (middle - centre.dot(a)),
                radius: c.diameter * 0.5,
                length: c.length,
            })
        }
    })
}

/// A stock block in model coordinates, for a setup machining `models`: the box
/// around it, wherever it is, whichever setup it is shown for.
pub fn stock_block(
    spec: &StockSpec,
    models: &[String],
    bodies: &Bodies<'_>,
) -> Result<Aabb, PrepareError> {
    Ok(block(spec, models, bodies, None)?.bounds())
}

/// A stock block as a setup turned by `t` has it, before its work zero: one
/// laid out in model coordinates and turned, when the turn is square, and
/// otherwise one laid out in the setup's axes ([`block`]), since a box turned
/// any other way is not a box along them.
fn laid(
    spec: &StockSpec,
    models: &[String],
    bodies: &Bodies<'_>,
    t: &Turn,
) -> Result<Block, PrepareError> {
    if t.square().is_some() {
        Ok(turn(t, &block(spec, models, bodies, None)?))
    } else {
        block(spec, models, bodies, Some(t))
    }
}

/// `b` turned the way the part sits, by a square turn. That only swaps and
/// negates, so a box stays a box, its corners' low and high ends the new box,
/// and a bar along an axis lies along another.
fn turn(o: &Turn, b: &Block) -> Block {
    match b {
        Block::Box(b) => {
            let (a, c) = (o.apply(b.min), o.apply(b.max));
            Block::Box(Aabb {
                min: a.min(c),
                max: a.max(c),
            })
        }
        Block::Bar(bar) => {
            let along = o.apply(match bar.axis {
                FrameAxis::X => DVec3::X,
                FrameAxis::Y => DVec3::Y,
                FrameAxis::Z => DVec3::Z,
            });
            let axis = if along.x != 0.0 {
                FrameAxis::X
            } else if along.y != 0.0 {
                FrameAxis::Y
            } else {
                FrameAxis::Z
            };
            Block::Bar(Bar {
                axis,
                centre: o.apply(bar.centre),
                ..*bar
            })
        }
    }
}

/// Where a stock block would be in a setup's `frame`, were the setup to use it:
/// for showing a stock the setup does not use, to be picked into it. `models` are
/// the setup's, which a block around no named models is around.
pub fn stock_in_frame(
    spec: &StockSpec,
    models: &[String],
    frame: &Frame,
    bodies: &Bodies<'_>,
) -> Result<Aabb, PrepareError> {
    let b = laid(spec, models, bodies, &frame.turn)?;
    Ok(b.bounds().translated(-frame.origin))
}

/// Where setup `index`'s part and stock are, in its own frame.
pub fn prepare(job: &Job, index: usize, bodies: &Bodies<'_>) -> Result<Prepared, PrepareError> {
    prepare_from(job, index, bodies, None)
}

/// [`prepare`], the stock what `carried` left when it is given, in place of
/// the setup's own: its one block the box round that material, as this setup
/// sees it, which a work zero on the stock is placed on.
pub fn prepare_from(
    job: &Job,
    index: usize,
    bodies: &Bodies<'_>,
    carried: Option<&Carried>,
) -> Result<Prepared, PrepareError> {
    let setup: &Setup = job.setups.get(index).ok_or(JobError::NoSetups)?;
    if setup.models.is_empty() {
        return Err(JobError::NoModels(setup.name.clone()).into());
    }

    // The part, in model coordinates.
    let parts = setup
        .models
        .iter()
        .map(|n| {
            bodies
                .get(n.as_str())
                .map(|b| b.mesh())
                .ok_or_else(|| PrepareError::NotLoaded(n.clone()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let o = setup.orientation.resolve(bodies, job.tolerance)?;
    // From the earlier setup's frame to this one's, turned but before its
    // work zero.
    let carry = carried.map(|c| Carry {
        to: affine_of(|p| o.apply(p)) * c.to_model,
        within: c.within,
    });
    let turned_blocks = match (carried, &carry) {
        (Some(c), Some(carry)) => vec![Block::Box(
            c.stock
                .carried_bounds(carry)
                .ok_or(PrepareError::NothingCarried)?,
        )],
        _ => blocks_of(job, setup, bodies, &o)?,
    };
    let turned_model = if o.square().is_some() {
        turn(&o, &Block::Box(bounds_of(&setup.models, bodies, None)?)).bounds()
    } else {
        bounds_of(&setup.models, bodies, Some(&o))?
    };
    let origin = work_zero(job, setup, bodies, &o, &turned_blocks, turned_model)?;
    let mut frame = Frame {
        turn: o,
        origin,
        wrap: None,
    };

    let merged;
    let part_in_model = if let [one] = parts.as_slice() {
        *one
    } else {
        merged = TriMesh::merged(&parts)?;
        &merged
    };
    let pieces = setup
        .models
        .iter()
        .map(|m| Piece { model: m.clone() })
        .collect();
    let (part, stocks) = match &setup.wrap {
        Some(wrap) => {
            if carried.is_some() {
                return Err(PrepareError::Wrap(
                    "its stock cannot carry on from an earlier setup".into(),
                ));
            }
            let (unrolled, part, stock) = round::unrolled(
                wrap,
                &mut frame,
                &turned_blocks,
                part_in_model,
                job.tolerance,
            )?;
            frame.wrap = Some(unrolled);
            (part, vec![stock])
        }
        None => (
            part_in_model.mapped(|p| frame.to_setup(p))?,
            turned_blocks
                .iter()
                .map(|b| b.translated(-origin))
                .collect(),
        ),
    };
    let origin = frame.origin;
    let model_bounds = part
        .bounds()
        .ok_or_else(|| PrepareError::EmptyModel(setup.name.clone()))?;
    Ok(Prepared {
        part,
        pieces,
        model_bounds,
        stocks,
        frame,
        carried: carried.zip(carry).map(|(c, carry)| {
            let carry = Carry {
                to: DAffine3::from_translation(-origin) * carry.to,
                ..carry
            };
            (Arc::clone(&c.stock), carry)
        }),
        shadow: OnceLock::new(),
    })
}

/// Each of `setup`'s stock blocks, turned by `t` as the part sits ([`laid`]). A
/// block around no named models is around the setup's.
fn blocks_of(
    job: &Job,
    setup: &Setup,
    bodies: &Bodies<'_>,
    t: &Turn,
) -> Result<Vec<Block>, PrepareError> {
    let mut blocks = Vec::with_capacity(setup.stocks.len());
    for name in &setup.stocks {
        let def = job.stock(name).ok_or_else(|| JobError::UnknownStock {
            setup: setup.name.clone(),
            stock: name.clone(),
        })?;
        blocks.push(laid(&def.spec, &setup.models, bodies, t)?);
    }
    if blocks.is_empty() {
        return Err(JobError::NoStock(setup.name.clone()).into());
    }
    Ok(blocks)
}

/// Where `setup`'s work zero is, with the part turned the way it sits, by `o`:
/// its blocks are `turned_blocks` and its models' box `turned_model`, both
/// turned.
fn work_zero(
    job: &Job,
    setup: &Setup,
    bodies: &Bodies<'_>,
    o: &Turn,
    turned_blocks: &[Block],
    turned_model: Aabb,
) -> Result<DVec3, PrepareError> {
    Ok(match &setup.origin {
        Origin::ModelOrigin { offset } => DVec3::from_array(*offset),
        Origin::Point { point, offset } => {
            let body = bodies
                .get(point.model.as_str())
                .ok_or_else(|| FeatureError::NotLoaded(point.model.clone()))?;
            let tol = features::tolerance(job.tolerance, body);
            let (p, _) = features::find_point(body, point, tol)?;
            o.apply(p) + DVec3::from_array(*offset)
        }
        &Origin::BoxPoint {
            of,
            x,
            y,
            z,
            offset,
        } => {
            let b = match of {
                BoxOf::Stock => bounds_all(turned_blocks).unwrap_or(turned_model),
                BoxOf::Model => turned_model,
            };
            DVec3::new(
                x.pick(b.min.x, b.max.x),
                y.pick(b.min.y, b.max.y),
                z.pick(b.min.z, b.max.z),
            ) + DVec3::from_array(offset)
        }
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::float_cmp,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests;
