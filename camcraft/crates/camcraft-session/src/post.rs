//! Posting: a setup's programs named, rendered and written.
//!
//! A [`Posting`] is made first, with every program rendered, then written in one
//! go. It says what stands in the way — setups that did not pass their checks, and
//! files that would be replaced — so a front end can ask before anything is
//! written. Several setups are all posted or none are: half a job's programs are a
//! trap on the machine. The command line's `--out` and the app's Post both come
//! through here, so they write the same files under the same names.

use std::path::{Path, PathBuf};

use camcraft_core::job::Setup;
use camcraft_core::run::{Made, OperationResult, StrategyDetail};
use camcraft_core::setup::frame::Frame;
use camcraft_post::op::{CustomGcode, DrillCycles, LowerError, ProgramSpec, extents, lower};
use camcraft_post::profile::Profile;
use camcraft_post::render::{RenderReport, render};

use crate::{Ran, SessionError, fs};

/// A builtin machine profile by name, or a TOML profile by path. A relative path
/// is found from `job_dir`, when there is one.
pub fn load_profile(name: &str, job_dir: Option<&Path>) -> Result<Profile, SessionError> {
    Ok(match Profile::builtin(name) {
        Some(p) => p?,
        None => match job_dir {
            Some(dir) => Profile::load(fs::resolve(dir, name))?,
            None => Profile::load(name)?,
        },
    })
}

/// The machine profile `setup` is posted with: `profile` for every setup when
/// given, or its own, named from `job_dir`.
pub fn setup_profile(
    setup: &Setup,
    job_dir: &Path,
    profile: Option<&str>,
) -> Result<Profile, SessionError> {
    load_profile(profile.unwrap_or(&setup.post), Some(job_dir)).map_err(|source| {
        SessionError::InSetup {
            setup: setup.name.clone(),
            source: Box::new(source),
        }
    })
}

/// The machine profiles built in, by name.
pub fn builtin_posts() -> impl Iterator<Item = &'static str> {
    Profile::builtin_names()
}

