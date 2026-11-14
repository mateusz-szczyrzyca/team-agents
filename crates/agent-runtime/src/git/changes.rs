//! What a run changed, as git says it: the commits between two heads with their author time, and the files the worktree
//! holds that no commit does. The history records it at the end of every run.

use agent_core::decide::checks::FileChange;
use agent_core::history::CommitRecord;
use agent_core::state::CommitId;

use super::{Git, GitError};

/// What a run changed in its worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkChanges {
    /// The commit the worktree is at now.
    pub head_after: CommitId,
    /// The commits between the head the run started at and the one it ended at, oldest first.
    pub commits: Vec<CommitRecord>,
    /// The files that differ between the two heads.
    pub files: Vec<FileChange>,
    /// The files that differ between the head and the tree: changed, new or gone, in no commit.
    pub uncommitted: Vec<FileChange>,
}

impl Git {
    /// The commits after `base` up to `head`, oldest first, each with the first line of its message and when it was
    /// authored. A merge is listed after the commits it brought in; a bad byte of a subject is replaced.
    ///
    /// # Errors
    /// [`GitError::TooManyCommits`] when the range has more commits than [`GitLimits::max_commits`](super::GitLimits);
    /// [`GitError::Failed`] when a commit is not in the repository; [`GitError::Run`], [`GitError::OutputTooLong`] and
    /// [`GitError::Unexpected`].
    pub async fn range_log(&self, _base: &CommitId, _head: &CommitId) -> Result<Vec<CommitRecord>, GitError> {
        Err(GitError::EmptyMessage)
    }

    /// The files that differ between the commit this worktree is at and the files in it, the new ones included, with the
    /// lines each differs by. The repository is not changed.
    ///
    /// # Errors
    /// [`GitError::Failed`] when the directory is no repository; [`GitError::Run`], [`GitError::OutputTooLong`],
    /// [`GitError::Unexpected`], [`GitError::NotUtf8`] and [`GitError::NotARepoPath`].
    pub async fn uncommitted_changes(&self) -> Result<Vec<FileChange>, GitError> {
        Err(GitError::EmptyMessage)
    }

    /// What a run changed whose worktree was at `before` when it started: the commits and the files between `before` and
    /// the head now, and what is uncommitted. Without `before` nothing is known between the heads.
    ///
    /// # Errors
    /// The errors of [`Git::range_log`], [`Git::range_changes`] and [`Git::uncommitted_changes`].
    pub async fn work_changes(&self, _before: Option<&CommitId>) -> Result<WorkChanges, GitError> {
        Err(GitError::EmptyMessage)
    }
}
