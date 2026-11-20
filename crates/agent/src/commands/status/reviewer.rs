//! What the line of a reviewer says: what it did or does, from the record of the team's activities, instead of the one
//! word of its status, which says `waiting` between its reviews.

use agent_core::ids::TaskId;
use agent_core::kinds::ActivityKind;
use agent_core::protocol::MemberSummary;
use agent_core::state::{Activity, ActivityEnd, ActivityOutcome, RunEnd, Timestamp};
use time::UtcOffset;

use super::clock_of;

/// What the line of the reviewer `member` says, `None` when the team recorded no activity of it: the activity replaces
/// the line of its status for a reviewer.
pub(crate) fn status(member: &MemberSummary, activities: &[Activity], offset: UtcOffset) -> Option<String> {
    let newest = newest(member, activities)?;
    Some(match newest.outcome() {
        ActivityOutcome::Working => "working".to_owned(),
        ActivityOutcome::Finished {
            at,
            end: ActivityEnd::Reported,
        } => format!(
            "done, {} review finished {}",
            ordinal_text(ordinal(newest, activities)),
            clock_of(*at, offset)
        ),
        ActivityOutcome::Finished { at, end } => stopped(*at, end, offset),
    })
}

/// What an activity that ended at `at` without its report says: when, and why in the words the human reads.
pub(super) fn stopped(at: Timestamp, end: &ActivityEnd, offset: UtcOffset) -> String {
    format!("stopped at {}, reason: {}", clock_of(at, offset), ending(end))
}

/// The newest activity of `member`'s task, or `None` when there is none.
fn newest<'a>(member: &MemberSummary, activities: &'a [Activity]) -> Option<&'a Activity> {
    activities
        .iter()
        .filter(|activity| activity.task() == &member.task)
        .max_by_key(|activity| activity.asked_at())
}

/// The place of the review `activity` among the reviews that its investigator gave for one of the rounds it serves,
/// counting from one: the first review, the verifications and the second opinions alike. A review gave its report; one
/// that ended without it is not counted, and `activity` itself counts whatever its outcome.
pub(super) fn ordinal(activity: &Activity, activities: &[Activity]) -> usize {
    /// Where an activity stands among the reviews: by when it was asked, then by its task.
    fn place(activity: &Activity) -> (Timestamp, &TaskId) {
        (activity.asked_at(), activity.task())
    }
    let this = place(activity);
    let earlier = activities
        .iter()
        .filter(|other| other.investigator() == activity.investigator())
        .filter(|other| matches!(other.request().kind(), ActivityKind::Reviewer))
        .filter(|other| other.rounds().iter().any(|round| activity.rounds().contains(round)))
        .filter(|other| {
            matches!(
                other.outcome(),
                ActivityOutcome::Finished {
                    end: ActivityEnd::Reported,
                    ..
                }
            )
        })
        .filter(|other| place(other) < this)
        .count();
    earlier.saturating_add(1)
}

/// What an ended activity that is no report says, in the words the human reads.
fn ending(end: &ActivityEnd) -> String {
    match end {
        ActivityEnd::Reported => "the report was given".to_owned(),
        ActivityEnd::Stopped => "the human stopped it".to_owned(),
        ActivityEnd::Failed { reason } => reason.as_ref().map_or_else(|| "it failed".to_owned(), run_end),
        ActivityEnd::OpinionFailed { .. } => "its second opinion was abandoned".to_owned(),
    }
}

/// Why the run that failed an assignment did, in the words the human reads.
fn run_end(end: &RunEnd) -> String {
    match end {
        RunEnd::Reported { outcome } => format!("the run reported {outcome}"),
        RunEnd::NoStatus { .. } => "the run ended without a status".to_owned(),
        RunEnd::QuotaExhausted { .. } => "the account's quota was used up".to_owned(),
        RunEnd::RateLimited => "the provider limited the rate of requests".to_owned(),
        RunEnd::Hung => "the run showed no activity for too long".to_owned(),
        RunEnd::TurnLimit => "the run used up its turn budget".to_owned(),
        RunEnd::Stopped => "the run was stopped".to_owned(),
    }
}

/// `n` with its English ordinal suffix: `1st`, `2nd`, `3rd`, `4th`, `11th`, `12th`, `13th`, `21st`.
pub(super) fn ordinal_text(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (0, _) | (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}
