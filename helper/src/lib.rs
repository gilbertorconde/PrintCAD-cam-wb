//! The CAM package's native helper: what it does, apart from reading and writing
//! (`main.rs`).
//!
//! printCAD runs it from a job of the package with a [`Request`] on its standard
//! input, and kills it when the job is stopped. It reads each model's solid from
//! ogeom's native format, runs the setup through camcraft's one engine path
//! (`camcraft_session::run_setup`, which the camcraft app and command line run
//! too) on every core, and writes a [`Response`] on its standard output: the
//! toolpaths for drawing, the checks, and a program for each operation.
//!
//! A setup that cannot run is answered with [`Response::Refused`] and a reason a
//! person can read; only a broken request or a crash ends it with a failure.

use std::sync::Arc;
use std::time::Instant;

use cam_protocol::{
    Finding, IDENTITY, Level, Material, OperationRun, Program, Request, Run, Stroke, VERSION,
};
use camcraft_core::path::toolpath::{MoveRole, Toolpath};
use camcraft_core::run::Made;
use camcraft_core::setup::frame::Frame;
use camcraft_geom::mesh::Aabb;
use camcraft_import::native::load_native;
use camcraft_session::checks::Level as CheckLevel;
use camcraft_session::{Model, Models, Ran, load_profile, parse_job, program, run_setup};
use glam::{DAffine3, DMat4, DVec2, DVec3};

/// How many columns a side the drawn material has, at most.
const MATERIAL_COLUMNS: usize = 100;

/// Run `request`'s setup, or say why it cannot run.
pub fn answer(request: &Request) -> Result<Run, String> {
    if request.version != VERSION {
        return Err(format!(
            "the package speaks version {} to its helper, which speaks {VERSION}: \
             reinstall the package",
            request.version
        ));
    }
    let started = Instant::now();
    let job = parse_job(&request.job, "the setup").map_err(|e| e.to_string())?;
    let setup = job.setups.first().ok_or("the job has no setup")?;

    // A STEP file is tessellated to a quarter of the machining tolerance; a
    // native shape is the same B-rep, so it gets the same.
    let tessellation = job.tolerance / 4.0;
    let mut loaded = Vec::new();
    for def in &job.models {
        let shape = request
            .shapes
            .get(&def.name)
            .ok_or_else(|| format!("no shape came for '{}'", def.name))?;
        let (body, report) =
            load_native(&shape.text, tessellation).map_err(|e| format!("{}: {e}", def.name))?;
        let body = if shape.placement == IDENTITY {
            body
        } else {
            body.placed(affine(&shape.placement))
                .map_err(|e| format!("{}: {e}", def.name))?
        };
        let bounds = body
            .mesh()
            .bounds()
            .ok_or_else(|| format!("{} is empty", def.name))?;
        let mut warnings = Vec::new();
        if report.degenerate > 0 {
            warnings.push(format!(
                "{} degenerate triangle(s) from tessellation",
                report.degenerate
            ));
        }
        loaded.push((
            def.name.as_str(),
            Model {
                file: Arc::new(body.clone()),
                body,
                bounds,
                path: def.name.clone(),
                units: "mm".into(),
                notes: vec![format!(
                    "{}: {} face(s), {} triangles at {} mm",
                    def.name, report.faces, report.triangles, tessellation
                )],
                warnings,
            },
        ));
    }
    let models: Models<'_> = loaded.iter().map(|(n, m)| (*n, m)).collect();
    let ran = run_setup(&job, 0, &models).map_err(|e| e.to_string())?;
    let profile = load_profile(&setup.post, None).map_err(|e| e.to_string())?;

    let frame = ran.result.frame;
    let tolerance = job.tolerance.max(0.02);
    let mut operations = Vec::new();
    for (k, op) in ran.result.operations.iter().enumerate() {
        let made = match op.made {
            Made::Generated => "generated",
            Made::Kept => "kept",
            Made::Skipped => "skipped",
            Made::Disabled => "disabled",
        };
        let (strokes, points, cut_mm, rapid_mm) =
            strokes(&op.path, &frame, tolerance, request.max_points);
        let program = if op.made.has_paths() {
            let name = camcraft_session::program_name(&setup.name, k, &op.name);
            let (text, report) =
                program(op, setup.wcs, &profile, &name).map_err(|e| e.to_string())?;
            Some(Program {
                name,
                extension: profile.extension.clone(),
                post: profile.name.clone(),
                lines: usize::try_from(report.lines).unwrap_or(usize::MAX),
                text,
            })
        } else {
            None
        };
        operations.push(OperationRun {
            name: op.name.clone(),
            made: made.into(),
            summary: format!(
                "{} passes, {} links stayed down, removed {:.0} mm³",
                op.passes, op.links.stayed_down, op.sim.removed_mm3
            ),
            warnings: op.warnings.clone(),
            strokes,
            points,
            cut_mm,
            rapid_mm,
            program,
        });
    }

    let stock = Aabb::union_all(&ran.result.stocks)
        .map(|b| corners(&frame, b))
        .unwrap_or([[0.0; 3]; 2]);
    Ok(Run {
        version: VERSION,
        passed: ran.passed(),
        findings: findings(&ran),
        notes: loaded
            .iter()
            .flat_map(|(_, m)| m.notes.iter().cloned())
            .chain(ran.notes.iter().cloned())
            .collect(),
        warnings: ran.warnings.clone(),
        stock,
        origin: to_array(frame.to_model(DVec3::ZERO)),
        operations,
        material: material(&ran),
        seconds: started.elapsed().as_secs_f64(),
    })
}

