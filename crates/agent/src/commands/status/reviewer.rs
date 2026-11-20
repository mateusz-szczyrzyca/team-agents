//! What the line of a reviewer says: what it did or does, from the record of the team's activities, instead of the one
//! word of its status, which says `waiting` between its reviews.

use agent_core::kinds::ActivityKind;
use agent_core::protocol::MemberSummary;
use agent_core::state::{Activity, ActivityEnd, ActivityOutcome, ActivityRequest, OpinionFailure, RunEnd, Timestamp};
use time::UtcOffset;

use super::clock_of;

/// What the line of the reviewer `member` says, `None` when the team recorded no activity of it: the activity replaces
/// the line of its status for a reviewer.
pub(crate) fn status(member: &MemberSummary, activities: &[Activity], offset: UtcOffset) -> Option<String> {
    let (place, newest) = newest(member, activities)?;
    Some(match newest.outcome() {
        ActivityOutcome::Working => "working".to_owned(),
        ActivityOutcome::Finished {
            at,
            end: ActivityEnd::Reported,
        } => format!(
            "done, {} review finished {}",
            ordinal_text(ordinal(place, newest, activities)),
            clock_of(*at, offset)
        ),
        ActivityOutcome::Finished { at, end } => stopped(*at, end, offset),
    })
}

/// What an activity that ended at `at` without its report says: when, and why in the words the human reads.
pub(super) fn stopped(at: Timestamp, end: &ActivityEnd, offset: UtcOffset) -> String {
    format!("stopped at {}, reason: {}", clock_of(at, offset), ending(end))
}

/// Where an activity stands in the record of its team, oldest first: by when it was asked, then by the pass of the pull
/// request it is about, then by its place in the record, which keeps the order in which the daemon recorded the
/// activities asked in one second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Place {
    /// When the activity was asked.
    asked_at: Timestamp,
    /// The fix passes of the pull request before it: 0 for a first review and for work that is about no pass.
    pass: u32,
    /// Its index in the team's record.
    index: usize,
}

/// The activities of a team's record, each with its place in it.
pub(super) fn placed(activities: &[Activity]) -> impl Iterator<Item = (Place, &Activity)> {
    activities.iter().enumerate().map(|(index, activity)| {
        let place = Place {
            asked_at: activity.asked_at(),
            pass: pass_of(activity.request()),
            index,
        };
        (place, activity)
    })
}

/// The fix passes of the pull request before the work `request` asks for: a verification or a second opinion of pass
/// `n` comes after the first review and after the passes before `n`.
const fn pass_of(request: &ActivityRequest) -> u32 {
    match request {
        ActivityRequest::TeamMember
        | ActivityRequest::FirstReview
        | ActivityRequest::Investigation
        | ActivityRequest::TaskAllocation => 0,
        ActivityRequest::Verification { pass } | ActivityRequest::SecondOpinion { pass, .. } => pass.get(),
    }
}

/// The newest activity of `member`'s task with its place, or `None` when there is none.
fn newest<'a>(member: &MemberSummary, activities: &'a [Activity]) -> Option<(Place, &'a Activity)> {
    placed(activities)
        .filter(|(_, activity)| activity.task() == &member.task)
        .max_by_key(|(place, _)| *place)
}

/// The place of the review `activity` among the reviews that its investigator gave for one of the rounds it serves,
/// counting from one: the first review, the verifications and the second opinions alike. A review gave its report; one
/// that ended without it is not counted, and `activity`, at `this` place of `activities`, counts whatever its outcome.
pub(super) fn ordinal(this: Place, activity: &Activity, activities: &[Activity]) -> usize {
    let earlier = placed(activities)
        .filter(|(place, _)| *place < this)
        .map(|(_, other)| other)
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
        .count();
    earlier.saturating_add(1)
}

/// What an ended activity that is no report says, in the words the human reads.
fn ending(end: &ActivityEnd) -> String {
    match end {
        ActivityEnd::Reported => "the report was given".to_owned(),
        ActivityEnd::Stopped => "the human stopped it".to_owned(),
        ActivityEnd::Failed { reason } => reason.as_ref().map_or_else(|| "it failed".to_owned(), run_end),
        ActivityEnd::OpinionFailed { reason } => {
            format!("its second opinion was abandoned: {}", opinion_failure(*reason))
        }
    }
}

/// Why a second opinion was abandoned, in the words the human reads.
const fn opinion_failure(reason: OpinionFailure) -> &'static str {
    match reason {
        OpinionFailure::Failed => "the run failed",
        OpinionFailure::NoStatus => "the run ended without a status",
        OpinionFailure::MissingReport => "the corrections ran out without an opinion",
        OpinionFailure::Silent => "the investigator stayed silent too long",
        OpinionFailure::Interrupted => "the task can no longer be resumed",
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
