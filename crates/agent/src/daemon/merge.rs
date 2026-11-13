//! The round of a pull request after it is open: where it is, what a review's verdict leads to, what the team and the
//! reviewers are waiting to be told, and the merge.
//!
//! The round machine of `agent_core::decide::round` decides; nothing is kept of it besides the state of the items. Where
//! a round is, is worked out again from the items' phases, the reviews they carry and the runs and questions of the
//! tasks, so that a daemon that restarts goes on where it was. The owner carries out what is decided
//! (`owner/rounds.rs`).

use std::num::NonZeroU32;

use agent_core::decide::round::{RoundAction, RoundError, RoundEvent, RoundPolicy, RoundState, Stage, Step};
use agent_core::ids::{InvestigatorName, ItemId, TaskId};
use agent_core::state::{
    ItemPhase, ItemProgress, Member, PhaseKind, PrNumber, PullRequest, Review, ReviewOutcome, Team, Timestamp,
};
use agent_core::status::{ReviewVerdict, TaskStatus};

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
    team.items()
        .iter()
        .filter(|progress| match progress.phase() {
            ItemPhase::InReview { pr: open, .. }
            | ItemPhase::Fixing { pr: open, .. }
            | ItemPhase::Approved { pr: open, .. } => open.number == pr,
            ItemPhase::Pending | ItemPhase::InProgress { .. } | ItemPhase::Done { .. } => false,
        })
        .map(ItemProgress::id)
        .collect()
}

/// Where the round of `progress` is, as the round machine knows it; `None` for an item that has no pull request or is
/// done.
pub(super) fn round_state(progress: &ItemProgress) -> Option<RoundState> {
    let passes = count(progress.fix_passes().len());
    match progress.phase() {
        ItemPhase::InReview { pr, reviews } => Some(if reviews.iter().any(asked_for_changes) {
            RoundState::AskingHuman { pr: pr.number, passes }
        } else {
            RoundState::InReview {
                pr: pr.number,
                passes,
                approvals: reviews
                    .iter()
                    .filter(|review| approved(review))
                    .map(|review| review.reviewer.clone())
                    .collect(),
            }
        }),
        ItemPhase::Approved { pr, .. } => Some(RoundState::Approved { pr: pr.number }),
        ItemPhase::Fixing { pr, .. } => Some(RoundState::MemberWork {
            stage: Stage::Fixing {
                pr: pr.number,
                // A fixing item has a fix pass behind it, which reading the state checks: the pass is the last one.
                pass: NonZeroU32::MIN.saturating_add(passes.saturating_sub(1)),
            },
        }),
        ItemPhase::Pending | ItemPhase::InProgress { .. } | ItemPhase::Done { .. } => None,
    }
}

/// A number of fix passes as the round machine counts them; a count that does not fit saturates, which no item reaches:
/// every fix pass is a review pass.
pub(super) fn count(passes: usize) -> u32 {
    u32::try_from(passes).unwrap_or(u32::MAX)
}

/// Whether `review` is a submitted approval.
fn approved(review: &Review) -> bool {
    matches!(
        review.outcome,
        ReviewOutcome::Submitted {
            verdict: ReviewVerdict::Approve,
            ..
        }
    )
}

/// When `review` asked for changes; `None` for any other review.
fn changes_requested_at(review: &Review) -> Option<Timestamp> {
    match review.outcome {
        ReviewOutcome::Submitted {
            verdict: ReviewVerdict::RequestChanges,
            at,
        } => Some(at),
        ReviewOutcome::Submitted {
            verdict: ReviewVerdict::Approve,
            ..
        }
        | ReviewOutcome::Pending => None,
    }
}

/// Whether `review` is a submitted request for changes.
fn asked_for_changes(review: &Review) -> bool {
    changes_requested_at(review).is_some()
}

/// What the action of the round machine means for the daemon after a verdict.
///
/// # Errors
/// [`Unexpected`] for an action that a verdict does not lead to.
pub(super) fn next_of(step: &Step) -> Result<Next, Unexpected> {
    match step.action {
        RoundAction::AwaitReviews => Ok(Next::Wait),
        RoundAction::Merge => Ok(Next::Merge),
        RoundAction::HandToHuman => Ok(Next::HandToHuman),
        RoundAction::StartFixPass { pass } => Ok(Next::FixPass { pass }),
        RoundAction::AskHuman => Ok(Next::AskHuman),
        action @ (RoundAction::IntegrateBranch
        | RoundAction::FixGate
        | RoundAction::PublishPullRequest
        | RoundAction::RequestReviews
        | RoundAction::CompleteItem) => Err(Unexpected { action }),
    }
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
    let phase = progress.phase().kind();
    let state = round_state(progress)
        .filter(|_| phase == PhaseKind::InReview)
        .ok_or(VerdictError::NotInReview {
            item: progress.id(),
            phase,
        })?;
    let event = RoundEvent::ReviewSubmitted {
        reviewer: reviewer.clone(),
        verdict,
    };
    Ok(next_of(&state.transition(&event, policy)?)?)
}

