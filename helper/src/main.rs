//! The CAM package's native helper: a [`Request`] on standard input, a
//! [`Response`] on standard output (see the library for what happens between).

use std::io::{Read, Write};

use cam_protocol::{Request, Response};
use camcraft_helper::answer;

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
    let response = match answer(&request) {
        Ok(run) => Response::Ran(Box::new(run)),
        Err(why) => Response::Refused { why },
    };
    let mut out = std::io::stdout().lock();
    match serde_json::to_writer(&mut out, &response) {
        Ok(()) if out.flush().is_ok() => std::process::ExitCode::SUCCESS,
        _ => {
            eprintln!("cannot write the answer");
            std::process::ExitCode::FAILURE
        }
    }
}
