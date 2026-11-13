//! Pull requests: which items are published through which branch, and what is written on the pull request.
//!
//! A pull request is made of the branch of the lead of the team: the coordinator integrates the members' branches into
//! its own and gets the gate green on it, and a solo agent's pull request is made of the branch of its item. The
//! daemon publishes that branch when the lead's last gate result passes on a clean tree at its tip; what is decided
//! here is which items go in which pull request, and the texts. Pushing and talking to GitHub is the owner's
//! (`owner/rounds.rs`).

use agent_core::config::RepoName;
use agent_core::ids::{BranchName, InvalidBranchName, InvalidName, ItemId};
use agent_core::prompt::{AutopilotPrompt, VariablesError};
use agent_core::state::{GateRecord, PullRequest, Team};
use agent_runtime::autopilot_prompts::{AutopilotPrompts, RenderAutopilotTextError};

/// The longest title of a pull request the harness writes, in characters: GitHub's limit is 256.
pub(super) const MAX_TITLE_CHARS: usize = 200;

/// An item that joins a pull request: finished by its member, to be checked against the branch of the pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Joining {
    /// The item.
    pub(super) item: ItemId,
    /// The branch its member worked on it on; it is part of the pull request when the head contains it.
    pub(super) branch: BranchName,
}

/// What is published of a branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Publish {
    /// A pull request is opened for these items, none of which has one.
    Open {
        /// The items that go in it.
        items: Vec<Joining>,
    },
    /// The pull request is updated: its items were fixed, and these finished items join it.
    Update {
        /// The pull request.
        pr: PullRequest,
        /// The items of the pull request that are being fixed.
        fixing: Vec<ItemId>,
        /// The finished items that join it.
        joining: Vec<Joining>,
    },
}

/// A branch that is to be published, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Publication {
    /// The branch the pull request is made of.
    pub(super) head_branch: BranchName,
    /// What is done to the pull request.
    pub(super) publish: Publish,
}

/// Why the publications of a team cannot be decided.
#[derive(Debug, thiserror::Error)]
pub(super) enum PlanError {
    /// The name of the lead is not an agent name.
    #[error("the name of the lead of the team is not an agent name")]
    Name(#[from] InvalidName),
    /// The branch of the lead is not a branch name.
    #[error("the branch of the lead of the team is not a branch name")]
    Branch(#[from] InvalidBranchName),
}

/// The branches of `team` that are ready to be published: the lead has finished (its task waits) and has something to
/// publish.
///
/// - A finished item (its member waits) with no pull request opens one. A branch has one pull request at a time: while
///   an item of the team is in review or approved on it, the finished items wait, except that a team's items that the
///   head of an update contains join it.
/// - An item that is being fixed, when its lead waits, updates its pull request.
///
/// # Errors
/// [`PlanError`] when a name of the team is not valid.
pub(super) fn plan_publications(team: &Team) -> Result<Vec<Publication>, PlanError> {
    let _ = team;
    Ok(Vec::new())
}

/// The title and the description of a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PullText {
    /// The title, on one line.
    pub(super) title: String,
    /// The description.
    pub(super) body: String,
}

/// Why the text of a pull request cannot be made.
#[derive(Debug, thiserror::Error)]
pub(super) enum PullTextError {
    /// There is no item to write about.
    #[error("a pull request is written for at least one item")]
    NoItems,
    /// An item is not in the task list: a mistake of the daemon.
    #[error("item {item} is not in the task list")]
    NoSuchItem {
        /// The item.
        item: ItemId,
    },
    /// A variable name of the program is not valid.
    #[error("a variable of a text is not valid")]
    Variable(#[from] VariablesError),
    /// A text does not take the values it is given.
    #[error("cannot render a text of the pull request")]
    Render(#[from] RenderAutopilotTextError),
}

/// The title and the description of the pull request of `items` of `team`, after the gate said `gate`.
///
/// # Errors
/// [`PullTextError`].
pub(super) fn pull_text(
    texts: &AutopilotPrompts,
    team: &Team,
    repo: &RepoName,
    items: &[ItemId],
    gate: &GateRecord,
) -> Result<PullText, PullTextError> {
    let _ = (texts, team, repo, items, gate, AutopilotPrompt::PrTitle);
    Ok(PullText {
        title: String::new(),
        body: String::new(),
    })
}

#[cfg(test)]
mod tests;
