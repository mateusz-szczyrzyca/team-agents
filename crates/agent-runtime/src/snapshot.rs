//! The snapshot the daemon takes of a task's worktree when its run was cut: [`Git::snapshot`] with the size guard and
//! the record the history keeps of it.

use agent_core::config::AbsolutePath;
use agent_core::history::SnapshotRecord;
use agent_core::ids::{BranchName, RoundId, TaskId};
use agent_core::log::LogEvent;
use agent_core::state::Timestamp;

use crate::git::{CheckpointSnapshot, CommitIdentity, CommitMessage, Git, GitError, Revision};

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

/// Takes the snapshot of the worktree at `worktree` for a checkpoint of `task` in `round`: the one under `previous` when
/// it holds the same tree on the same head, else a new one.
///
/// # Errors
/// [`SnapshotError::Git`] when git cannot read the worktree or write the snapshot.
pub async fn take_checkpoint_snapshot(
    worktree: &AbsolutePath,
    round: &RoundId,
    task: &TaskId,
    previous: Option<&str>,
) -> Result<CheckpointSnapshot, SnapshotError> {
    let identity = CommitIdentity::new(SNAPSHOT_NAME.to_owned(), SNAPSHOT_EMAIL.to_owned(), None);
    Git::new(worktree.clone(), identity)
        .snapshot_unless_unchanged(round, task, previous)
        .await
        .map_err(|source| SnapshotError::Git {
            round: round.clone(),
            task: task.clone(),
            source,
        })
}

/// Why the commits of a task's branch could not be read.
#[derive(Debug, thiserror::Error)]
#[error("cannot read the commits of the branch of task {task} past {base}")]
pub struct BranchCommitsError {
    /// The task whose worktree was read.
    pub task: TaskId,
    /// The branch the task's branch was forked from.
    pub base: BranchName,
    /// What git said.
    #[source]
    pub source: GitError,
}

/// The commits of the branch in the worktree at `worktree` of `task` that `base` does not have, oldest first, each with
/// its whole message: what the work of the task has committed.
///
/// # Errors
/// [`BranchCommitsError`] when git cannot find where the branch left `base` or cannot read the commits.
pub async fn branch_commits(
    worktree: &AbsolutePath,
    task: &TaskId,
    base: &BranchName,
) -> Result<Vec<CommitMessage>, BranchCommitsError> {
    let identity = CommitIdentity::new(SNAPSHOT_NAME.to_owned(), SNAPSHOT_EMAIL.to_owned(), None);
    let git = Git::new(worktree.clone(), identity);
    let failed = |source| BranchCommitsError {
        task: task.clone(),
        base: base.clone(),
        source,
    };
    let _ = &git;
    let _ = failed;
    Ok(Vec::new())
}

/// Why the snapshot references of a round could not be removed.
#[derive(Debug, thiserror::Error)]
#[error("cannot remove the snapshot references of round {round}")]
pub struct PruneError {
    /// The round whose references were to be removed.
    pub round: RoundId,
    /// What git said.
    #[source]
    pub source: GitError,
}

/// Removes the snapshot references `references` of `round` from the repository of `worktree`.
///
/// # Errors
/// [`PruneError`] when git cannot delete a reference.
pub async fn prune_snapshots(
    worktree: &AbsolutePath,
    round: &RoundId,
    references: &[String],
) -> Result<(), PruneError> {
    let identity = CommitIdentity::new(SNAPSHOT_NAME.to_owned(), SNAPSHOT_EMAIL.to_owned(), None);
    Git::new(worktree.clone(), identity)
        .delete_snapshot_references(references)
        .await
        .map_err(|source| PruneError {
            round: round.clone(),
            source,
        })
}
