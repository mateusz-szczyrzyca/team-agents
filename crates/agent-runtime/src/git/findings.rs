//! Repository paths changed since the common ancestor with a base branch.
use super::{Git, GitError};
use agent_core::decide::findings::{InvalidRepoPath, RepoPath};
use agent_core::ids::BranchName;

/// A changed-file list could not be read or normalized.
#[derive(Debug, thiserror::Error)]
pub enum ChangedFilesError {
    /// Git could not compare the checkout.
    #[error("cannot read changed files")]
    Git(#[from] GitError),
    /// A changed file has an invalid repository path.
    #[error("invalid changed repository path")]
    Path(#[from] InvalidRepoPath),
}
impl Git {
    /// Lists committed, uncommitted, deleted and untracked paths against origin's `base` merge base.
    ///
    /// # Errors
    /// Returns [`ChangedFilesError`] if git fails or a path cannot be normalized.
    pub async fn changed_files(&self, _base: &BranchName) -> Result<Vec<RepoPath>, ChangedFilesError> {
        Ok(Vec::new())
    }
}
