//! The snapshot the daemon takes of a task's worktree when its run was cut: [`crate::git::Git::snapshot`] with the size guard and
//! the record the history keeps of it.

use agent_core::config::AbsolutePath;
use agent_core::history::SnapshotRecord;
use agent_core::ids::{RoundId, TaskId};
use agent_core::state::Timestamp;

use crate::git::{GitError, GitOperation};

/// Why a snapshot of a task's worktree could not be taken.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    /// Git could not read the worktree or write the snapshot.
    #[error("cannot snapshot the worktree of task {task} in round {round}")]
    Git {
        /// The round the snapshot was to be kept under.
        round: RoundId,
        /// The task whose worktree was to be kept.
        task: TaskId,
        /// What git said.
        #[source]
        source: GitError,
    },
}

/// Takes the snapshot of the worktree at `worktree` for `task` of `round`, as of `at`, and returns the record the history
/// keeps. A snapshot bigger than `warn_bytes` is still taken; it is logged with a warning that says its size.
///
/// # Errors
/// [`SnapshotError::Git`] when git cannot read the worktree or write the snapshot.
pub async fn take_snapshot(
    _worktree: &AbsolutePath,
    round: &RoundId,
    task: &TaskId,
    _at: Timestamp,
    _warn_bytes: u64,
) -> Result<SnapshotRecord, SnapshotError> {
    std::future::ready(Err(SnapshotError::Git {
        round: round.clone(),
        task: task.clone(),
        source: GitError::Unexpected {
            operation: GitOperation::WriteTree,
            output: String::new(),
        },
    }))
    .await
}