/// What follows the human's answer about the item `progress`, which waits for it after a request for changes that the
/// fix passes could not take: one more fix pass.
///
/// # Errors
/// [`VerdictError`].
pub(super) fn after_answer(progress: &ItemProgress, policy: RoundPolicy) -> Result<Next, VerdictError> {
    let phase = progress.phase().kind();
    let state = round_state(progress)
        .filter(|_| phase == PhaseKind::InReview)
        .ok_or(VerdictError::NotInReview {
            item: progress.id(),
            phase,
        })?;
    Ok(next_of(&state.transition(&RoundEvent::HumanAnswered, policy)?)?)
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
    let mut found: Vec<Exhausted> = Vec::new();
    for progress in team.items() {
        let ItemPhase::InReview { pr, reviews } = progress.phase() else {
            continue;
        };
        let Some(requested_at) = reviews.iter().filter_map(changes_requested_at).max() else {
            continue;
        };
        match found.iter_mut().find(|known| known.pr.number == pr.number) {
            Some(known) => {
                known.items.push(progress.id());
                known.requested_at = known.requested_at.max(requested_at);
            }
            None => found.push(Exhausted {
                pr: pr.clone(),
                items: vec![progress.id()],
                passes: count(progress.fix_passes().len()),
                requested_at,
            }),
        }
    }
    found
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
    match lead
        .questions()
        .iter()
        .rev()
        .find(|question| question.asked_at() >= since)
    {
        None => Decision::Ask,
        Some(question) if question.is_open() => Decision::Waiting,
        Some(_) => Decision::Answered,
    }
}

/// Whether `member` started a run at or after `time`: what it was told then was told in that run.
fn ran_since(member: &Member, time: Timestamp) -> bool {
    member.runs().last().is_some_and(|run| run.started_at() >= time)
}

/// The task of the lead of `team` when a fix pass waits to be given to it: an item is being fixed, the lead waits and
/// has not started a run since the request for changes.
pub(super) fn fix_delivery(team: &Team) -> Option<TaskId> {
    let roster = team.crew().roster();
    let (_, lead) = roster.first()?;
    if lead.status() != TaskStatus::Waiting {
        return None;
    }
    let requested = team
        .items()
        .iter()
        .filter_map(|progress| match progress.phase() {
            ItemPhase::Fixing { assignee, .. } if assignee.as_str() == lead.agent().name() => progress
                .fix_passes()
                .last()
                .and_then(|pass| pass.requested_by.iter().filter_map(changes_requested_at).max()),
            ItemPhase::Fixing { .. }
            | ItemPhase::Pending
            | ItemPhase::InProgress { .. }
            | ItemPhase::InReview { .. }
            | ItemPhase::Approved { .. }
            | ItemPhase::Done { .. } => None,
        })
        .max()?;
    (!ran_since(lead, requested)).then(|| lead.task().clone())
}

/// The review tasks of `team` that wait for a review to be asked of them: a review is pending, the task waits and has
/// not started a run since the review was asked.
pub(super) fn review_deliveries(team: &Team) -> Vec<TaskId> {
    team.reviewers()
        .iter()
        .filter(|member| member.status() == TaskStatus::Waiting)
        .filter(|member| asked_at(team, member.task()).is_some_and(|asked| !ran_since(member, asked)))
        .map(|member| member.task().clone())
        .collect()
}

/// When the review of the task `task` that is pending was asked; the latest, for a task asked in several items.
fn asked_at(team: &Team, task: &TaskId) -> Option<Timestamp> {
    team.items()
        .iter()
        .filter_map(|progress| match progress.phase() {
            ItemPhase::InReview { reviews, .. } => reviews
                .iter()
                .filter(|review| review.task == *task && matches!(review.outcome, ReviewOutcome::Pending))
                .map(|review| review.requested_at)
                .max(),
            ItemPhase::Pending
            | ItemPhase::InProgress { .. }
            | ItemPhase::Fixing { .. }
            | ItemPhase::Approved { .. }
            | ItemPhase::Done { .. } => None,
        })
        .max()
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
    let roster = team.crew().roster();
    let Some((_, lead)) = roster.first() else {
        return Vec::new();
    };
    if lead.status() != TaskStatus::Waiting {
        return Vec::new();
    }
    exhausted(team)
        .into_iter()
        .filter(|waiting| decision(lead, waiting.requested_at) == Decision::Ask)
        .map(|waiting| {
            if ran_since(lead, waiting.requested_at) {
                Asking::Notify { exhausted: waiting }
            } else {
                Asking::Wake {
                    lead: lead.task().clone(),
                    exhausted: waiting,
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
