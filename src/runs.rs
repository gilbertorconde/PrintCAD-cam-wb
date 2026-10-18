//! Setups being run by the helper, and what came of them.
//!
//! A setup runs as one job, which hands it to the native helper; the helper
//! uses every core, so one setup runs at a time and the rest wait their turn.
//! A setup is run again only when what it is run from changed: its job, or its
//! bodies' shapes.

use std::collections::BTreeMap;

use cam_protocol::{Response, Run};
use printcad_bench_sdk::{host, serde_json};

/// The job entry that runs a setup.
pub const JOB: &str = "run";

#[derive(Debug, Clone)]
pub enum State {
    /// Waiting for the setup before it to finish.
    Queued,
    Running(u64),
    Ready(Box<Run>),
    Stopped,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct Computed {
    /// What it is run from: the request, or a digest of it.
    pub key: u64,
    /// The request, while it waits.
    request: Option<String>,
    pub state: State,
    /// What it had last time it ran, shown while it runs again.
    pub last: Option<Box<Run>>,
}

impl Computed {
    /// The run to show: this one when ready, else the one before.
    pub fn shown(&self) -> Option<&Run> {
        match &self.state {
            State::Ready(run) => Some(run),
            _ => self.last.as_deref(),
        }
    }

    /// Whether what is shown is not for the setup as it now is.
    pub fn stale(&self) -> bool {
        !matches!(self.state, State::Ready(_))
    }
}

#[derive(Debug, Default)]
pub struct Runs {
    setups: BTreeMap<String, Computed>,
}

impl Runs {
    pub fn get(&self, setup: &str) -> Option<&Computed> {
        self.setups.get(setup)
    }

    /// Run `setup` from `request` unless it has a run, or one coming, from the
    /// same request; `again` runs a stopped or failed one again too.
    pub fn want(&mut self, setup: &str, request: String, again: bool) {
        let key = digest(&request);
        if let Some(known) = self.setups.get(setup)
            && known.key == key
            && (!again || !matches!(known.state, State::Stopped | State::Failed(_)))
        {
            return;
        }
        self.stop(setup);
        let last = self.setups.get(setup).and_then(|c| match &c.state {
            State::Ready(run) => Some(run.clone()),
            _ => c.last.clone(),
        });
        self.setups.insert(
            setup.to_string(),
            Computed {
                key,
                request: Some(request),
                state: State::Queued,
                last,
            },
        );
        self.pump();
    }

    /// Say why `setup` cannot run, in place of a run.
    pub fn refuse(&mut self, setup: &str, why: String) {
        self.stop(setup);
        let last = self.setups.get(setup).and_then(|c| c.last.clone());
        self.setups.insert(
            setup.to_string(),
            Computed {
                key: 0,
                request: None,
                state: State::Failed(why),
                last,
            },
        );
        self.pump();
    }

    /// Stop `setup`'s run, or take it out of the queue.
    pub fn stop(&mut self, setup: &str) {
        if let Some(c) = self.setups.get_mut(setup) {
            match c.state {
                State::Running(job) => {
                    host::cancel_job(job);
                    c.state = State::Stopped;
                }
                State::Queued => c.state = State::Stopped,
                _ => {}
            }
            c.request = None;
        }
    }

    pub fn forget(&mut self, setup: &str) {
        self.stop(setup);
        self.setups.remove(setup);
        self.pump();
    }

    /// Start the next queued setup when none is running.
    fn pump(&mut self) {
        if self
            .setups
            .values()
            .any(|c| matches!(c.state, State::Running(_)))
        {
            return;
        }
        let Some(c) = self
            .setups
            .values_mut()
            .find(|c| matches!(c.state, State::Queued))
        else {
            return;
        };
        let request = c.request.take().unwrap_or_default();
        c.state = match host::start_job(JOB, &request) {
            Ok(job) => State::Running(job),
            Err(e) => State::Failed(e),
        };
        if matches!(c.state, State::Failed(_)) {
            self.pump();
        }
    }

    /// Job `job` ended with `result`; the setup it ran, if it was one.
    pub fn finished(&mut self, job: u64, result: &Result<String, String>) -> Option<String> {
        let (setup, c) = self
            .setups
            .iter_mut()
            .find(|(_, c)| matches!(c.state, State::Running(j) if j == job))?;
        c.state = match result {
            Ok(json) => match serde_json::from_str::<Response>(json) {
                Ok(Response::Ran(run)) => State::Ready(run),
                Ok(Response::Refused { why }) => State::Failed(why),
                Err(e) => State::Failed(format!("the helper's answer does not read: {e}")),
            },
            Err(e) if e == "stopped" => State::Stopped,
            Err(e) => State::Failed(e.clone()),
        };
        let setup = setup.clone();
        self.pump();
        Some(setup)
    }
}

/// FNV-1a, to tell one request from another without keeping it.
fn digest(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}
