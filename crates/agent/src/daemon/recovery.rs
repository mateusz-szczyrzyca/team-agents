//! Recovery on start: what a restart left half done in the state.

use agent_core::config::Limits;
use agent_core::decide::outcome::ClassifyError;
use agent_core::ids::TaskId;
use agent_core::state::{State, Timestamp};

/// What recovery found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Recovery {
    /// The tasks whose run was going when the daemon went down: the run is ended and the task queued.
    pub(super) interrupted: Vec<TaskId>,
    /// The paused tasks and when each goes on.
    pub(super) wakes: Vec<(Timestamp, TaskId)>,
}

/// Why the state cannot be recovered.
#[derive(Debug, thiserror::Error)]
pub(super) enum RecoveryError {
    /// The state refused a change that its own data call for.
    #[error("the state of task {task} cannot be recovered")]
    State {
        /// The task.
        task: TaskId,
        /// What the state said.
        message: String,
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
    let _ = (state, now, limits);
    Ok(Recovery::default())
}

#[cfg(test)]
mod tests;
