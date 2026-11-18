//! Where the work of an open round is, as `agent status` tells it: on the machine only, in a draft pull request, in a
//! pull request that waits for its reviewers, in review, or approved.
//!
//! The state keeps the phase of the round's item and its pull request once the reviews are asked for. It does not keep
//! the draft the harness opens before its checks let the round go to the reviewers: that is in the history, as a pull
//! request that was opened and not yet marked ready ([`draft_pull`]).

use crate::history::HistoryEvent;
use crate::protocol::RoundDetail;

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
pub fn draft_pull<'a>(_events: impl IntoIterator<Item = &'a HistoryEvent>) -> DraftPull {
    DraftPull::Unknown
}

/// Where the work of the round of `detail` is, given what the history says of its draft; `None` for a round that is
/// closed or whose item is done: nothing is worked on.
#[must_use]
pub const fn round_work(_detail: &RoundDetail, _draft: DraftPull) -> Option<RoundWork> {
    None
}
