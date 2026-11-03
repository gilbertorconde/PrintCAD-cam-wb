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
    /// The request the run under way, or ready, was started from: what a
    /// playback session runs again, so it plays what is shown even when
    /// the document has moved on unseen (an edit in the Data panel, an
    /// undo).
    ran: Option<String>,
    pub state: State,
    /// What it had last time it ran, shown while it runs again.
    pub last: Option<Box<Run>>,
    /// Its operations' feature ids, in the order it runs them: which of the
    /// run's operations each is. Kept here because a feature's badge is
    /// asked for without the document, where they could be looked up.
    pub operations: Vec<String>,
}

impl Computed {
    /// The run to show: this one when ready, else the one before.
    pub fn shown(&self) -> Option<&Run> {
        match &self.state {
            State::Ready(run) => Some(run),
            _ => self.last.as_deref(),
        }
    }

    /// The request a ready run came from.
    pub fn ran_from(&self) -> Option<&str> {
        match self.state {
            State::Ready(_) => self.ran.as_deref(),
            _ => None,
        }
    }

    /// Whether what is shown is not for the setup as it now is.
    pub fn stale(&self) -> bool {
        !matches!(self.state, State::Ready(_))
    }

    /// A run of `operations` that is ready, as one comes back.
    #[cfg(test)]
    pub fn ready(run: Run, operations: Vec<String>) -> Self {
        Self {
            key: 0,
            request: None,
            ran: None,
            state: State::Ready(Box::new(run)),
            last: None,
            operations,
        }
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

    /// Run `setup` from `request`, which runs `operations` (their feature
    /// ids, in order), unless it has a run, or one coming, from the same
    /// request; `again` runs a stopped or failed one again too.
    pub fn want(&mut self, setup: &str, request: String, operations: Vec<String>, again: bool) {
        let key = digest(&request);
        if let Some(known) = self.setups.get_mut(setup)
            && known.key == key
            && (!again || !matches!(known.state, State::Stopped | State::Failed(_)))
        {
            known.operations = operations;
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
                ran: None,
                state: State::Queued,
                last,
                operations,
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
                ran: None,
                state: State::Failed(why),
                last,
                operations: Vec::new(),
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
        c.ran = Some(request);
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
                Ok(Response::Read { .. }) => State::Failed("the helper read a file instead".into()),
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

#[cfg(test)]
mod tests {
    use cam_protocol::Response;

    use super::*;

    /// A run that comes back keeps the request it was started from, for a
    /// playback session to run again as it was.
    #[test]
    fn a_ready_run_knows_what_it_ran_from() {
        let mut runs = Runs::default();
        runs.setups.insert(
            "s".into(),
            Computed {
                key: digest("asked"),
                request: None,
                ran: Some("asked".into()),
                state: State::Running(7),
                last: None,
                operations: Vec::new(),
            },
        );
        assert_eq!(runs.get("s").unwrap().ran_from(), None, "not back yet");
        let answer = serde_json::to_string(&Response::Ran(Box::default())).unwrap();
        assert_eq!(runs.finished(7, &Ok(answer)).as_deref(), Some("s"));
        let c = runs.get("s").unwrap();
        assert_eq!(c.ran_from(), Some("asked"));
        assert_eq!(digest(c.ran_from().unwrap()), c.key);
    }
}
