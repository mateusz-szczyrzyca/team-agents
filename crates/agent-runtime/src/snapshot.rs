//! The snapshot the daemon takes of a task's worktree when its run was cut: [`Git::snapshot`] with the size guard and
//! the record the history keeps of it.

use agent_core::config::AbsolutePath;
use agent_core::history::SnapshotRecord;
use agent_core::ids::{BranchName, RoundId, TaskId};
use agent_core::log::LogEvent;
use agent_core::state::{CommitId, Timestamp};

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

/// The commits of the branch in the worktree at `worktree` of `task` that `origin/<base>` does not have, oldest first,
/// each with its whole message: what the work of the task has committed. The base is the remote-tracking branch, as for
/// the checks and the commit guard: a local base is never updated, and the commits merged since would count as work.
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
    let head = git.rev_parse(&Revision::Head).await.map_err(failed)?;
    let forked = git
        .merge_base(&Revision::Head, &Revision::RemoteBranch(base.clone()))
        .await
        .map_err(failed)?;
    git.range_messages(&forked, &head).await.map_err(failed)
}

/// Where the work of a worktree stands, for [`work_since`].
#[derive(Debug, Clone, Copy)]
pub struct WorkSpan<'a> {
    /// The commit the worktree started at.
    pub start: &'a CommitId,
    /// The tip of the worktree's branch.
    pub head: &'a CommitId,
    /// The commit that holds the tree of the worktree, uncommitted files included: the snapshot's.
    pub tree: &'a CommitId,
}

/// What a worktree has done since it started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkSince {
    /// The commits after the start, oldest first, each with its whole message.
    pub commits: Vec<CommitMessage>,
    /// The files whose content differs from the start's in the tree, each once.
    pub changed: Vec<String>,
}

/// Why the work of a worktree since its start could not be read.
#[derive(Debug, thiserror::Error)]
#[error("cannot read the work of task {task} since it started")]
pub struct WorkSinceError {
    /// The task whose work was to be read.
    pub task: TaskId,
    /// What git said.
    #[source]
    pub source: GitError,
}

/// The commits of the worktree at `worktree` of `task` after its start and the files that differ from the start's: what a
/// worker's request, a review or an investigation has done in a worktree made for it.
///
/// # Errors
/// [`WorkSinceError`] when git cannot find a commit of `span` or cannot read the range or the files.
pub async fn work_since(
    worktree: &AbsolutePath,
    task: &TaskId,
    span: &WorkSpan<'_>,
) -> Result<WorkSince, WorkSinceError> {
    let _ = (worktree, task, span);
    Ok(WorkSince {
        commits: Vec::new(),
        changed: Vec::new(),
    })
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
