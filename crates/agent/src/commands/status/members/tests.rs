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
            for (name, _) in reviews() {
                assert!(alive(role), "{role} {status}, {name}");
            }
        }
    }
}

// Rule: a reviewer is listed like the rest of the team: whatever its status is and whatever the newest activity the team
// recorded of it says — none yet, one that goes on, one that gave its report, one the human stopped, one that failed
// and a second opinion that was abandoned. The decision reads neither, and this pins that it never does again.
#[test]
fn a_reviewer_is_listed_in_every_status_and_after_every_review_outcome() {
    for status in TaskStatus::ALL {
        for (name, _) in reviews() {
            assert!(alive(Role::Reviewer), "reviewer {status}, {name}");
        }
    }
}
