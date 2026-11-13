//! The owner's side of `gate`: the task, the repository's gate and the run of the task are looked up and the gate is
//! put on the queue of the gate service, which answers the client when it has run.
//!
//! The owner never waits for a gate: the reply of the client goes with the job. While the gate waits and runs, the
//! task's run is held off the watchdog, and stopping the task stops its gate.

use std::str::FromStr;
use std::time::Duration;

use agent_core::config::{AbsolutePath, CommandLine};
use agent_core::ids::{TaskId, TeamId};
use agent_core::protocol::{Failure, FailureKind, GateRequest, Reply};
use agent_runtime::process::error_chain;
use agent_runtime::run::WatchdogHold;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::{Owner, not_found};
use crate::daemon::gate::{GateJob, NotQueued, QUEUE};

/// What the owner finds out about the gate of a task before it queues it.
struct Planned {
    /// The task's team.
    team: TeamId,
    /// The task's worktree.
    worktree: AbsolutePath,
    /// The repository's gate command.
    command: CommandLine,
    /// How long the command may run.
    timeout: Duration,
}

impl Owner {
    /// Queues the gate of the task `request` names; `reply` is answered by the gate service when the gate is over, or at
    /// once with the refusal when the gate cannot be queued.
    pub(super) fn gate_request(&mut self, request: GateRequest, reply: oneshot::Sender<Result<Reply, Failure>>) {
        let task = request.task;
        let planned = match self.plan_gate(&task) {
            Ok(planned) => planned,
            Err(failure) => return answer(reply, failure),
        };
        let Some(queue) = self.gate.clone() else {
            return answer(
                reply,
                Failure {
                    kind: FailureKind::Internal,
                    message: "this daemon runs without a gate service: it cannot run a gate".to_owned(),
                },
            );
        };
        let job = GateJob {
            hold: self.holds.get(&task).map(WatchdogHold::hold),
            cancel: self.gate_cancel(&task),
            task,
            team: planned.team,
            worktree: planned.worktree,
            command: planned.command,
            timeout: planned.timeout,
            reply,
        };
        if let Err(not_queued) = queue.enqueue(job) {
            let (job, failure) = match not_queued {
                NotQueued::Full(job) => (
                    job,
                    Failure {
                        kind: FailureKind::Refused,
                        message: format!(
                            "the gate queue is full: {QUEUE} gates are waiting for the machine's one gate at a time; ask again later"
                        ),
                    },
                ),
                NotQueued::Stopped(job) => (
                    job,
                    Failure {
                        kind: FailureKind::Internal,
                        message: "the daemon is shutting down".to_owned(),
                    },
                ),
            };
            answer(job.reply, failure);
        }
    }

    /// The team, the worktree and the repository's gate of `task`.
    fn plan_gate(&self, task: &TaskId) -> Result<Planned, Failure> {
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| not_found(format!("there is no task {task}")))?;
        let repo = found.team.repo();
        let configured = self.parts.settings.config.repo(repo).ok_or_else(|| Failure {
            kind: FailureKind::Refused,
            message: format!("the repository {repo} of task {task} is not in agents.toml: there is no gate to run"),
        })?;
        let path = self.parts.store.home().worktree_dir(repo, task);
        let text = path.to_str().ok_or_else(|| Failure {
            kind: FailureKind::Internal,
            message: format!("the worktree {} of task {task} is not UTF-8 text", path.display()),
        })?;
        let worktree = AbsolutePath::from_str(text).map_err(|source| Failure {
            kind: FailureKind::Internal,
            message: format!(
                "the worktree of task {task} is not a valid path: {}",
                error_chain(&source)
            ),
        })?;
        Ok(Planned {
            team: found.team.id().clone(),
            worktree,
            command: configured.gate.clone(),
            timeout: configured.gate_timeout_minutes.as_duration(),
        })
    }

    /// What stops the gate of `task`: the stop of its run; a task with no run has a token of its own, which the human's
    /// stop or pause cancels (`stop_gates`), and the shutdown of the daemon cancels both.
    fn gate_cancel(&mut self, task: &TaskId) -> CancellationToken {
        if let Some(cancel) = self.cancels.get(task) {
            return cancel.clone();
        }
        self.gate_stops
            .entry(task.clone())
            .or_insert_with(|| self.shutdown.child_token())
            .clone()
    }

    /// Stops the gates of `task` that were asked for while it had no run going.
    pub(super) fn stop_gates(&mut self, task: &TaskId) {
        if let Some(cancel) = self.gate_stops.remove(task) {
            cancel.cancel();
        }
    }
}

/// Answers the client of a gate with `failure`.
fn answer(reply: oneshot::Sender<Result<Reply, Failure>>, failure: Failure) {
    if reply.send(Err(failure)).is_err() {
        // The connection closed while its request waited: nobody is left to answer.
        tracing::debug!("the connection of a gate closed before the answer");
    }
}

#[cfg(test)]
mod tests;
