//! Setups being run by the helper, and what came of them.
//!
//! A setup runs as one job, which hands it to the native helper; the helper
//! uses every core, so one setup runs at a time and the rest wait their turn.
//! A setup is run again only when what it is run from changed: its job, or its
//! bodies' shapes, or, for one that carries on from an earlier setup, that
//! one's. A setup waits for the one it carries on from, so the helper keeps
//! what that one left for it rather than run it twice.

use std::collections::BTreeMap;
use std::rc::Rc;

use cam_protocol::{Request, Response, Run};
use printcad_bench_sdk::{host, serde_json};

/// The job entry that runs a setup.
pub const JOB: &str = "run";

#[derive(Debug, Clone)]
pub enum State {
    /// Waiting for the setup before it to finish.
    Queued,
    Running(u64),
    /// Shared, so a call that needs the run beside `&mut` access to the
    /// package takes it without copying its toolpaths and programs.
    Ready(Rc<Run>),
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
    pub last: Option<Rc<Run>>,
    /// Its operations' feature ids, in the order it runs them: which of the
    /// run's operations each is. Kept here because a feature's badge is
    /// asked for without the document, where they could be looked up.
    pub operations: Vec<String>,
    /// The setup it carries on from, which runs first.
    after: Option<String>,
}

impl Computed {
    /// The run to show: this one when ready, else the one before.
    pub fn shown(&self) -> Option<&Run> {
        match &self.state {
            State::Ready(run) => Some(run),
            _ => self.last.as_deref(),
        }
    }

