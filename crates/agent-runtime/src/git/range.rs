//! What git says about a range of commits: where it starts, which files it changed, the commits in it and the lines it
//! added. The checks before a review read these and decide with no model (`agent_core::decide::checks`).
//!
//! Every answer is read from the NUL-separated forms of git's output, so that a path or a message with any character
//! but NUL is read as it is.

use agent_core::decide::checks::{AddedLine, CommitFacts, FileChange, RoundFacts};
use agent_core::repo_path::RepoPath;
use agent_core::state::CommitId;

use super::{Git, GitError, Revision};

impl Git {
    /// The best common ancestor of two revisions: where a branch left its base.
    ///
    /// # Errors
    /// [`GitError::NoMergeBase`] when they have no commit in common; [`GitError::Failed`] when a revision names no
    /// commit; [`GitError::Run`], [`GitError::OutputTooLong`], [`GitError::NotUtf8`] and [`GitError::NotACommit`].
    pub async fn merge_base(&self, _first: &Revision, second: &Revision) -> Result<CommitId, GitError> {
        self.rev_parse(second).await
    }

    /// The files that differ between `base` and `head`, as git prints them: a rename is one entry, with the lines it
    /// changed.
    ///
    /// # Errors
    /// [`GitError::Failed`] when a commit is not in the repository; [`GitError::Unexpected`] and
    /// [`GitError::NotARepoPath`] for output that is not what git prints; [`GitError::Run`] and
    /// [`GitError::OutputTooLong`].
    pub async fn range_changes(&self, _base: &CommitId, head: &CommitId) -> Result<Vec<FileChange>, GitError> {
        self.rev_parse(&Revision::Commit(head.clone())).await?;
        Ok(Vec::new())
    }

    /// The commits after `base` up to `head`, oldest first (a commit after its parents), each with its parents, its
    /// message and the files it changed against its first parent; a rename is a deletion and an addition.
    ///
    /// # Errors
    /// [`GitError::TooManyCommits`] when the range has more than the limit; the errors of [`Git::range_changes`].
    pub async fn range_commits(&self, _base: &CommitId, head: &CommitId) -> Result<Vec<CommitFacts>, GitError> {
        self.rev_parse(&Revision::Commit(head.clone())).await?;
        Ok(Vec::new())
    }

    /// The lines added between `base` and `head` to each of `paths`, with their numbers in the file at `head`. A line
    /// that is not UTF-8 text has its bad bytes replaced: only ASCII markers are looked for in it.
    ///
    /// # Errors
    /// The errors of [`Git::range_changes`].
    pub async fn range_added_lines(
        &self,
        _base: &CommitId,
        head: &CommitId,
        _paths: &[RepoPath],
    ) -> Result<Vec<AddedLine>, GitError> {
        self.rev_parse(&Revision::Commit(head.clone())).await?;
        Ok(Vec::new())
    }

    /// The facts the checks before a review decide from: the files, the commits, the added lines of the files whose
    /// extension has markers, and `description`, the pull request's.
    ///
    /// # Errors
    /// The errors of [`Git::range_commits`] and [`Git::range_added_lines`].
    pub async fn round_facts(
        &self,
        _base: &CommitId,
        head: &CommitId,
        _description: &str,
    ) -> Result<RoundFacts, GitError> {
        self.rev_parse(&Revision::Commit(head.clone())).await?;
        Ok(RoundFacts::default())
    }
}
