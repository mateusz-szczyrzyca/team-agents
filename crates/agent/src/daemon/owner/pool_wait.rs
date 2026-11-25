//! How long a request waits in the queue for a worker of the pool: the daemon notes since when each queued request
//! waits and, when `[workers] wait_seconds` has passed, fails the request with the words that tell its requester to do
//! the step itself.

use std::collections::HashSet;
use std::path::PathBuf;

use agent_core::decide::worker_wait::{WaitVerdict, wait_verdict};
use agent_core::log::LogEvent;
use agent_core::state::Timestamp;
use agent_runtime::agentbus::{DelegationStatus, DelegationUpdate, HelperEntry, RequestId};
use agent_runtime::process::error_chain;
use agent_runtime::worker_wait;

use super::Owner;
use crate::daemon::team::worker_wait_returned;

/// The seconds of a minute, as the text tells the wait in whole minutes.
const MINUTE: u64 = 60;

/// What the text tells of a request without a step of the plan.
const NO_STEP: &str = "no step";

impl Owner {
    /// Forgets the note of the wait of a request that is no longer queued.
    pub(super) async fn forget_wait(&mut self, request: &HelperEntry) {
        let task_dir = self.parts.store.home().task_dir(&request.from_task);
        self.clear_wait(request.id, task_dir).await;
    }

    /// Takes away the notes of requests that are not queued: those the daemon holds (the bus lists only open requests, so
    /// a request that was failed or settled while it waited is gone from its list) and, at the first look after a start
    /// of the daemon, every note on disk, which a request that ended while the daemon was down left behind.
    pub(super) async fn forget_left_waits(&mut self, open: &[HelperEntry]) {
        let queued: HashSet<RequestId> = open
            .iter()
            .filter(|request| request.status == DelegationStatus::Queued)
            .map(|request| request.id)
            .collect();
        let left: Vec<(RequestId, PathBuf)> = self
            .pool_dispatch
            .waits_noted
            .iter()
            .filter(|(id, _)| !queued.contains(*id))
            .map(|(id, task)| (*id, self.parts.store.home().task_dir(task)))
            .collect();
        for (id, task_dir) in left {
            self.clear_wait(id, task_dir).await;
        }
        if !self.pool_dispatch.waits_swept {
            self.sweep_waits(&queued).await;
        }
    }

