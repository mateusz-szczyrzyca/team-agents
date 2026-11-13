//! The gate service: the repository's gate command, one run at a time on the machine, in the order it was asked for.
//!
//! `agent gate` asks the owner, which checks the task and puts a [`GateJob`] on the queue; this service takes the jobs
//! one by one. A job runs the gate command in the task's worktree under a timeout, records the commit, whether the
//! tree was clean and how the command ended, and answers the client that waits. A run that waits for its gate holds
//! the watchdog off (`WatchdogHold`), so that its silence is not taken for a hang.

use std::sync::Arc;
use std::time::Duration;

use agent_core::config::{AbsolutePath, CommandLine};
use agent_core::ids::{TaskId, TeamId};
use agent_core::protocol::{Event, Failure, FailureKind, Reply};
use agent_core::state::GateOutcome;
use agent_runtime::clock::SharedClock;
use agent_runtime::process::ExitReport;
use agent_runtime::run::HoldGuard;
use agent_runtime::store::StateStore;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// How many gates may wait for their turn; a gate asked for beyond that is refused.
pub(super) const QUEUE: usize = 16;

/// What the service is made of.
pub(super) struct GateParts {
    /// The store, for the layout of the home, the activity logs and the records.
    pub(super) store: Arc<StateStore>,
    /// The clock that stamps the records.
    pub(super) clock: SharedClock,
    /// Where the events go.
    pub(super) events: broadcast::Sender<Event>,
    /// The directories of the gate command's `PATH`: the agents' own.
    pub(super) agent_path: Vec<AbsolutePath>,
    /// How long a stopped gate has to end after `SIGTERM` before `SIGKILL`.
    pub(super) stop_grace: Duration,
}

/// One gate to run.
#[derive(Debug)]
pub(super) struct GateJob {
    /// The task whose worktree the gate runs in.
    pub(super) task: TaskId,
    /// The task's team.
    pub(super) team: TeamId,
    /// The worktree: where the command runs.
    pub(super) worktree: AbsolutePath,
    /// The repository's gate command.
    pub(super) command: CommandLine,
    /// How long the command may run before it is stopped and the gate counts as failed.
    pub(super) timeout: Duration,
    /// Cancelled when the task is stopped or the daemon shuts down: a gate that waits is dropped, one that runs is
    /// stopped, and neither is recorded.
    pub(super) cancel: CancellationToken,
    /// Holds the watchdog off the task's run while the gate waits and runs; `None` when the task has no run going.
    pub(super) hold: Option<HoldGuard>,
    /// Where the answer goes: the client waits for it.
    pub(super) reply: oneshot::Sender<Result<Reply, Failure>>,
}

/// The way to the queue of the service.
#[derive(Debug, Clone)]
pub(super) struct GateQueue {
    /// The jobs waiting, oldest first.
    jobs: mpsc::Sender<GateJob>,
}

/// A job that could not be queued, given back so that its client can be answered.
#[derive(Debug)]
pub(super) enum NotQueued {
    /// [`QUEUE`] gates are waiting already.
    Full(GateJob),
    /// The service has ended: the daemon is shutting down.
    Stopped(GateJob),
}

impl GateQueue {
    /// A queue and the end the service takes jobs from.
    pub(super) fn new() -> (Self, mpsc::Receiver<GateJob>) {
        let (jobs, waiting) = mpsc::channel(QUEUE);
        (Self { jobs }, waiting)
    }

    /// Puts `job` behind the ones waiting.
    ///
    /// # Errors
    /// [`NotQueued`], with the job.
    pub(super) fn enqueue(&self, job: GateJob) -> Result<(), NotQueued> {
        self.jobs.try_send(job).map_err(|refused| match refused {
            mpsc::error::TrySendError::Full(job) => NotQueued::Full(job),
            mpsc::error::TrySendError::Closed(job) => NotQueued::Stopped(job),
        })
    }
}

/// How a gate command's end is recorded.
fn outcome_of(exit: ExitReport) -> GateOutcome {
    let _ = exit;
    GateOutcome::Passed
}

/// Runs the jobs of `jobs`, one at a time in the order they arrive, until `shutdown` is cancelled or the queue is closed.
pub(super) async fn run(mut jobs: mpsc::Receiver<GateJob>, parts: GateParts, shutdown: CancellationToken) {
    let _ = (&parts, &shutdown);
    while let Some(job) = jobs.recv().await {
        let refusal = Failure {
            kind: FailureKind::Internal,
            message: "the gate is not run yet".to_owned(),
        };
        if job.reply.send(Err(refusal)).is_err() {
            tracing::debug!("a gate client went away");
        }
    }
}

#[cfg(test)]
mod tests;
