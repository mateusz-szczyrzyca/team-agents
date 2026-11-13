//! The round of a pull request after it is open: where it is, what a review's verdict leads to, what the team and the
//! reviewers are waiting to be told, and the merge.
//!
//! The round machine of `agent_core::decide::round` decides; nothing is kept of it besides the state of the items. Where
//! a round is, is worked out again from the items' phases, the reviews they carry and the runs and questions of the
//! tasks, so that a daemon that restarts goes on where it was. The owner carries out what is decided
//! (`owner/rounds.rs`).

use std::num::NonZeroU32;

use agent_core::decide::round::{RoundAction, RoundError, RoundPolicy, RoundState, Step};
use agent_core::ids::{InvestigatorName, ItemId, TaskId};
use agent_core::state::{ItemProgress, Member, PhaseKind, PrNumber, PullRequest, Team, Timestamp};
use agent_core::status::ReviewVerdict;

/// What follows a verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Next {
    /// The other review is not in yet.
    Wait,
    /// Both approve: the harness merges the pull request.
    Merge,
    /// Both approve and the team does not merge by itself: the human is told.
    HandToHuman,
    /// A reviewer asked for changes: the lead gets a fix pass.
    FixPass {
        /// Which fix pass this is, counted from 1.
        pass: NonZeroU32,
    },
    /// A reviewer asked for changes and the fix passes are used up: the human is asked.
    AskHuman,
}

/// The round machine answered with an action that a verdict cannot lead to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a verdict cannot lead to the action {action:?}")]
pub(super) struct Unexpected {
    /// The action.
    pub(super) action: RoundAction,
}

/// Why a verdict cannot be applied to an item.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(super) enum VerdictError {
    /// The item is not in review.
    #[error("item {item} is {phase}: only an item in review is given a verdict")]
    NotInReview {
        /// The item.
        item: ItemId,
        /// Its phase.
        phase: PhaseKind,
    },
    /// The round does not take the verdict.
    #[error("the round does not take the verdict")]
    Round(#[from] RoundError),
    /// The round answered with an action a verdict does not lead to.
    #[error("the round answered with an unexpected action")]
    Unexpected(#[from] Unexpected),
}

/// The items of `team` that share the pull request `pr` and are in review, being fixed or approved, in the order of
/// the list.
pub(super) fn items_of(team: &Team, pr: PrNumber) -> Vec<ItemId> {
    let _ = (team, pr);
    Vec::new()
}

/// Where the round of `progress` is, as the round machine knows it; `None` for an item that has no pull request or is
/// done.
pub(super) fn round_state(progress: &ItemProgress) -> Option<RoundState> {
    let _ = progress;
    None
}

/// What the action of the round machine means for the daemon after a verdict.
///
/// # Errors
/// [`Unexpected`] for an action that a verdict does not lead to.
pub(super) fn next_of(step: &Step) -> Result<Next, Unexpected> {
    let _ = step;
    Ok(Next::Wait)
}

/// What follows the verdict `verdict` of `reviewer` on the item `progress`, which is in review.
///
/// # Errors
/// [`VerdictError`].
pub(super) fn after_verdict(
    progress: &ItemProgress,
    reviewer: &InvestigatorName,
    verdict: ReviewVerdict,
    policy: RoundPolicy,
) -> Result<Next, VerdictError> {
    let _ = (progress, reviewer, verdict, policy);
    Ok(Next::Wait)
}

/// A pull request whose reviewers still ask for changes when the fix passes are used up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Exhausted {
    /// The pull request.
    pub(super) pr: PullRequest,
    /// Its items, in the order of the list.
    pub(super) items: Vec<ItemId>,
    /// The fix passes it has had.
    pub(super) passes: u32,
    /// When the last request for changes came.
    pub(super) requested_at: Timestamp,
}

/// The pull requests of `team` that wait for the human: an item in review that a reviewer sent back.
pub(super) fn exhausted(team: &Team) -> Vec<Exhausted> {
    let _ = team;
    Vec::new()
}

/// What the human has said to a question put since `since`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Decision {
    /// No question was put since.
    Ask,
    /// A question was put and is open.
    Waiting,
    /// A question was put and is answered.
    Answered,
}

/// The state of the human's decision in the questions of `lead` since `since`.
pub(super) fn decision(lead: &Member, since: Timestamp) -> Decision {
    let _ = (lead, since);
    Decision::Waiting
}

/// The task of the lead of `team` when a fix pass waits to be given to it: an item is being fixed, the lead waits and
/// has not started a run since the request for changes.
pub(super) fn fix_delivery(team: &Team) -> Option<TaskId> {
    let _ = team;
    None
}

/// The review tasks of `team` that wait for a review to be asked of them: a review is pending, the task waits and has
/// not started a run since the review was asked.
pub(super) fn review_deliveries(team: &Team) -> Vec<TaskId> {
    let _ = team;
    Vec::new()
}

/// What is done about a pull request that waits for the human and has no question yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Asking {
    /// The lead is woken to write the question.
    Wake {
        /// The lead's task.
        lead: TaskId,
        /// The pull request.
        exhausted: Exhausted,
    },
    /// The lead ran since the request for changes and put no question: the human is told directly.
    Notify {
        /// The pull request.
        exhausted: Exhausted,
    },
}

/// What is done about the pull requests of `team` that wait for the human.
pub(super) fn asking(team: &Team) -> Vec<Asking> {
    let _ = team;
    Vec::new()
}

#[cfg(test)]
mod tests;
