use super::*;

const MAY_SLEEP: Duration = Duration::from_mins(5);

/// A team of a coordinator and a member that both sleep, with an item unfinished, for longer than it may.
fn asleep(tasks: &[TaskStatus]) -> Team<'_> {
    Team {
        unfinished: true,
        tasks,
        lead_done: false,
        standing_item: false,
        quiet_for: Duration::from_secs(301),
        may_sleep: MAY_SLEEP,
        woken: 0,
        wake_limit: 2,
    }
}

// Rule: every task sleeps, an item is unfinished and nothing will wake anybody: the lead is woken.
#[test]
fn a_team_whose_every_task_sleeps_with_an_item_unfinished_has_its_lead_woken() {
    let tasks = [TaskStatus::Waiting, TaskStatus::Waiting];

    assert_eq!(asleep_step(&asleep(&tasks)), AsleepStep::WakeLead);
    // A solo agent that sleeps on its own item is a team of one.
    assert_eq!(asleep_step(&asleep(&tasks[..1])), AsleepStep::WakeLead);
}

// Rule: a team with nothing unfinished is not woken.
#[test]
fn a_team_with_nothing_unfinished_is_left_alone() {
    let tasks = [TaskStatus::Waiting, TaskStatus::Waiting];
    let team = Team {
        unfinished: false,
        ..asleep(&tasks)
    };

    assert_eq!(asleep_step(&team), AsleepStep::Nothing);
}

// Rule: a task that is queued, runs, asks the human or is paused means somebody works or will.
#[test]
fn a_task_that_does_not_wait_means_the_team_is_not_asleep() {
    for busy in [
        TaskStatus::Queued,
        TaskStatus::Running,
        TaskStatus::Questions,
        TaskStatus::Paused,
    ] {
        let tasks = [TaskStatus::Waiting, busy];

        assert_eq!(asleep_step(&asleep(&tasks)), AsleepStep::Nothing, "{busy:?}");
    }
}

// Rule: a task that is over (done, failed, stopped) does not work and keeps nobody awake.
#[test]
fn a_task_that_is_over_keeps_nobody_awake() {
    for over in [TaskStatus::Done, TaskStatus::Failed, TaskStatus::Stopped] {
        let tasks = [TaskStatus::Waiting, over];

        assert_eq!(asleep_step(&asleep(&tasks)), AsleepStep::WakeLead, "{over:?}");
    }
}

// Rule: a lead that is over cannot be woken: the team is not the harness's to move.
#[test]
fn a_lead_that_is_over_is_not_woken() {
    for over in [TaskStatus::Done, TaskStatus::Failed, TaskStatus::Stopped] {
        let tasks = [over, TaskStatus::Waiting];

        assert_eq!(asleep_step(&asleep(&tasks)), AsleepStep::Nothing, "{over:?}");
    }
}

// Rule: a lead that ended with DONE handed its branch to the harness: that is not sleep.
#[test]
fn a_lead_that_is_done_is_not_asleep() {
    let tasks = [TaskStatus::Waiting, TaskStatus::Waiting];
    let team = Team {
        lead_done: true,
        ..asleep(&tasks)
    };

    assert_eq!(asleep_step(&team), AsleepStep::Nothing);
}

// Rule: a team is given the time it may sleep before anybody is woken: a message may still be on its way.
#[test]
fn a_team_is_given_the_time_it_may_sleep() {
    let tasks = [TaskStatus::Waiting, TaskStatus::Waiting];
    let just_slept = Team {
        quiet_for: MAY_SLEEP.saturating_sub(Duration::from_secs(1)),
        ..asleep(&tasks)
    };
    let long_enough = Team {
        quiet_for: MAY_SLEEP,
        ..asleep(&tasks)
    };

    assert_eq!(asleep_step(&just_slept), AsleepStep::Nothing);
    assert_eq!(asleep_step(&long_enough), AsleepStep::WakeLead);
}

// Rule: the lead is woken as often as the limit says; then the human is told.
#[test]
fn after_the_lead_was_woken_as_often_as_it_is_the_human_is_told() {
    let tasks = [TaskStatus::Waiting, TaskStatus::Waiting];
    let steps: Vec<AsleepStep> = (0..4)
        .map(|woken| {
            asleep_step(&Team {
                woken,
                ..asleep(&tasks)
            })
        })
        .collect();

    assert_eq!(
        steps,
        [
            AsleepStep::WakeLead,
            AsleepStep::WakeLead,
            AsleepStep::TellHuman,
            AsleepStep::TellHuman
        ]
    );
}

// Rule: a lead that is done with a standing item tells the human immediately, without waiting for may_sleep.
#[test]
fn a_team_whose_lead_is_done_and_has_a_standing_item_tells_the_human_immediately() {
    let tasks = [TaskStatus::Waiting, TaskStatus::Failed];
    let team = Team {
        lead_done: true,
        standing_item: true,
        quiet_for: Duration::ZERO,
        ..asleep(&tasks)
    };

    assert_eq!(asleep_step(&team), AsleepStep::TellHuman);
}

// Rule: pure table testing combinations of lead done, standing item, and crew activity.
#[test]
fn pure_c_table_standing_item_decisions() {
    let all_statuses = [
        TaskStatus::Waiting,
        TaskStatus::Running,
        TaskStatus::Queued,
        TaskStatus::Questions,
        TaskStatus::Paused,
        TaskStatus::Done,
        TaskStatus::Failed,
        TaskStatus::Stopped,
    ];

    for lead_status in all_statuses {
        for lead_done in [false, true] {
            for standing_item in [false, true] {
                for busy_member in [false, true] {
                    let member_status = if busy_member {
                        TaskStatus::Running
                    } else {
                        TaskStatus::Failed
                    };
                    let tasks = [lead_status, member_status];
                    let team = Team {
                        unfinished: true,
                        tasks: &tasks,
                        lead_done,
                        standing_item,
                        quiet_for: Duration::ZERO,
                        may_sleep: MAY_SLEEP,
                        woken: 0,
                        wake_limit: 2,
                    };
                    let step = asleep_step(&team);
                    let expected = if lead_status != TaskStatus::Waiting {
                        AsleepStep::Nothing
                    } else if lead_done {
                        if standing_item {
                            AsleepStep::TellHuman
                        } else {
                            AsleepStep::Nothing
                        }
                    } else {
                        AsleepStep::Nothing
                    };
                    assert_eq!(
                        step, expected,
                        "lead_status={lead_status:?}, lead_done={lead_done}, standing={standing_item}, busy={busy_member}"
                    );
                }
            }
        }
    }
}

// Rule: blocked_item_step returns TellHuman iff lead_done is true, lead_status is Waiting, and assignee_status is Failed.
#[test]
fn pure_blocked_item_step_table() {
    let all_statuses = [
        TaskStatus::Waiting,
        TaskStatus::Running,
        TaskStatus::Queued,
        TaskStatus::Questions,
        TaskStatus::Paused,
        TaskStatus::Done,
        TaskStatus::Failed,
        TaskStatus::Stopped,
    ];

    for lead_status in all_statuses {
        for lead_done in [false, true] {
            for assignee_status in all_statuses {
                let step = blocked_item_step(lead_done, lead_status, assignee_status);
                let expected =
                    if lead_done && lead_status == TaskStatus::Waiting && assignee_status == TaskStatus::Failed {
                        BlockedItemStep::TellHuman
                    } else {
                        BlockedItemStep::Nothing
                    };
                assert_eq!(
                    step, expected,
                    "lead_status={lead_status:?}, lead_done={lead_done}, assignee_status={assignee_status:?}"
                );
            }
        }
    }
}
