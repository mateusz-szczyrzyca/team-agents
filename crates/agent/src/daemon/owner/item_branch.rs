//! The branch a run works on: before a run of a task that has an item, its worktree is put on the item's branch, so that
//! what the round judges and publishes afterwards is the item's work and not whatever branch the worktree was left on.

use agent_core::ids::{BranchName, TaskId};
use agent_core::state::ItemPhase;
use agent_runtime::git::TreeState;

use super::{Owner, OwnerError};

#[cfg(test)]
mod tests;

/// What is done to a worktree before a run, as far as its branch goes.
#[cfg_attr(not(test), expect(dead_code, reason = "wired in the green commit of this change"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Checkout {
    /// The worktree has the target branch out already.
    Stay,
    /// Another branch, or none, is out and the tree has nothing a switch could lose: the worktree is switched.
    Switch,
    /// Another branch, or none, is out and the tree has changes: the worktree is left as it is.
    Refuse,
}

/// The branch the next run of `agent` works on: the branch of the item it is fixing; else the branch of the one item of
/// `phases` it has in progress; `None` when it has neither, or has several items in progress.
#[cfg_attr(not(test), expect(dead_code, reason = "wired in the green commit of this change"))]
pub(super) fn target_branch<'a>(_phases: impl Iterator<Item = &'a ItemPhase>, _agent: &str) -> Option<&'a BranchName> {
    None
}

/// What to do about the worktree that has `checked_out` out (`None` on a detached HEAD) when the run is to work on
/// `target`, given the state of its tree. Untracked files do not count: a switch keeps them.
#[cfg_attr(not(test), expect(dead_code, reason = "wired in the green commit of this change"))]
pub(super) fn decide(_target: &BranchName, _checked_out: Option<&BranchName>, _tree: &TreeState) -> Checkout {
    Checkout::Stay
}

impl Owner {
    /// Puts the worktree of `task` on the branch of the item its next run works on, when it has another branch out and
    /// its tree has no changes. A tree with changes, or a switch git refuses, is logged and left: the run starts anyway.
    ///
    /// # Errors
    /// [`OwnerError`] when `task` is not in the state.
    #[expect(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "the stub of the red commit; the green commit replaces it"
    )]
    pub(super) async fn checkout_item_branch(&mut self, _task: &TaskId) -> Result<(), OwnerError> {
        Ok(())
    }
}
