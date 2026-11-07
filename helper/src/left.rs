//! The material a setup leaves, for a setup that carries on from it.
//!
//! A setup that carries on from an earlier one is asked with that one's own
//! request beside it ([`Request::from`]). Each run the helper answers keeps
//! what its setup left in the cache, by what it was run from, so the later
//! setup takes it from there; when it is not there (the cache is off, or has
//! moved on), the earlier setup is run again for it, and that one's earlier
//! one before it.
//!
//! What is kept is the simulator's material, its height field and its
//! underside, packed, with the box it is within and how its frame sits in
//! the document's: everything the later setup turns it by.

use std::path::Path;
use std::sync::Arc;

use cam_protocol::Request;
use camcraft_core::setup::Carried;
use camcraft_geom::mesh::Aabb;
use camcraft_session::{Ran, parse_job, run_setup_from};
use camcraft_sim::stock::PackedStock;
use glam::{DAffine3, DMat3, DVec3};

use crate::{cache, load, speaks};

/// What a kept setup's material starts with.
const MAGIC: &[u8] = b"cam-left 1\n";

/// What `request`'s setup is kept under: what it was run from that shapes
/// what it leaves, and not what is drawn or posted.
fn key(request: &Request) -> Option<String> {
    let made_of = (
        request.version,
        &request.job,
        &request.shapes,
        &request.from,
    );
    serde_json::to_string(&made_of).ok().map(|t| cache::key(&t))
}

/// Keep in `folder` what `ran`, run from `request`, left.
pub(crate) fn keep(folder: &Path, request: &Request, ran: &Ran) {
    let Some(key) = key(request) else {
        return;
    };
    if let Ok(left) = ran.result.left() {
        cache::put_kind(folder, &key, cache::Kind::Left, &to_bytes(&left));
    }
}

/// What `request`'s setup leaves: kept in the folder `kept_in` from when it
/// ran, or run again for, and then kept there.
pub(crate) fn left_by(request: &Request, kept_in: Option<&Path>) -> Result<Carried, String> {
    speaks(request)?;
    let job = parse_job(&request.job, "the setup it carries on from").map_err(|e| e.to_string())?;
    let name = job
        .setups
        .first()
        .map_or_else(String::new, |s| s.name.clone());
    let kept = kept_in.zip(key(request));
    if let Some((folder, key)) = &kept
        && let Some(bytes) = cache::get_kind(folder, key, cache::Kind::Left)
        && let Some(left) = from_bytes(&bytes)
    {
        return Ok(left);
    }
    let within = |e: String| format!("the setup it carries on from, {name}: {e}");
    let carried = request
        .from
        .as_deref()
        .map(|earlier| left_by(earlier, kept_in))
        .transpose()
        .map_err(within)?;
    let loaded = load(request, &job).map_err(within)?;
    let models = loaded.iter().map(|(n, m)| (n.as_str(), m)).collect();
    let ran = run_setup_from(&job, 0, &models, &[], carried.as_ref(), &mut |_| {})
        .map_err(|e| within(e.to_string()))?;
    let left = ran.result.left().map_err(|e| within(e.to_string()))?;
    if let Some((folder, key)) = &kept {
        cache::put_kind(folder, key, cache::Kind::Left, &to_bytes(&left));
    }
    Ok(left)
}

fn to_bytes(left: &Carried) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    let m = left.to_model;
    let numbers = [left.within.min, left.within.max]
        .into_iter()
        .chain([
            m.matrix3.x_axis,
            m.matrix3.y_axis,
            m.matrix3.z_axis,
            m.translation,
        ])
        .flat_map(|v| v.to_array());
    for v in numbers {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend(PackedStock::pack(&left.stock).to_bytes());
    out
}

fn from_bytes(bytes: &[u8]) -> Option<Carried> {
    let rest = bytes.strip_prefix(MAGIC)?;
    let (numbers, stock) = rest.split_at_checked(18 * 8)?;
    let v: Vec<f64> = numbers
        .as_chunks::<8>()
        .0
        .iter()
        .map(|b| f64::from_le_bytes(*b))
        .collect();
    let at = |i: usize| Some(DVec3::new(*v.get(i)?, *v.get(i + 1)?, *v.get(i + 2)?));
    let stock = PackedStock::from_bytes(stock).ok()?.unpack().ok()?;
    Some(Carried {
        stock: Arc::new(stock),
        within: Aabb {
            min: at(0)?,
            max: at(3)?,
        },
        to_model: DAffine3 {
            matrix3: DMat3::from_cols(at(6)?, at(9)?, at(12)?),
            translation: at(15)?,
        },
    })
}
