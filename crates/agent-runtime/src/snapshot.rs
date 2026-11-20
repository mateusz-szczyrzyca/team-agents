//! The snapshot the daemon takes of a task's worktree when its run was cut: [`Git::snapshot`] with the size guard and
//! the record the history keeps of it.

use agent_core::config::AbsolutePath;
use agent_core::history::SnapshotRecord;
use agent_core::ids::{RoundId, TaskId};
use agent_core::log::LogEvent;
use agent_core::state::Timestamp;

use crate::git::{CommitIdentity, Git, GitError};

/// The name the commits of snapshots have: a snapshot is the harness's commit, not an agent's.
const SNAPSHOT_NAME: &str = "team-agents";

/// The e-mail address the commits of snapshots have; a reserved domain that is never delivered to.
const SNAPSHOT_EMAIL: &str = "team-agents@team-agents.invalid";

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
    worktree: &AbsolutePath,
    round: &RoundId,
    task: &TaskId,
    at: Timestamp,
    warn_bytes: u64,
) -> Result<SnapshotRecord, SnapshotError> {
    let identity = CommitIdentity::new(SNAPSHOT_NAME.to_owned(), SNAPSHOT_EMAIL.to_owned(), None);
    let taken = Git::new(worktree.clone(), identity)
        .snapshot(round, task)
        .await
        .map_err(|source| SnapshotError::Git {
            round: round.clone(),
            task: task.clone(),
            source,
        })?;
    if taken.bytes > warn_bytes {
        tracing::warn!(
            event = LogEvent::SnapshotLarge.as_str(),
            round = %round,
            task = %task,
            number = taken.number,
            bytes = taken.bytes,
            limit_bytes = warn_bytes,
            "a snapshot is bigger than the configured size: it is kept"
        );
    }
    Ok(SnapshotRecord {
        round: round.clone(),
        task: task.clone(),
        number: taken.number,
        at,
        commit: taken.commit,
        head: taken.head,
        bytes: taken.bytes,
    })
}
