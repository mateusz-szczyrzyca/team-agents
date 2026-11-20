//! Fail-closed evidence that an item branch adds no work outside a team's PR or base.

use super::{Git, GitError, Revision, TreeState};
use agent_core::ids::BranchName;

/// The evidence that could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryRead {
    /// The checkout's tracked and untracked changes.
    Tree,
    /// The checked-out commit.
    Head,
    /// The item's named branch tip.
    Item,
    /// The current base tip and ancestry.
    Base,
    /// The owning PR's actual branch tip and ancestry.
    PullRequest,
}

/// Why git could not prove that an item is already delivered.
#[derive(Debug, thiserror::Error)]
pub enum DeliveryGitError {
    /// A required git operation failed; absence of evidence never accepts a claim.
    #[error("cannot read delivery evidence {read:?}")]
    Read {
        /// Which proof was being read.
        read: DeliveryRead,
        /// Git's original failure.
        #[source]
        source: GitError,
    },
    /// Tracked or untracked changes would be lost by closing the item.
    #[error("the item's worktree has uncommitted changes")]
    Dirty,
    /// The worktree is not at the item's named branch tip.
    #[error("the item's worktree is not at its branch tip")]
    Checkout,
    /// The item still owns commits outside both accepted destinations.
    #[error("the item has commits outside the PR and base branches")]
    OwnCommits,
}

impl Git {
    /// Verifies a clean checkout at `item` whose entire tip is contained in `base` or `pr`.
    ///
    /// The caller refreshes the base and supplies the actual PR branch from the team's persisted state.
    ///
    /// # Errors
    /// Returns [`DeliveryGitError`] when any evidence is missing, the checkout is dirty or different,
    /// or the item has commits of its own.
    pub async fn verify_delivery(
        &self,
        item: &BranchName,
        pr: &BranchName,
        base: &Revision,
    ) -> Result<(), DeliveryGitError> {
        let read = |read, source| DeliveryGitError::Read { read, source };
        if !matches!(
            self.tree_state()
                .await
                .map_err(|source| read(DeliveryRead::Tree, source))?,
            TreeState::Clean
        ) {
            return Err(DeliveryGitError::Dirty);
        }
        let head = self
            .rev_parse(&Revision::Head)
            .await
            .map_err(|source| read(DeliveryRead::Head, source))?;
        let tip = self
            .rev_parse(&Revision::Branch(item.clone()))
            .await
            .map_err(|source| read(DeliveryRead::Item, source))?;
        if head != tip {
            return Err(DeliveryGitError::Checkout);
        }
        let tip = Revision::Commit(tip);
        if self
            .is_ancestor(&tip, base)
            .await
            .map_err(|source| read(DeliveryRead::Base, source))?
        {
            return Ok(());
        }
        if self
            .is_ancestor(&tip, &Revision::Branch(pr.clone()))
            .await
            .map_err(|source| read(DeliveryRead::PullRequest, source))?
        {
            return Ok(());
        }
        Err(DeliveryGitError::OwnCommits)
    }
}

#[cfg(test)]
mod tests;
