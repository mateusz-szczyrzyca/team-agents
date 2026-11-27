//! Repository paths changed since the common ancestor with a base branch.
use super::{Git, GitError, GitOperation, TreeState};
use agent_core::decide::findings::{InvalidRepoPath, RepoPath};
use agent_core::ids::BranchName;
use std::collections::BTreeSet;
use std::os::unix::ffi::OsStrExt;

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
    pub async fn changed_files(&self, base: &BranchName) -> Result<Vec<RepoPath>, ChangedFilesError> {
        let reference = format!("refs/remotes/origin/{base}");
        let common = self
            .base_output(GitOperation::MergeBase, &["merge-base", "HEAD", &reference])
            .await?;
        let diff = self
            .base_output(GitOperation::Diff, &["diff", "--name-only", "-z", common.trim(), "--"])
            .await?;
        let mut paths = diff
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(RepoPath::parse)
            .collect::<Result<BTreeSet<_>, _>>()?;
        if let TreeState::Dirty(entries) = self.tree_state().await? {
            for entry in entries {
                let path = String::from_utf8(entry.path.as_os_str().as_bytes().to_vec()).map_err(|source| {
                    GitError::NotUtf8 {
                        operation: GitOperation::Status,
                        source,
                    }
                })?;
                paths.insert(RepoPath::parse(&path)?);
            }
        }
        Ok(paths.into_iter().collect())
    }
}
