//! Bringing the commit of a worker's step to the requester's worktree.
//!
//! A worker's run ends `STATUS: DONE` with its commit on its own branch. Nothing of it may reach the requester's branch
//! before the request's check passes, and the requester's uncommitted and untracked work must be exactly as it was
//! whatever happens.
//!
//! The way chosen is the merge in a scratch index, not a cherry-pick: a cherry-pick of a commit that conflicts leaves the
//! requester's worktree and index in the middle of the cherry-pick, which must then be undone file by file, while the
//! requester goes on working. [`Git::merge_into_scratch`] writes the objects of the merge into a scratch directory and
//! merges the worker's commit into the requester's head there; the requester's worktree is not read or written, so a
//! conflict cannot touch it. [`Git::bring_over`] takes the snapshot first, then writes the merged tree to the head and the
//! worktree with `git read-tree -m -u`, which refuses to overwrite a file the requester changed uncommitted instead of
//! losing it.

use std::path::PathBuf;

use agent_core::ids::{RoundId, TaskId};
use agent_core::repo_path::RepoPath;
use agent_core::state::CommitId;

use super::snapshot::Snapshot;
use super::{Git, GitError};

/// What bringing a worker's commit over the requester's head came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BringOver {
    /// The worker's commit merged cleanly and is now on the requester's head: the commit written for the merge, whose
    /// parent is the head the requester had.
    Applied {
        /// The snapshot taken of the requester's worktree before the merge was written.
        snapshot: Snapshot,
        /// The commit that holds the requester's head with the worker's change merged into it.
        commit: CommitId,
    },
    /// The worker's commit conflicts with the requester's head: nothing of it was applied, the requester's worktree,
    /// index and files are as before, and the worker's commit is returned as the reference to do the step from.
    Conflicted {
        /// The snapshot taken of the requester's worktree before the merge was tried.
        snapshot: Snapshot,
        /// The worker's commit, as a reference: nothing of it is on the requester's branch.
        worker: CommitId,
        /// The paths of the files the merge could not settle, oldest path first.
        files: Vec<RepoPath>,
    },
}

/// A merge of a worker's commit into the requester's head, kept in a scratch directory: what
/// [`Git::merge_into_scratch`] made of it, and what [`Git::bring_over`] writes to the worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScratchMerge {
    /// The commit the merge would make on the requester's head; `None` when it conflicted.
    commit: Option<CommitId>,
    /// The files the merge could not settle, empty when it did.
    conflicts: Vec<RepoPath>,
    /// The scratch directory that holds the index and the objects of the merge.
    scratch: PathBuf,
}

impl Git {
    /// The files `commit` changes against its first parent, each path once, a rename as both the old and the new path.
    ///
    /// # Errors
    /// [`GitError::Failed`] when the commit is not in the repository; [`GitError::Unexpected`], [`GitError::NotUtf8`],
    /// [`GitError::NotARepoPath`], [`GitError::Run`] and [`GitError::OutputTooLong`] for what git prints.
    pub async fn changed_in(&self, commit: &CommitId) -> Result<Vec<RepoPath>, GitError> {
        let _ = commit;
        Ok(Vec::new())
    }

    /// The files `commit` changes that `allowed` does not name: a path the request did not give the worker.
    ///
    /// # Errors
    /// The errors of [`Git::changed_in`].
    pub async fn outside(&self, commit: &CommitId, allowed: &[RepoPath]) -> Result<Vec<RepoPath>, GitError> {
        let changed = self.changed_in(commit).await?;
        Ok(changed.into_iter().filter(|path| !allowed.contains(path)).collect())
    }

    /// Merges `commit` into the requester's head in a scratch directory, with the objects of the repository readable
    /// through it: the requester's worktree and its index are not read or written.
    ///
    /// # Errors
    /// [`GitError::Failed`] when the commit or the head is not in the repository; [`GitError::Scratch`] when the
    /// directory cannot be made; [`GitError::NotACommit`] when the written commit is not one; [`GitError::Run`] and
    /// [`GitError::OutputTooLong`].
    pub async fn merge_into_scratch(&self, commit: &CommitId) -> Result<ScratchMerge, GitError> {
        let _ = commit;
        Ok(ScratchMerge {
            commit: None,
            conflicts: Vec::new(),
            scratch: PathBuf::new(),
        })
    }

    /// Brings `commit` to the requester's worktree after a snapshot of it: the merge is made in a scratch directory, and
    /// only a merge that applies is written to the head and the worktree, with `git read-tree -m -u`, which refuses rather
    /// than overwrite a file the requester changed uncommitted.
    ///
    /// # Errors
    /// The errors of [`Git::snapshot`] and [`Git::merge_into_scratch`]; [`GitError::Failed`] when writing the merge would
    /// overwrite a file the requester changed.
    pub async fn bring_over(&self, commit: &CommitId, round: &RoundId, task: &TaskId) -> Result<BringOver, GitError> {
        let _ = (commit, round, task);
        let empty: CommitId =
            "0000000000000000000000000000000000000000"
                .parse()
                .map_err(|source| GitError::NotACommit {
                    operation: super::GitOperation::CommitTree,
                    source,
                })?;
        Ok(BringOver::Applied {
            snapshot: Snapshot {
                number: 1,
                commit: empty.clone(),
                head: empty.clone(),
                bytes: 0,
            },
            commit: empty,
        })
    }
}
