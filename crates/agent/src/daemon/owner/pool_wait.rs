//! How long a request waits in the queue for a worker of the pool: the daemon notes since when each queued request
//! waits and, when `[workers] wait_seconds` has passed, fails the request with the words that tell its requester to do
//! the step itself.

use agent_core::decide::worker_wait::{WaitVerdict, wait_verdict};
use agent_core::ids::TaskId;
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
        self.clear_wait(request.id, &request.from_task).await;
    }

    /// Takes away the notes the daemon holds of requests that `open` does not list as queued: the bus lists only open
    /// requests, so a request that was failed or settled while it waited is gone from it.
    pub(super) async fn forget_left_waits(&mut self, open: &[HelperEntry]) {
        let left: Vec<(RequestId, TaskId)> = self
            .pool_dispatch
            .waits_noted
            .iter()
            .filter(|(id, _)| {
                !open
                    .iter()
                    .any(|request| request.id == **id && request.status == DelegationStatus::Queued)
            })
            .map(|(id, task)| (*id, task.clone()))
            .collect();
        for (id, task) in left {
            self.clear_wait(id, &task).await;
        }
    }

    /// Removes the note of the wait of `request` from the directory of `task` and stops holding it.
    async fn clear_wait(&mut self, request: RequestId, task: &TaskId) {
        let task_dir = self.parts.store.home().task_dir(task);
        match worker_wait::clear(&task_dir, request).await {
            Ok(()) => {
                self.pool_dispatch.waits_noted.remove(&request);
            }
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::WorkerRequestDamaged.as_str(),
                    request = %request,
                    error = %error_chain(&error),
                    "the note of the wait of a request cannot be removed"
                );
            }
        }
    }

    /// Notes since when the queued `request` waits, at the first look at it, and returns it to its requester when it has
    /// waited `[workers] wait_seconds`; `true` when it was returned and is not to be served.
    ///
    /// A request that is on its second try or whose workspace is kept for a recovery is not counted: it is served as before.
    pub(super) async fn wait_returns(&mut self, request: &HelperEntry) -> bool {
        if self.again.contains_key(&request.id) {
            return false;
        }
        match self.owned_journal(&request.from_task, request.id).await {
            Ok(None) => {}
            Ok(Some(_)) => return false,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::WorkerRequestDamaged.as_str(),
                    request = %request.id,
                    error = %error_chain(&error),
                    "the journal of a queued request cannot be read: its wait is not counted"
                );
                return false;
            }
        }
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
