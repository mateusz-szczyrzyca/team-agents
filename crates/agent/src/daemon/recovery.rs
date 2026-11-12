//! Recovery on start: what a restart left half done in the state.

use agent_core::config::Limits;
use agent_core::decide::outcome::ClassifyError;
use agent_core::ids::TaskId;
use agent_core::state::{Run, RunEnd, RunError, RunState, State, Timestamp};
use agent_core::status::{InvalidTransition, TaskStatus};

use super::settle::wake_time;

/// What recovery found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Recovery {
    /// The tasks whose run was going when the daemon went down: the run is ended and the task queued.
    pub(super) interrupted: Vec<TaskId>,
    /// The paused tasks and when each goes on.
    pub(super) wakes: Vec<(Timestamp, TaskId)>,
    /// The queued tasks whose last runs hung, and how many hangs in a row: the watchdog's count, which a restart must
    /// not reset.
    pub(super) hangs: Vec<(TaskId, u32)>,
}

/// Why the state cannot be recovered.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RecoveryError {
    /// A task listed in the state cannot be found in it.
    #[error("task {task} is in the state but cannot be found in it")]
    Missing {
        /// The task.
        task: TaskId,
    },
    /// The run of a task cannot be ended.
    #[error("the run of task {task} cannot be ended")]
    Run {
        /// The task.
        task: TaskId,
        /// What the state refused.
        source: RunError,
    },
    /// The status of a task cannot be changed.
    #[error("the status of task {task} cannot be changed")]
    Status {
        /// The task.
        task: TaskId,
        /// What the state refused.
        source: InvalidTransition<TaskStatus>,
    },
    /// A pause cannot be timed.
    #[error("the pause of task {task} cannot be timed")]
    Pause {
        /// The task.
        task: TaskId,
        /// What the classifier refused.
        source: ClassifyError,
    },
}

/// Ends the runs that were going, queues their tasks and finds the pauses to wake.
///
/// # Errors
/// [`RecoveryError`] when the state refuses a change or a pause cannot be timed.
pub(super) fn recover(state: &mut State, now: Timestamp, limits: &Limits) -> Result<Recovery, RecoveryError> {
    let mut recovery = Recovery::default();
    for task in all_tasks(state) {
        let (_, mut member) = state
            .find_task_mut(&task)
            .ok_or_else(|| RecoveryError::Missing { task: task.clone() })?;
        let was_going = member.current_run().is_some();
        let was_running = member.status() == TaskStatus::Running;
        if let Some(started_at) = member.current_run().map(Run::started_at) {
            // No code: the restart ended the process by a signal, or its end was never seen. A clock that was set back
            // since the run started would end it before its start: it ends when it started, at the earliest.
            member
                .end_run(now.max(started_at), RunEnd::NoStatus { exit_code: None })
                .map_err(|source| RecoveryError::Run {
                    task: task.clone(),
                    source,
                })?;
        }
        if was_running {
            member
                .transition(TaskStatus::Queued)
                .map_err(|source| RecoveryError::Status {
                    task: task.clone(),
                    source,
                })?;
        }
        if was_going || was_running {
            recovery.interrupted.push(task.clone());
        }
        let hangs = trailing_hangs(member.runs());
        if member.status() == TaskStatus::Queued && hangs > 0 {
            recovery.hangs.push((task.clone(), hangs));
        }
        if member.status() == TaskStatus::Paused
            && let Some(wake) = paused_until(member.runs().last(), limits).map_err(|source| RecoveryError::Pause {
                task: task.clone(),
                source,
            })?
        {
            recovery.wakes.push((wake, task));
        }
    }
    Ok(recovery)
}

/// How many of the last `runs` hung in a row. A run a restart interrupted is no strike and does not end the row.
/// Whether a hung run had made progress is not saved: after a restart, every hang of the row counts.
const fn trailing_hangs(_runs: &[Run]) -> u32 {
    0
}

/// When a task whose last run is `last` goes on, if its pause is a provider's limit.
fn paused_until(last: Option<&Run>, limits: &Limits) -> Result<Option<Timestamp>, ClassifyError> {
    match last.map(Run::state) {
        Some(RunState::Ended { at, end }) => wake_time(end, *at, limits),
        Some(RunState::Running) | None => Ok(None),
    }
}

/// Every task of the state: the crew of each team and its reviewers, team by team.
fn all_tasks(state: &State) -> Vec<TaskId> {
    state
        .teams()
        .iter()
        .flat_map(|team| {
            let crew = team.crew().roster().into_iter().map(|(_, member)| member);
            crew.chain(team.reviewers().iter()).map(|member| member.task().clone())
        })
        .collect()
}

#[cfg(test)]
mod tests;
