//! The branch a run works on: before a run of a task that has an item, its worktree is put on the item's branch, so that
//! what the round judges and publishes afterwards is the item's work and not whatever branch the worktree was left on.

use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::ids::{AgentName, BranchName, TaskId};
use agent_core::log::LogEvent;
use agent_core::state::{ItemPhase, ItemProgress};
use agent_runtime::git::{CommitIdentity, Git, TreeState};
use agent_runtime::process::error_chain;

use super::{Owner, OwnerError};

#[cfg(test)]
mod tests;

/// How a worktree with no branch out, a detached HEAD, is named to a lead and in the log.
const DETACHED: &str = "a detached HEAD";

/// The name of the branch `checked_out`, or [`DETACHED`] when no branch is out.
pub(super) fn named(checked_out: Option<&BranchName>) -> String {
    checked_out.map_or_else(|| DETACHED.to_owned(), ToString::to_string)
}

/// What is done to a worktree before a run, as far as its branch goes.
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
pub(super) fn target_branch<'a>(phases: impl Iterator<Item = &'a ItemPhase>, agent: &str) -> Option<&'a BranchName> {
    let mut fixing = None;
    let mut in_progress = Vec::new();
    for phase in phases {
        match phase {
            ItemPhase::Fixing { assignee, branch, .. } if assignee.as_str() == agent => {
                fixing.get_or_insert(branch);
            }
            ItemPhase::InProgress { assignee, branch } if assignee.as_str() == agent => in_progress.push(branch),
            ItemPhase::Fixing { .. }
            | ItemPhase::InProgress { .. }
            | ItemPhase::Pending
            | ItemPhase::InReview { .. }
            | ItemPhase::Approved { .. }
            | ItemPhase::Done { .. } => {}
        }
    }
    fixing.or(match in_progress.as_slice() {
        [only] => Some(*only),
        _ => None,
    })
}

/// What to do about the worktree that has `checked_out` out (`None` on a detached HEAD) when the run is to work on
/// `target`, given the state of its tree. Untracked files do not count: a switch keeps them.
pub(super) fn decide(target: &BranchName, checked_out: Option<&BranchName>, tree: &TreeState) -> Checkout {
    if checked_out == Some(target) {
        return Checkout::Stay;
    }
    let loses = match tree {
        TreeState::Clean => false,
        TreeState::Dirty(entries) => entries.iter().any(|entry| (entry.index, entry.worktree) != ('?', '?')),
    };
    if loses { Checkout::Refuse } else { Checkout::Switch }
}

/// What came of putting a worktree on the branch of its item.
enum Outcome {
    /// The worktree had the branch out already, or no item named one.
    Stayed,
    /// The worktree was switched; it had `from` out.
    Switched {
        /// The branch the worktree had out, or [`DETACHED`].
        from: String,
    },
    /// The worktree, which had `from` out, was left as it was: its tree has changes (`error` is `None`), or git refused
    /// the switch (`error` is its answer).
    Refused {
        /// The branch the worktree had out, or [`DETACHED`].
        from: String,
        /// What git said, when it refused.
        error: Option<String>,
    },
    /// Git could not tell which branch the worktree has out, or what its tree holds: nothing was done.
    Unreadable {
        /// What git said.
        error: String,
    },
}

/// Puts the worktree of `git` on `target` when [`decide`] says so.
async fn put_on(git: &Git, target: &BranchName) -> Outcome {
    let checked_out = match git.current_branch().await {
        Ok(checked_out) => checked_out,
        Err(error) => {
            return Outcome::Unreadable {
                error: error_chain(&error),
            };
        }
    };
    let tree = match git.tree_state().await {
        Ok(tree) => tree,
        Err(error) => {
            return Outcome::Unreadable {
                error: error_chain(&error),
            };
        }
    };
    let from = named(checked_out.as_ref());
    match decide(target, checked_out.as_ref(), &tree) {
        Checkout::Stay => Outcome::Stayed,
        Checkout::Refuse => Outcome::Refused { from, error: None },
        Checkout::Switch => match git.switch_branch(target).await {
            Ok(()) => Outcome::Switched { from },
            Err(error) => Outcome::Refused {
                from,
                error: Some(error_chain(&error)),
            },
        },
    }
}

impl Owner {
    /// Puts the worktree of `task` on the branch of the item its next run works on, when it has another branch out and
    /// its tree has no changes. A tree with changes, or a switch git refuses, is logged and left: the run starts anyway.
    ///
    /// # Errors
    /// [`OwnerError`] when `task` is not in the state.
    pub(super) async fn checkout_item_branch(&mut self, task: &TaskId) -> Result<(), OwnerError> {
        let found = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        let agent = found.member.agent().name();
        let Some(target) = target_branch(found.team.items().iter().map(ItemProgress::phase), agent).cloned() else {
            return Ok(());
        };
        let team = found.team.id().clone();
        let path = self.parts.store.home().worktree_dir(found.team.repo(), task);
        let git = path
            .to_str()
            .ok_or_else(|| format!("the worktree path {} is not valid UTF-8", path.display()))
            .and_then(|text| AbsolutePath::from_str(text).map_err(|error| error_chain(&error)))
            .and_then(|worktree| {
                let identity = AgentName::from_str(agent)
                    .map(|name| CommitIdentity::for_agent(self.parts.settings.config.git(), &name))
                    .map_err(|error| error_chain(&error))?;
                Ok(Git::new(worktree, identity))
            });
        let outcome = match git {
            Ok(git) => put_on(&git, &target).await,
            Err(error) => Outcome::Unreadable { error },
        };
        let span = tracing::error_span!("task", team = %team, task = %task);
        let _entered = span.enter();
        match outcome {
            Outcome::Stayed => {}
            Outcome::Switched { from } => tracing::info!(
                event = LogEvent::WorktreeSwitched.as_str(),
                from = %from,
                to = %target,
                "the worktree was switched to the branch of its item"
            ),
            Outcome::Refused { from, error: None } => tracing::warn!(
                event = LogEvent::WorktreeSwitchRefused.as_str(),
                from = %from,
                to = %target,
                "the worktree was not switched to the branch of its item: its tree has changes"
            ),
            Outcome::Refused {
                from,
                error: Some(error),
            } => tracing::warn!(
                event = LogEvent::WorktreeSwitchRefused.as_str(),
                from = %from,
                to = %target,
                error = %error,
                "the worktree was not switched to the branch of its item: git refused"
            ),
            Outcome::Unreadable { error } => tracing::warn!(
                event = LogEvent::GitCallFailed.as_str(),
                to = %target,
                error = %error,
                "the branch the worktree has out cannot be read: it is not switched to the branch of its item"
            ),
        }
        Ok(())
    }
}