    /// [`Self::shown`], shared.
    pub fn shown_rc(&self) -> Option<Rc<Run>> {
        match &self.state {
            State::Ready(run) => Some(run.clone()),
            _ => self.last.clone(),
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
            state: State::Ready(Rc::new(run)),
            last: None,
            operations,
            after: None,
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
    /// request; `again` runs a stopped or failed one again too. `after` is
    /// the setup it carries on from, which it waits for. The request is
    /// written out only to run it: a body's solid can be megabytes, and
    /// whether the setups changed is asked on every edit.
    pub fn want(
        &mut self,
        setup: &str,
        request: Request,
        operations: Vec<String>,
        again: bool,
        after: Option<String>,
    ) {
        let key = key_of(&request);
        if let Some(known) = self.setups.get_mut(setup)
            && known.key == key
            && (!again || !matches!(known.state, State::Stopped | State::Failed(_)))
        {
            known.operations = operations;
            known.after = after;
            return;
        }
        let Ok(request) = serde_json::to_string(&request) else {
            return self.refuse(setup, "the setup's request does not write".into());
        };
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
                after,
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
                after: None,
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
        let Some(c) = self.next().and_then(|id| self.setups.get_mut(&id)) else {
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

    /// The setup to run next: the first waiting whose earlier setup is not
    /// waiting or running itself. One always is, as each comes after the one
    /// it waits for.
    fn next(&self) -> Option<String> {
        let busy = |id: &Option<String>| {
            id.as_ref()
                .and_then(|a| self.setups.get(a))
                .is_some_and(|a| matches!(a.state, State::Queued | State::Running(_)))
        };
        let waiting = || {
            self.setups
                .iter()
                .filter(|(_, c)| matches!(c.state, State::Queued))
        };
        waiting()
            .find(|(_, c)| !busy(&c.after))
            .or_else(|| waiting().next())
            .map(|(id, _)| id.clone())
    }

    /// Job `job` ended with `result`; the setup it ran, if it was one.
    pub fn finished(&mut self, job: u64, result: &Result<String, String>) -> Option<String> {
        let (setup, c) = self
            .setups
            .iter_mut()
            .find(|(_, c)| matches!(c.state, State::Running(j) if j == job))?;
        c.state = match result {
            Ok(json) => match serde_json::from_str::<Response>(json) {
                Ok(Response::Ran(run)) => State::Ready(Rc::from(run)),
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

/// What tells `request` from another without keeping it: its job, each
/// solid and where it sits, its post, and the earlier setup's request it
/// carries on from.
fn key_of(request: &Request) -> u64 {
    let mut h = digest(0xcbf2_9ce4_8422_2325, request.job.as_bytes());
    if let Some(from) = &request.from {
        h = digest(h, &key_of(from).to_le_bytes());
    }
    for (name, shape) in &request.shapes {
        h = digest(h, name.as_bytes());
        h = digest(h, shape.text.as_bytes());
        for v in shape.placement {
            h = digest(h, &v.to_le_bytes());
        }
    }
    h = digest(h, request.post.as_deref().unwrap_or_default().as_bytes());
    digest(
        h,
        &[
            u64::from(request.version).to_le_bytes(),
            (request.max_points as u64).to_le_bytes(),
        ]
        .concat(),
    )
}

/// `h` on with `bytes`, eight at a time (FNV-1a over words, then the
/// length): a solid of megabytes is read through quickly.
fn digest(h: u64, bytes: &[u8]) -> u64 {
    let (words, rest) = bytes.as_chunks::<8>();
    let step = |h: u64, w: u64| (h ^ w).wrapping_mul(0x0100_0000_01b3).rotate_left(29);
    let mut h = words.iter().fold(h, |h, w| step(h, u64::from_le_bytes(*w)));
    let mut tail = [0; 8];
    tail[..rest.len()].copy_from_slice(rest);
    h = step(h, u64::from_le_bytes(tail));
    step(h, bytes.len() as u64)
}

#[cfg(test)]
mod tests {
    use cam_protocol::Response;

    use super::*;

    /// A setup's request is told from another by what it runs: its job,
    /// each solid and where it sits, its post; the same request is the same.
    #[test]
    fn a_request_is_known_by_what_it_runs() {
        let request = Request {
            version: cam_protocol::VERSION,
            job: "[[setup]]\nname = \"top\"\n".into(),
            shapes: BTreeMap::from([(
                "part".to_string(),
                cam_protocol::Shape {
                    text: "a solid, longer than a word or two".into(),
                    placement: cam_protocol::IDENTITY,
                },
            )]),
            max_points: 100,
            post: None,
            read: None,
            from: None,
        };
        let key = key_of(&request);
        assert_eq!(key_of(&request.clone()), key);
        let mut moved = request.clone();
        moved.shapes.get_mut("part").unwrap().placement[3] = 10.0;
        let mut edited = request.clone();
        edited.shapes.get_mut("part").unwrap().text.push('!');
        let mut posted = request.clone();
        posted.post = Some("name = \"mine\"".into());
        let mut rejobbed = request.clone();
        rejobbed.job.push(' ');
        let mut carried = request.clone();
        carried.from = Some(Box::new(request.clone()));
        for other in [moved, edited, posted, rejobbed, carried] {
            assert_ne!(key_of(&other), key, "{other:?}");
        }
    }

    /// A run that comes back keeps the request it was started from, for a
    /// playback session to run again as it was.
    #[test]
    fn a_ready_run_knows_what_it_ran_from() {
        let mut runs = Runs::default();
        runs.setups.insert(
            "s".into(),
            Computed {
                key: 1,
                request: None,
                ran: Some("asked".into()),
                state: State::Running(7),
                last: None,
                operations: Vec::new(),
                after: None,
            },
        );
        assert_eq!(runs.get("s").unwrap().ran_from(), None, "not back yet");
        let answer = serde_json::to_string(&Response::Ran(Box::default())).unwrap();
        assert_eq!(runs.finished(7, &Ok(answer)).as_deref(), Some("s"));
        let c = runs.get("s").unwrap();
        assert_eq!(c.ran_from(), Some("asked"));
        assert_eq!(c.key, 1);
    }

    /// A setup that carries on from another waits for it, whichever comes
    /// first by id.
    #[test]
    fn a_setup_waits_for_the_one_it_carries_on_from() {
        let waiting = |after: Option<&str>| Computed {
            key: 0,
            request: Some("asked".into()),
            ran: None,
            state: State::Queued,
            last: None,
            operations: Vec::new(),
            after: after.map(String::from),
        };
        let mut runs = Runs::default();
        runs.setups
            .insert("a-side-two".into(), waiting(Some("b-side-one")));
        runs.setups.insert("b-side-one".into(), waiting(None));
        assert_eq!(runs.next().as_deref(), Some("b-side-one"));
        runs.setups.get_mut("b-side-one").unwrap().state = State::Stopped;
        assert_eq!(runs.next().as_deref(), Some("a-side-two"));
    }
}
