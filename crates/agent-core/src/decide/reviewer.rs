//! Whether the reviewer of a review that is still to come is replaced by another.
//!
//! A review pass waits for every review that was asked. A reviewer that cannot work — its CLI is not logged in, its
//! provider has no quota left, the human stopped it — would keep the pass, and with it the team, waiting for ever.
//! [`replace_now`] says when another investigator takes such a review:
//!
//! 0. The lead of the team is over (stopped or failed): the team does not go on, so nobody is asked for anything.
//! 1. The task failed or was stopped: it will not review, so at once.
//! 2. The task is paused (a quota or a rate limit, which end by themselves, or the human's pause): when it has been
//!    paused for as long as a reviewer may be, counted from its last run that its provider did not refuse
//!    ([`quiet_since`]).
//!    A paused reviewer whose agent the refusal rule changes ([`replace_after_refusal`]) is replaced at once instead, and
//!    one the rule only pauses (its retry is still to come) is kept until the retry.
//! 3. Otherwise not: a task that is queued, runs or asks the human works on the review; one that waits is about to be
//!    given it; one that is done reported, and a report without a verdict is asked for again elsewhere.

use std::collections::BTreeSet;
use std::time::Duration;

use crate::decide::limit::Decision;
use crate::ids::InvestigatorName;
use crate::state::{RunEnd, RunState, Timestamp};
use crate::status::TaskStatus;

/// What is known of a review that is still to come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Awaited {
    /// The status of the reviewer's task.
    pub reviewer: TaskStatus,
    /// The status of the task of the team's lead.
    pub lead: TaskStatus,
    /// How long the reviewer has been quiet: since the end of its last run that its provider did not refuse, or
    /// since the review was asked when it has no such run after the request ([`quiet_since`]).
    pub quiet_for: Duration,
    /// How long a reviewer may be paused.
    pub patience: Duration,
}

/// Whether the review `awaited` describes is given to another investigator.
#[must_use]
pub fn replace_now(awaited: &Awaited) -> bool {
    if matches!(awaited.lead, TaskStatus::Stopped | TaskStatus::Failed) {
        return false;
    }
    let (status, quiet_for, patience) = (awaited.reviewer, awaited.quiet_for, awaited.patience);
    match status {
        TaskStatus::Failed | TaskStatus::Stopped => true,
        TaskStatus::Paused => quiet_for >= patience,
        TaskStatus::Queued | TaskStatus::Running | TaskStatus::Questions | TaskStatus::Waiting | TaskStatus::Done => {
            false
        }
    }
}

/// Whether the review `awaited` describes is given to another investigator, when the reviewer's last run was refused by
/// its provider and the refusal rule (`refusal`, from [`crate::decide::limit::decide`]) answered for it; `None` when the
/// last run was not refused. A paused reviewer whose agent the rule changes is replaced at once, without the patience;
/// one the rule pauses is kept until its retry, whatever the silence, since its pause is the provider's to end and not
/// the patience's; every other case is [`replace_now`].
#[must_use]
pub fn replace_after_refusal(awaited: &Awaited, refusal: Option<Decision>) -> bool {
    let lead_over = matches!(awaited.lead, TaskStatus::Stopped | TaskStatus::Failed);
    let paused = awaited.reviewer == TaskStatus::Paused && !lead_over;
    match refusal {
        Some(Decision::ChangeAgent) if paused => true,
        Some(Decision::Pause) if paused => false,
        Some(Decision::ChangeAgent | Decision::Pause) | None => replace_now(awaited),
    }
}

/// An investigator and the provider it runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Provided<'a> {
    /// The investigator.
    pub name: &'a InvestigatorName,
    /// Its provider, as [`crate::decide::pool::provider_of`] names it.
    pub provider: &'a str,
}

/// The investigators of `investigators` that run on `provider`: a refusal replaces a reviewer by an investigator of
/// another provider, since one of the same provider meets the same refusal.
#[must_use]
pub fn on_provider(provider: &str, investigators: &[Provided<'_>]) -> BTreeSet<InvestigatorName> {
    investigators
        .iter()
        .filter(|investigator| investigator.provider == provider)
        .map(|investigator| investigator.name.clone())
        .collect()
}

/// When the reviewer of a review asked at `requested_at` last did anything but get refused by its provider, given its
/// runs `runs`, oldest first: the end of its last run that its provider did not refuse, never before `requested_at`;
/// `requested_at` when there is none. A run that is going counts from the request, as before: a reviewer that runs is
/// never replaced anyway.
///
/// A refused run does not end the quiet time: a provider that answers every run with a short rate limit would
/// otherwise keep the review waiting for ever (issue #234).
#[must_use]
pub fn quiet_since<'a>(runs: impl DoubleEndedIterator<Item = &'a RunState>, requested_at: Timestamp) -> Timestamp {
    runs.rev()
        .find_map(|run| match run {
            RunState::Ended { at, end } => (!refused_by_provider(end)).then_some(*at),
            RunState::Running => Some(requested_at),
        })
        .map_or(requested_at, |at| at.max(requested_at))
}

/// Whether a run that ended as `end` was refused by its provider instead of doing work.
const fn refused_by_provider(end: &RunEnd) -> bool {
    match end {
        RunEnd::RateLimited | RunEnd::QuotaExhausted { .. } => true,
        RunEnd::Reported { .. } | RunEnd::NoStatus { .. } | RunEnd::Hung | RunEnd::TurnLimit | RunEnd::Stopped => false,
    }
}

#[cfg(test)]
mod tests;
