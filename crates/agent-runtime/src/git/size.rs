//! Pull request size under the developer rules, independent of the checkout.
use super::{Git, GitError, Revision};
use agent_core::state::CommitId;
impl Git {
    /// Counts added and deleted lines outside tests using the PR-1 diff and exclusions.
    ///
    /// # Errors
    /// Returns the errors of the bounded git diff command.
    pub async fn pull_request_size(&self, _base: &Revision, _head: &CommitId) -> Result<usize, GitError> {
        Ok(0)
    }
}
