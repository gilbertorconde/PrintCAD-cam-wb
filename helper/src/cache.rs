//! The runs this helper answered, kept on disk by what was asked, so a setup
//! asked again as it was (a document opened again, an edit undone) is
//! answered at once.
//!
//! A package keeps nothing between sessions but its document and its
//! settings, and a run is too big for either. The helper is native, so it
//! keeps them in the user's cache folder, `printcad-cam`, the newest
//! [`KEPT`] of them. `CAM_HELPER_CACHE` names another folder, or `off`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How many runs are kept.
pub const KEPT: usize = 64;

/// The folder runs are kept in, if any.
pub fn folder() -> Option<PathBuf> {
    match std::env::var_os("CAM_HELPER_CACHE") {
        Some(v) if v == "off" => None,
        Some(v) => Some(PathBuf::from(v)),
        None => user_cache().map(|c| c.join("printcad-cam")),
    }
}

/// The user's cache folder, as each system has it.
fn user_cache() -> Option<PathBuf> {
    let var = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if cfg!(windows) {
        var("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library/Caches"))
    } else {
        var("XDG_CACHE_HOME").or_else(|| var("HOME").map(|h| h.join(".cache")))
    }
}

/// The name a request is kept under: a digest of its text and of which
/// helper answers it, so a helper built again never answers with an old
/// build's run, whose answer may lack what the package now reads.
pub fn key(request: &str) -> String {
    key_for(request, &build())
}

/// Which helper this is: its version, and its own file's size and time,
/// which change with every build.
fn build() -> String {
    let file = std::env::current_exe()
        .ok()
        .and_then(|p| fs::metadata(p).ok())
        .map(|m| {
            let at = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos());
            format!("{}:{at}", m.len())
        })
        .unwrap_or_default();
    format!("{}:{file}", env!("CARGO_PKG_VERSION"))
}

fn key_for(request: &str, build: &str) -> String {
    // FNV-1a, 64 bits, twice over different seeds: no dependency, stable
    // across systems, and wide enough for a few dozen runs.
    let fnv = |seed: u64| {
        let mut h = seed;
        for b in build.bytes().chain([0]).chain(request.bytes()) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        h
    };
    format!(
        "{:016x}{:016x}-{}",
        fnv(0xcbf2_9ce4_8422_2325),
        fnv(0x6c62_272e_07bb_0142),
        request.len()
    )
}

fn path(folder: &Path, key: &str) -> PathBuf {
    folder.join(format!("{key}.json"))
}

/// The answer kept for `key`, if one is; it becomes the newest.
pub fn get(folder: &Path, key: &str) -> Option<Vec<u8>> {
    let path = path(folder, key);
    let bytes = fs::read(&path).ok()?;
    if let Ok(f) = fs::File::options().append(true).open(&path) {
        let _ = f.set_modified(SystemTime::now());
    }
    Some(bytes)
}

/// Keep `answer` for `key`, and drop all but the newest [`KEPT`]. A folder
/// that cannot be written is left alone: the run was answered anyway.
pub fn put(folder: &Path, key: &str, answer: &[u8]) {
    if fs::create_dir_all(folder).is_err() {
        return;
    }
    // Written aside and moved in, so a run read while it is written is
    // whole or not there.
    let path = path(folder, key);
    let part = path.with_extension("part");
    if fs::write(&part, answer).is_err() || fs::rename(&part, &path).is_err() {
        let _ = fs::remove_file(&part);
        return;
    }
    prune(folder, KEPT);
}

/// Drop all but the newest `keep` kept answers.
fn prune(folder: &Path, keep: usize) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    let mut kept: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| Some((fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    kept.sort_by_key(|k| std::cmp::Reverse(k.0));
    for (_, p) in kept.into_iter().skip(keep) {
        let _ = fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_is_kept_by_what_was_asked() {
        let folder = std::env::temp_dir().join(format!("cam-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        let (a, b) = (key("{\"job\":\"a\"}"), key("{\"job\":\"b\"}"));
        assert_ne!(a, b);
        // Another build of the helper asks again.
        assert_ne!(key_for("x", "0.2.0:1:2"), key_for("x", "0.2.0:1:3"));
        assert_eq!(a, key("{\"job\":\"a\"}"));
        assert_eq!(get(&folder, &a), None);
        put(&folder, &a, b"ran a");
        assert_eq!(get(&folder, &a).as_deref(), Some(&b"ran a"[..]));
        assert_eq!(get(&folder, &b), None);
        // Only the newest are kept.
        for i in 0..5 {
            put(&folder, &key(&i.to_string()), b"x");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        prune(&folder, 3);
        let left = fs::read_dir(&folder).unwrap().count();
        assert_eq!(left, 3);
        assert!(get(&folder, &key("4")).is_some());
        assert_eq!(get(&folder, &a), None, "the oldest went");
        let _ = fs::remove_dir_all(&folder);
    }
}
