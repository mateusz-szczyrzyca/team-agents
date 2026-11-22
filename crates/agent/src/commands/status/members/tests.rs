//! Which tasks the short view of `agent status` lists.

use agent_core::kinds::Role;
use agent_core::state::{ActivityEnd, ActivityOutcome, OpinionFailure, RunEnd};
use agent_core::status::TaskStatus;

use super::alive;
use crate::fixtures::at;

/// The outcome of an activity that ended as `end`.
fn finished(end: ActivityEnd) -> ActivityOutcome {
    ActivityOutcome::Finished { at: at(30), end }
}

/// What the record of a team can say of a task's newest activity, each with its name for a failed case: nothing, that
/// the work goes on, and each way it ended.
fn reviews() -> Vec<(&'static str, Option<ActivityOutcome>)> {
    vec![
        ("no activity recorded", None),
        ("working", Some(ActivityOutcome::Working)),
        ("reported", Some(finished(ActivityEnd::Reported))),
        ("stopped by the human", Some(finished(ActivityEnd::Stopped))),
        ("failed", Some(finished(ActivityEnd::Failed { reason: None }))),
        (
            "failed with the end of its run",
            Some(finished(ActivityEnd::Failed {
                reason: Some(RunEnd::Hung),
            })),
        ),
        (
            "second opinion abandoned",
            Some(finished(ActivityEnd::OpinionFailed {
                reason: OpinionFailure::Silent,
            })),
        ),
    ]
}

// Rule: the human wants to see the team: its coordinator, its members, its workers and a solo task are listed whatever
// their status is and whatever the record says of them — also stopped, failed and done.
#[test]
fn the_crew_a_worker_and_a_solo_task_are_listed_in_every_status() {
    for role in [Role::Solo, Role::Coordinator, Role::Member, Role::Worker] {
        for status in TaskStatus::ALL {
            for (name, review) in reviews() {
                assert!(alive(role, *status, review.as_ref()), "{role} {status}, {name}");
            }
        }
    }
}

// Rule: a reviewer is listed while its review is not over. A task that runs is always listed. One that is done or
// stopped is not. Otherwise the newest activity the team recorded of it decides: none yet, one that goes on and one
// that failed (the human may resume it) are listed; one that gave its report, one the human stopped and a second opinion
// that was abandoned are over.
#[test]
fn a_reviewer_is_listed_only_while_its_review_is_not_over() {
    let unfinished = [
        TaskStatus::Running,
        TaskStatus::Waiting,
        TaskStatus::Questions,
        TaskStatus::Queued,
        TaskStatus::Paused,
        TaskStatus::Failed,
    ];
    let only_running = [TaskStatus::Running];
    let table: Vec<(Option<ActivityOutcome>, &[TaskStatus])> = vec![
        (None, &unfinished),
        (Some(ActivityOutcome::Working), &unfinished),
        (Some(finished(ActivityEnd::Failed { reason: None })), &unfinished),
        (
            Some(finished(ActivityEnd::Failed {
                reason: Some(RunEnd::Hung),
            })),
            &unfinished,
        ),
        (Some(finished(ActivityEnd::Reported)), &only_running),
        (Some(finished(ActivityEnd::Stopped)), &only_running),
        (
            Some(finished(ActivityEnd::OpinionFailed {
                reason: OpinionFailure::Silent,
            })),
            &only_running,
        ),
    ];
    // Every outcome the record can hold is a row of the table.
    assert_eq!(table.len(), reviews().len());

    for (review, listed) in table {
        for status in TaskStatus::ALL {
            assert_eq!(
                alive(Role::Reviewer, *status, review.as_ref()),
                listed.contains(status),
                "reviewer {status}, {review:?}"
            );
        }
    }
}