fn file_safe(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// The program name for the `index`-th operation (from zero) of a setup:
/// `top-01-rough`.
///
/// The name is kept usable as a file name.
pub fn program_name(setup: &str, index: usize, op_name: &str) -> String {
    format!(
        "{}-{:02}-{}",
        file_safe(setup),
        index + 1,
        file_safe(op_name)
    )
}

/// Everything one operation's program needs besides its path, for `profile`,
/// in a setup whose frame is `frame`: tilted, where the machine's rotaries go
/// to stand its tool axis up the spindle, which they may not reach.
pub fn program_spec(
    op: &OperationResult,
    wcs: u8,
    frame: &Frame,
    profile: &Profile,
) -> Result<ProgramSpec, LowerError> {
    let [x, _, z] = frame.turn.axes();
    let rotary = match &profile.rotary {
        Some(r) => r.position(z, x)?,
        None => Vec::new(),
    };
    Ok(ProgramSpec {
        op_name: op.name.clone(),
        strategy: op.strategy.to_string(),
        tool: op.tool_number,
        tool_comment: op.tool_comment.clone(),
        wcs,
        rpm: op.rpm,
        spin: op.spindle,
        coolant: op.coolant,
        units: profile.units,
        drill: match &op.detail {
            StrategyDetail::Drill(r) if profile.drill_cycles => Some(DrillCycles {
                holes: r.holes.clone(),
                peck: r.peck,
                chip_break: r.chip_break,
                dwell: r.dwell,
                tap: r.tap,
                feed: r.feed,
                clearance: op.clearance_z,
            }),
            _ => None,
        },
        taps: matches!(&op.detail, StrategyDetail::Drill(r) if r.tap.is_some()),
        // Said in the header when the part is tilted, or turned by the
        // rotaries; a part turned square by hand is written as it always was.
        tilt: (z != glam::DVec3::Z && (frame.turn.square().is_none() || !rotary.is_empty()))
            .then_some(z),
        rotary,
        before: Vec::new(),
        after: Vec::new(),
    })
}

/// For each of a setup's operations, as run (`ops`), the custom G-code
/// written before its own start and after its end. Each Custom G-code
/// operation of `setup` goes before the next operation with a program, or,
/// with none after it, after the last; in a setup with no program at all it
/// is written nowhere.
pub fn custom_gcode(
    setup: &Setup,
    ops: &[OperationResult],
) -> Vec<(Vec<CustomGcode>, Vec<CustomGcode>)> {
    let mut out = vec![(Vec::new(), Vec::new()); ops.len()];
    let mut waiting = Vec::new();
    let mut last = None;
    for (k, op) in ops.iter().enumerate() {
        if op.made == Made::Text {
            let text = setup.operations.get(k).map_or("", |o| o.gcode.as_str());
            waiting.push(CustomGcode::new(&op.name, text));
        } else if op.made.has_paths() {
            if let Some((before, _)) = out.get_mut(k) {
                before.append(&mut waiting);
            }
            last = Some(k);
        }
    }
    if let Some((_, after)) = last.and_then(|k| out.get_mut(k)) {
        after.append(&mut waiting);
    }
    out
}

/// What is wrong with operation `op`'s moves on `profile`'s machine: each
/// axis they take past its `travel`.
pub fn travel_problems(op: &OperationResult, profile: &Profile) -> Vec<String> {
    match extents(&op.path) {
        Some((low, high)) => profile.travel_problems(low, high),
        None => Vec::new(),
    }
}

/// Render one operation, of a setup whose frame is `frame`, as a program for
/// `profile`.
pub fn program(
    op: &OperationResult,
    wcs: u8,
    frame: &Frame,
    profile: &Profile,
    name: &str,
) -> Result<(String, RenderReport), SessionError> {
    program_with(op, &program_spec(op, wcs, frame, profile)?, profile, name)
}

/// Render one operation, with `spec` (from [`program_spec`], and its custom
/// G-code), as a program for `profile`.
pub fn program_with(
    op: &OperationResult,
    spec: &ProgramSpec,
    profile: &Profile,
    name: &str,
) -> Result<(String, RenderReport), SessionError> {
    let ops = lower(&op.path, spec)?;
    Ok(render(&ops, profile, name))
}

/// One program, rendered.
#[derive(Debug, Clone)]
pub struct Program {
    /// Where it goes.
    pub path: PathBuf,
    /// What it says.
    pub text: String,
    /// The operation it cuts.
    pub op: String,
    /// The machine profile it was rendered for.
    pub profile: String,
    /// What rendering it wrote.
    pub report: RenderReport,
}

/// What posting would write, and what stands in the way.
#[derive(Debug, Clone)]
pub struct Posting {
    /// The folder the programs go into.
    pub dir: PathBuf,
    /// Every program, in setup and operation order.
    pub programs: Vec<Program>,
    /// Setups that did not pass their checks, each with what failed.
    pub refused: Vec<(String, Vec<String>)>,
    /// Programs that would replace a file already there.
    pub overwrites: Vec<PathBuf>,
}

impl Posting {
    /// Whether anything must be confirmed before writing.
    pub fn needs_confirming(&self) -> bool {
        !self.refused.is_empty() || !self.overwrites.is_empty()
    }
}

/// Render every program of `setups` (each with its run) into `dir`, writing
/// nothing.
///
/// A setup's `post` names a builtin profile or a file, found from `job_dir`;
/// `profile`, when given, is used for every setup instead.
pub fn plan_posting(
    setups: &[(&Setup, &Ran)],
    job_dir: &Path,
    dir: &Path,
    profile: Option<&str>,
) -> Result<Posting, SessionError> {
    let mut out = Posting {
        dir: dir.to_path_buf(),
        programs: Vec::new(),
        refused: Vec::new(),
        overwrites: Vec::new(),
    };
    // Each program's file, by what a file system that ignores case sees, and
    // whose it is: two names can come to one file.
    let mut taken: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (setup, ran) in setups {
        let in_setup = |source| SessionError::InSetup {
            setup: setup.name.clone(),
            source: Box::new(source),
        };
        let loaded = setup_profile(setup, job_dir, profile)?;
        // Past the machine's travel, a program stops at its limits part-way
        // through, or crashes into them.
        let past: Vec<String> = ran
            .result
            .operations
            .iter()
            .flat_map(|op| {
                travel_problems(op, &loaded)
                    .into_iter()
                    .map(|why| format!("travel {}: {why}", op.name))
            })
            .collect();
        if !ran.passed() || !past.is_empty() {
            let mut why = ran.checks.failure_lines();
            if why.is_empty() && !ran.passed() {
                why.push("the simulator did not verify it".into());
            }
            why.extend(past);
            out.refused.push((setup.name.clone(), why));
        }
        let custom = custom_gcode(setup, &ran.result.operations);
        // An operation turned off or left out of the run has no program.
        let made = ran.result.operations.iter().enumerate();
        for (k, op) in made.filter(|(_, op)| op.made.has_paths()) {
            let name = program_name(&setup.name, k, &op.name);
            let mut spec = program_spec(op, setup.wcs, &ran.result.frame, &loaded)
                .map_err(|e| in_setup(e.into()))?;
            if let Some((before, after)) = custom.get(k) {
                spec.before.clone_from(before);
                spec.after.clone_from(after);
            }
            let (text, report) = program_with(op, &spec, &loaded, &name).map_err(in_setup)?;
            let path = dir.join(format!("{name}.{}", loaded.extension));
            let whose = format!("setup '{}' operation '{}'", setup.name, op.name);
            let key = path.display().to_string().to_lowercase();
            if let Some(first) = taken.insert(key, whose.clone()) {
                return Err(SessionError::SameProgram {
                    path: path.display().to_string(),
                    first,
                    second: whose,
                });
            }
            if path.exists() {
                out.overwrites.push(path.clone());
            }
            out.programs.push(Program {
                path,
                text,
                op: op.name.clone(),
                profile: loaded.name.clone(),
                report,
            });
        }
    }
    Ok(out)
}

/// Write every program in `posting`: each beside its place first, and only once
/// all are written is each moved into it, so a failure writing any of them
/// leaves every program as it was, not some of this posting's and some of the
/// last's. A move that fails part-way through the moves can still leave some
/// of each. Returns how many were written.
pub fn write_posting(posting: &Posting) -> Result<usize, SessionError> {
    let failed = |path: &Path| {
        let path = path.display().to_string();
        move |source| SessionError::ProgramWrite { path, source }
    };
    std::fs::create_dir_all(&posting.dir).map_err(failed(&posting.dir))?;
    let temp = |path: &Path| {
        let mut t = path.as_os_str().to_owned();
        t.push(".camcraft-posting");
        std::path::PathBuf::from(t)
    };
    let clean = || {
        for p in &posting.programs {
            let _ = std::fs::remove_file(temp(&p.path));
        }
    };
    for p in &posting.programs {
        if let Err(e) = std::fs::write(temp(&p.path), &p.text) {
            clean();
            return Err(failed(&p.path)(e));
        }
    }
    for p in &posting.programs {
        if let Err(e) = std::fs::rename(temp(&p.path), &p.path) {
            clean();
            return Err(failed(&p.path)(e));
        }
    }
    Ok(posting.programs.len())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests;