    /// Takes away every note on disk whose request is not in `queued`, and remembers that it has done so.
    async fn sweep_waits(&mut self, queued: &HashSet<RequestId>) {
        let tasks_dir = self.parts.store.home().tasks_dir();
        match worker_wait::find_notes(&tasks_dir).await {
            Ok(found) => {
                let mut all_removed = true;
                for note in found.into_iter().filter(|note| !queued.contains(&note.request)) {
                    all_removed &= self.clear_wait(note.request, note.task_dir).await;
                }
                // A note that could not be removed is not held in `waits_noted`: only another sweep finds it again.
                self.pool_dispatch.waits_swept = all_removed;
            }
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::WorkerRequestDamaged.as_str(),
                    error = %error_chain(&error),
                    "the notes of waits left on disk cannot be looked for: they are looked for again at the next pass"
                );
            }
        }
    }

    /// Removes the note of the wait of `request` from `task_dir` and stops holding it; `false` when the note could not be
    /// removed.
    async fn clear_wait(&mut self, request: RequestId, task_dir: PathBuf) -> bool {
        match worker_wait::clear(&task_dir, request).await {
            Ok(()) => {
                self.pool_dispatch.waits_noted.remove(&request);
                true
            }
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::WorkerRequestDamaged.as_str(),
                    request = %request,
                    error = %error_chain(&error),
                    "the note of the wait of a request cannot be removed"
                );
                false
            }
        }
    }

    /// Notes since when each queued request of `open` that no worker of its team serves waits, at the first look at it, and
    /// returns those that have waited `[workers] wait_seconds` to their requesters; the requests returned.
    ///
    /// This is a pass of its own, before a request is dispatched: what keeps a run from starting (the disk, the slots)
    /// does not stop the clock of any request.
    pub(super) async fn return_requests_nobody_took(&mut self, open: &[HelperEntry]) -> HashSet<RequestId> {
        let mut returned = HashSet::new();
        for request in open {
            if request.status == DelegationStatus::Queued
                && !self.requester_has_workers(&request.from_task)
                && self.wait_returns(request).await
            {
                returned.insert(request.id);
            }
        }
        returned
    }

    /// Notes since when the queued `request` waits, at the first look at it, and returns it to its requester when it has
    /// waited `[workers] wait_seconds`; `true` when it was returned and is not to be served.
    async fn wait_returns(&mut self, request: &HelperEntry) -> bool {
        let now = match Timestamp::now(&self.parts.clock) {
            Ok(now) => now,
            Err(error) => {
                tracing::warn!(error = %error_chain(&error), "the clock gives no time: the wait of a request is not counted");
                return false;
            }
        };
        let Some(since) = self.noted_since(request, now).await else {
            return false;
        };
        let limit = self.parts.settings.config.workers().wait_seconds();
        match wait_verdict(since, now, limit) {
            WaitVerdict::Waits { .. } => false,
            WaitVerdict::Return { seconds } => self.return_request(request, seconds).await,
        }
    }

    /// Since when `request` waits: its note, or `now` when it has none, which the first look at it makes; `None` when the
    /// note cannot be read or made.
    async fn noted_since(&mut self, request: &HelperEntry, now: Timestamp) -> Option<Timestamp> {
        let task_dir = self.parts.store.home().task_dir(&request.from_task);
        let noted = match worker_wait::read_since(&task_dir, request.id).await {
            Ok(noted) => noted,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::WorkerRequestDamaged.as_str(),
                    request = %request.id,
                    error = %error_chain(&error),
                    "the note of the wait of a request cannot be read: its wait is not counted"
                );
                return None;
            }
        };
        if noted.is_some() {
            self.pool_dispatch
                .waits_noted
                .insert(request.id, request.from_task.clone());
            return noted;
        }
        if let Err(error) = worker_wait::write_since(&task_dir, request.id, now).await {
            tracing::warn!(
                event = LogEvent::WorkerRequestDamaged.as_str(),
                request = %request.id,
                error = %error_chain(&error),
                "the wait of a request cannot be noted: it is not counted"
            );
            return None;
        }
        self.pool_dispatch
            .waits_noted
            .insert(request.id, request.from_task.clone());
        Some(now)
    }

    /// Fails `request`, which waited `seconds`, with the words that tell its requester to do the step itself; `true` when
    /// the bus took it.
    async fn return_request(&mut self, request: &HelperEntry, seconds: u64) -> bool {
        let step = request
            .step
            .as_deref()
            .map_or_else(|| NO_STEP.to_owned(), |step| format!("step {step}"));
        let summary = match worker_wait_returned(&self.parts.team_texts, request.id, &step, seconds / MINUTE) {
            Ok(summary) => summary,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::WorkerRequestDamaged.as_str(),
                    request = %request.id,
                    error = %error_chain(&error),
                    "the words that return a request nobody took cannot be made: the request waits on"
                );
                return false;
            }
        };
        let update = DelegationUpdate {
            status: Some(DelegationStatus::Failed),
            task: None,
            summary: Some(summary),
        };
        if let Err(error) = self.parts.bus.helper_set(request.id, &update).await {
            tracing::warn!(
                event = LogEvent::BusDown.as_str(),
                request = %request.id,
                error = %error_chain(&error),
                "a request nobody took cannot be returned to its requester: the request waits on"
            );
            return false;
        }
        self.forget_wait(request).await;
        tracing::info!(
            event = LogEvent::WorkerWaitReturned.as_str(),
            request = %request.id,
            task = %request.from_task,
            step = %step,
            seconds,
            "a request nobody took was returned to its requester"
        );
        true
    }
}

#[cfg(test)]
mod tests;
