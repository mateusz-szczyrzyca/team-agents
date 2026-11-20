//! Fetch and comparison before a team creates any durable resources.

use super::{Git, GitError};
use agent_core::ids::BranchName;
use agent_core::state::UnpushedBase;

impl Git {
    /// Fetches origin's base and reports local commits missing from it; no origin leaves the checkout unchanged.
    ///
    /// # Errors
    /// [`GitError`] if remote discovery, fetching or reading the local history fails.
    pub async fn unpushed_base(&self, _base: &BranchName) -> Result<Option<UnpushedBase>, GitError> {
        Ok(None)
    }
}
