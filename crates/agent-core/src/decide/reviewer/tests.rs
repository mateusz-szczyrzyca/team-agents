use super::*;

const PATIENCE: Duration = Duration::from_mins(15);

/// A review whose reviewer is `reviewer` and whose last run ended `quiet_for` ago, in a team that goes on.
fn awaited(reviewer: TaskStatus, quiet_for: Duration) -> Awaited {
    Awaited {
        reviewer,
        lead: TaskStatus::Waiting,
        quiet_for,
        patience: PATIENCE,
    }
}

// Rule: a reviewer whose task failed or was stopped will not review: it is replaced at once.
#[test]
fn a_reviewer_that_failed_or_was_stopped_is_replaced_at_once() {
    for status in [TaskStatus::Failed, TaskStatus::Stopped] {
        assert!(replace_now(&awaited(status, Duration::ZERO)), "{status:?}");
    }
}

// Rule: a paused reviewer is given the time a reviewer may be paused, and replaced when it is over.
#[test]
fn a_paused_reviewer_is_replaced_when_it_was_paused_as_long_as_it_may_be() {
    let almost = PATIENCE.saturating_sub(Duration::from_secs(1));

    assert!(!replace_now(&awaited(TaskStatus::Paused, Duration::ZERO)));
    assert!(!replace_now(&awaited(TaskStatus::Paused, almost)));
    assert!(replace_now(&awaited(TaskStatus::Paused, PATIENCE)));
    assert!(replace_now(&awaited(TaskStatus::Paused, Duration::from_hours(8))));
}

// Rule: a reviewer that works on its review, is about to be given it or has reported is not replaced, however long
// it takes.
#[test]
fn a_reviewer_that_works_waits_or_reported_is_not_replaced() {
    for status in [
        TaskStatus::Queued,
        TaskStatus::Running,
        TaskStatus::Questions,
        TaskStatus::Waiting,
        TaskStatus::Done,
    ] {
        assert!(!replace_now(&awaited(status, Duration::from_hours(8))), "{status:?}");
    }
}

// Rule: a team whose lead was stopped or failed does not go on: its reviewers are not replaced, so that stopping a team
// starts nobody.
#[test]
fn the_reviewers_of_a_team_whose_lead_is_over_are_not_replaced() {
    for lead in [TaskStatus::Stopped, TaskStatus::Failed] {
        for reviewer in [TaskStatus::Stopped, TaskStatus::Failed, TaskStatus::Paused] {
            let review = Awaited {
                lead,
                ..awaited(reviewer, Duration::from_hours(8))
            };
            assert!(!replace_now(&review), "lead {lead:?}, reviewer {reviewer:?}");
        }
    }
    // A lead that is done handed its work over: the review of it still has to come.
    let review = Awaited {
        lead: TaskStatus::Done,
        ..awaited(TaskStatus::Failed, Duration::ZERO)
    };
    assert!(replace_now(&review));
}
