//! The CAM package's native helper: a [`Request`] on standard input, a
//! [`Response`] on standard output (see the library for what happens between).
//!
//! While a setup runs it writes `progress <done> <total>` lines to standard
//! error, for a host that reads them. A run it has answered before, asked as
//! it was, comes from its cache ([`camcraft_helper::cache`]).

use std::io::{Read, Write};

use cam_protocol::{Request, Response};
use camcraft_helper::{answer_reporting, cache, read};

fn main() -> std::process::ExitCode {
    let mut text = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut text) {
        eprintln!("cannot read the request: {e}");
        return std::process::ExitCode::FAILURE;
    }
    // CAM_HELPER_DUMP=<file> keeps the last request, to run it again by hand:
    // `camcraft-helper < <file>`.
    if let Some(path) = std::env::var_os("CAM_HELPER_DUMP") {
        let _ = std::fs::write(path, &text);
    }
    let request: Request = match serde_json::from_str(&text) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("the request does not read: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let kept = request
        .read
        .is_none()
        .then(cache::folder)
        .flatten()
        .map(|folder| (folder, cache::key(&text)));
    if let Some((folder, key)) = &kept
        && let Some(answer) = cache::get(folder, key)
    {
        return write(&answer);
    }
    let response = match &request.read {
        Some(path) => match read(&request, path) {
            Ok(text) => Response::Read { text },
            Err(why) => Response::Refused { why },
        },
        None => {
            let mut errors = std::io::stderr();
            let mut progress = |done: u64, total: u64| {
                let _ = writeln!(errors, "progress {done} {total}");
                let _ = errors.flush();
            };
            match answer_reporting(&request, &mut progress) {
                Ok(run) => Response::Ran(Box::new(run)),
                Err(why) => Response::Refused { why },
            }
        }
    };
    let Ok(answer) = serde_json::to_vec(&response) else {
        eprintln!("cannot write the answer");
        return std::process::ExitCode::FAILURE;
    };
    // Only a run is kept: a refusal may be put right outside the request (a
    // helper reinstalled, a file written).
    if let (Some((folder, key)), Response::Ran(_)) = (&kept, &response) {
        cache::put(folder, key, &answer);
    }
    write(&answer)
}

fn write(answer: &[u8]) -> std::process::ExitCode {
    let mut out = std::io::stdout().lock();
    if out.write_all(answer).is_ok() && out.flush().is_ok() {
        std::process::ExitCode::SUCCESS
    } else {
        eprintln!("cannot write the answer");
        std::process::ExitCode::FAILURE
    }
}