/// The checks' findings, failures first, then warnings, notes and passes.
fn findings(ran: &Ran) -> Vec<Finding> {
    let mut out: Vec<Finding> = ran
        .checks
        .groups
        .iter()
        .flat_map(|group| {
            group.findings.iter().map(move |f| Finding {
                level: match f.level {
                    CheckLevel::Fail => Level::Fail,
                    CheckLevel::Warn => Level::Warn,
                    CheckLevel::Note => Level::Note,
                    CheckLevel::Pass => Level::Pass,
                },
                code: f.code.to_string(),
                text: f.line(group),
                operation: group.op,
            })
        })
        .collect();
    let rank = |l: Level| match l {
        Level::Fail => 0,
        Level::Warn => 1,
        Level::Note => 2,
        Level::Pass => 3,
    };
    out.sort_by_key(|f| rank(f.level));
    out
}

/// A box in the setup's frame as low and high corners in the model's.
fn corners(frame: &Frame, b: Aabb) -> [[f64; 3]; 2] {
    let (a, z) = (frame.to_model(b.min), frame.to_model(b.max));
    [to_array(a.min(z)), to_array(a.max(z))]
}

/// A row-major rigid 4x4 matrix as an affine map.
fn affine(rows: &[f64; 16]) -> DAffine3 {
    DAffine3::from_mat4(DMat4::from_cols_array(rows).transpose())
}

fn to_array(p: DVec3) -> [f64; 3] {
    [p.x, p.y, p.z]
}

fn to_f32(p: DVec3) -> [f32; 3] {
    [p.x as f32, p.y as f32, p.z as f32]
}

/// `path` as strokes in the model's coordinates, thinned to `budget` points,
/// with how many points it has, how far it cuts and how far it rapids.
fn strokes(
    path: &Toolpath,
    frame: &Frame,
    tolerance: f64,
    budget: usize,
) -> (Vec<Stroke>, usize, f64, f64) {
    let mut out: Vec<Stroke> = Vec::new();
    let (mut cut, mut rapid) = (0.0, 0.0);
    let mut at: Option<DVec3> = None;
    for m in path.moves() {
        let fast = m.role == MoveRole::Rapid;
        let Some(from) = at else {
            at = Some(m.to);
            out.push(Stroke {
                rapid: fast,
                points: vec![to_f32(frame.to_model(m.to))],
            });
            continue;
        };
        let length = m.length(from);
        if fast {
            rapid += length;
        } else {
            cut += length;
        }
        if out.last().is_none_or(|s| s.rapid != fast) {
            out.push(Stroke {
                rapid: fast,
                points: vec![to_f32(frame.to_model(from))],
            });
        }
        if let Some(stroke) = out.last_mut() {
            stroke.points.extend(
                m.chords(from, tolerance)
                    .into_iter()
                    .map(|p| to_f32(frame.to_model(p))),
            );
        }
        at = Some(m.to);
    }
    let points: usize = out.iter().map(|s| s.points.len()).sum();
    if budget == 0 {
        return (Vec::new(), points, cut, rapid);
    }
    if points > budget {
        let every = points.div_ceil(budget);
        for stroke in &mut out {
            let last = stroke.points.len().saturating_sub(1);
            let kept = stroke
                .points
                .iter()
                .enumerate()
                .filter(|(i, _)| i % every == 0 || *i == last)
                .map(|(_, p)| *p)
                .collect();
            stroke.points = kept;
        }
    }
    (out, points, cut, rapid)
}

/// The finished material's top over the stock, as triangles in the model's
/// coordinates.
fn material(ran: &Ran) -> Option<Material> {
    let r = &ran.result;
    let stock = r.keyframes.last()?.unpack().ok()?;
    let (lo, hi) = (r.stock_min, r.stock_max);
    let span = hi - lo;
    if !(span.x > 0.0 && span.y > 0.0) {
        return None;
    }
    let pitch = span.x.max(span.y) / MATERIAL_COLUMNS as f64;
    let nx = ((span.x / pitch).ceil() as usize).max(1) + 1;
    let ny = ((span.y / pitch).ceil() as usize).max(1) + 1;
    let step = DVec2::new(span.x / (nx - 1) as f64, span.y / (ny - 1) as f64);
    let floor = stock.floor();
    let mut positions = Vec::with_capacity(nx * ny);
    for j in 0..ny {
        for i in 0..nx {
            let p = lo + step * DVec2::new(i as f64, j as f64);
            let z = stock.height_at(p.x, p.y).unwrap_or(floor);
            positions.push(to_f32(r.frame.to_model(p.extend(z))));
        }
    }
    let mut indices = Vec::with_capacity((nx - 1) * (ny - 1) * 6);
    for j in 0..ny - 1 {
        for i in 0..nx - 1 {
            let a = (j * nx + i) as u32;
            let b = a + 1;
            let c = a + nx as u32;
            let d = c + 1;
            indices.extend([a, b, d, a, d, c]);
        }
    }
    Some(Material { positions, indices })
}

#[cfg(test)]
mod tests;
