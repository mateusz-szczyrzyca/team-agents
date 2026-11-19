//! Where the work of an open round is, as `agent status` tells it: on the machine only, in a draft pull request, in a
//! pull request that waits for its reviewers, in review, or approved.
//!
//! The state keeps the phase of the round's item and its pull request once the reviews are asked for. It does not keep
//! the draft the harness opens before its checks let the round go to the reviewers: that is in the history, as a pull
//! request that was opened and not yet marked ready ([`draft_pull`]).

use crate::history::HistoryEvent;
use crate::kinds::Role;
use crate::protocol::RoundDetail;
use crate::state::{Review, ReviewOutcome, RoundLife};
use crate::status::ItemStatus;

#[cfg(test)]
mod tests;

/// Where the work of a round that goes on is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display)]
pub enum RoundWork {
    /// The work is in the worktrees only: no pull request is open.
    #[strum(serialize = "only local")]
    OnlyLocal,
    /// The pull request is open as a draft: the checks before the review hold it.
    #[strum(serialize = "PR draft")]
    PullDraft,
    /// No reviews are asked for yet and the history cannot say whether a draft is open.
    #[strum(serialize = "only local or PR draft")]
    LocalOrDraft,
    /// The pull request is ready for review and no reviewer has started.
    #[strum(serialize = "PR ready")]
    PullReady,
    /// A reviewer works or has reported, or the team fixes what the reviewers asked for.
    #[strum(serialize = "PR in review")]
    PullInReview,
    /// A requested independent opinion has not arrived.
    #[strum(serialize = "waiting for a second opinion")]
    SecondOpinion,
    /// The reviews approve: the pull request waits for its merge.
    #[strum(serialize = "PR approved")]
    PullApproved,
}

/// Whether the history says that a draft pull request of a round is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DraftPull {
    /// No pull request was opened, or the one that was is ready, merged, handed over or closed.
    Absent,
    /// A pull request was opened and nothing followed that takes it out of draft.
    Open,
    /// The history could not be read.
    Unknown,
}

/// Whether `events`, the events of one round in the order they were written, leave a draft pull request open.
#[must_use]
pub fn draft_pull<'a>(events: impl IntoIterator<Item = &'a HistoryEvent>) -> DraftPull {
    events.into_iter().fold(DraftPull::Absent, |draft, event| match event {
        HistoryEvent::PullOpened { .. } => DraftPull::Open,
        HistoryEvent::PullReady { .. }
        | HistoryEvent::PullApproved { .. }
        | HistoryEvent::PullMerged { .. }
        | HistoryEvent::PullHandedOver { .. }
        | HistoryEvent::PullClosed { .. } => DraftPull::Absent,
        HistoryEvent::Gate { .. }
        | HistoryEvent::Checked { .. }
        | HistoryEvent::PullPushed { .. }
        | HistoryEvent::ReviewAsked { .. }
        | HistoryEvent::ReviewSubmitted { .. }
        | HistoryEvent::FindingExplained { .. }
        | HistoryEvent::FindingAccepted { .. }
        | HistoryEvent::FindingRejected { .. }
        | HistoryEvent::FindingRaised { .. }
        | HistoryEvent::SecondOpinionAsked { .. }
        | HistoryEvent::SecondOpinionEnded { .. }
        | HistoryEvent::FindingConfirmed { .. }
        | HistoryEvent::FindingNotConfirmed { .. }
        | HistoryEvent::FixPassStarted { .. }
        | HistoryEvent::QuestionAsked { .. }
        | HistoryEvent::QuestionAnswered { .. }
        | HistoryEvent::Controlled { .. }
        | HistoryEvent::Notified { .. }
        | HistoryEvent::RoundClosed { .. }
        | HistoryEvent::TeamEnded {}
        | HistoryEvent::Lost { .. } => draft,
    })
}

/// Where the work of the round of `detail` is, given what the history says of its draft; `None` for a round that is
/// closed or whose item is done: nothing is worked on.
#[must_use]
pub fn round_work(detail: &RoundDetail, draft: DraftPull) -> Option<RoundWork> {
    if *detail.round.life() != RoundLife::Open {
        return None;
    }
    if detail.opinions.values().any(|opinion| {
        matches!(
            opinion.stage,
            crate::state::OpinionStage::Waiting | crate::state::OpinionStage::Assigned { .. }
        )
    }) {
        return Some(RoundWork::SecondOpinion);
    }
    match detail.item.as_ref().map(|item| item.status) {
        Some(ItemStatus::Done) => None,
        Some(ItemStatus::Approved) => Some(RoundWork::PullApproved),
        Some(ItemStatus::InReview) if review_started(detail) => Some(RoundWork::PullInReview),
        Some(ItemStatus::InReview) => Some(RoundWork::PullReady),
        // An item in progress that has a pull request is in a fix pass.
        None | Some(ItemStatus::Pending | ItemStatus::InProgress) => Some(match (&detail.pr, draft) {
            (Some(_), _) => RoundWork::PullInReview,
            (None, DraftPull::Absent) => RoundWork::OnlyLocal,
            (None, DraftPull::Open) => RoundWork::PullDraft,
            (None, DraftPull::Unknown) => RoundWork::LocalOrDraft,
        }),
    }
}

/// Whether the review of the pull request of `detail` has begun: a fix pass is behind it, a verdict is in, or a
/// reviewer has run for the round.
fn review_started(detail: &RoundDetail) -> bool {
    let reported = |review: &Review| match review.outcome {
        ReviewOutcome::Submitted { .. } => true,
        ReviewOutcome::Pending => false,
    };
    detail.review_passes.len() > 1
        || detail.review_passes.iter().flat_map(|pass| &pass.reviews).any(reported)
        || detail
            .tasks
            .iter()
            .any(|task| task.role == Role::Reviewer && !task.runs.is_empty())
}
