//! A snapshot of a worktree: everything in it, kept as a commit under a reference of its round and task, without changing
//! the worktree, its index or the files. The snapshot is the part of a run's work that nobody else can see when the run is
//! cut; taking it is read-only for the checkout.

use agent_core::ids::{RoundId, TaskId};
use agent_core::state::CommitId;

use super::{Git, GitError, GitOperation};

/// A snapshot taken by [`Git::snapshot`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The number of the snapshot among those of its task, counted from 1.
    pub number: u32,
    /// The commit that holds the snapshot's tree.
    pub commit: CommitId,
    /// The head of the worktree the snapshot stands on: the parent of [`Snapshot::commit`].
    pub head: CommitId,
    /// The bytes of the files the snapshot holds.
    pub bytes: u64,
}

impl Git {
    /// Keeps everything in the worktree — modified, staged, deleted, untracked and binary files; ignored files are left out
    /// — as a commit under `refs/agents/snapshots/<round>/<task>/<n>`, whose parent is the head.
    ///
    /// The tree is built in a scratch index, so the worktree's index and files are not touched; the objects are written to
    /// the repository, where the reference keeps them. The repository's filters, hooks and filesystem monitor are switched
    /// off as for a read of the uncommitted changes.
    ///
    /// # Errors
    /// [`GitError`] of the git commands that read the head, the worktree and the references, and write the commit; the
    /// scratch directory is removed again whatever happens.
    pub async fn snapshot(&self, _round: &RoundId, _task: &TaskId) -> Result<Snapshot, GitError> {
        Err(GitError::Unexpected {
            operation: GitOperation::WriteTree,
            output: String::new(),
        })
    }
}
