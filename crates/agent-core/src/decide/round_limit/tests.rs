use super::*;

fn at(text: &str) -> Timestamp {
    text.parse().expect("a time of the tests")
}

/// A run started at `started` that ended stopped when `stopped`.
fn run(started: &str, stopped: bool) -> RunMark {
    RunMark {
        started_at: at(started),
        stopped,
    }
}

/// The clock of a round opened at 10:00 of a task its agent held from the start, with `runs`.
fn opened_at_ten(runs: &[RunMark]) -> RoundClock<'_> {
    RoundClock {
        opened_at: at("2026-11-25T10:00:00Z"),
        held_since: None,
        runs,
    }
}

#[test]
fn a_limit_of_zero_never_makes_a_round_overdue() {
    let runs = [run("2026-11-25T10:01:00Z", false)];
    assert_eq!(
        round_overdue(&opened_at_ten(&runs), at("2026-11-25T20:00:00Z"), 0),
        Overdue::NotDue
    );
}

#[test]
fn a_round_is_overdue_once_its_minutes_reach_the_limit() {
    let runs = [run("2026-11-25T10:01:00Z", false)];
    let clock = opened_at_ten(&runs);
    assert_eq!(round_overdue(&clock, at("2026-11-25T11:59:59Z"), 120), Overdue::NotDue);
    assert_eq!(
        round_overdue(&clock, at("2026-11-25T12:00:00Z"), 120),
        Overdue::Due {
            minutes: 120,
            since: at("2026-11-25T10:00:00Z"),
        }
    );
    assert_eq!(
        round_overdue(&clock, at("2026-11-25T12:29:00Z"), 120),
        Overdue::Due {
            minutes: 149,
            since: at("2026-11-25T10:00:00Z"),
        }
    );
}

#[test]
fn the_clock_starts_again_when_another_agent_takes_the_task_over() {
    let runs = [run("2026-11-25T10:01:00Z", false), run("2026-11-25T11:00:30Z", false)];
    let clock = RoundClock {
        held_since: Some(at("2026-11-25T11:00:00Z")),
        ..opened_at_ten(&runs)
    };
    assert_eq!(clock_start(&clock), at("2026-11-25T11:00:00Z"));
    assert_eq!(round_overdue(&clock, at("2026-11-25T12:30:00Z"), 120), Overdue::NotDue);
    assert_eq!(
        round_overdue(&clock, at("2026-11-25T13:00:00Z"), 120),
        Overdue::Due {
            minutes: 120,
            since: at("2026-11-25T11:00:00Z"),
        }
    );
}

#[test]
fn a_hand_over_before_the_round_was_opened_leaves_the_opening_as_the_start() {
    let runs = [run("2026-11-25T10:01:00Z", false)];
    let clock = RoundClock {
        held_since: Some(at("2026-11-25T09:00:00Z")),
        ..opened_at_ten(&runs)
    };
    assert_eq!(clock_start(&clock), at("2026-11-25T10:00:00Z"));
}

#[test]
fn the_first_run_after_a_stopped_run_starts_the_clock_again() {
    let runs = [
        run("2026-11-25T10:01:00Z", false),
        run("2026-11-25T10:30:00Z", true),
        run("2026-11-25T11:30:00Z", false),
    ];
    let clock = opened_at_ten(&runs);
    assert_eq!(clock_start(&clock), at("2026-11-25T11:30:00Z"));
    assert_eq!(round_overdue(&clock, at("2026-11-25T13:00:00Z"), 120), Overdue::NotDue);
}

#[test]
fn only_the_last_stopped_run_counts() {
    let runs = [
        run("2026-11-25T10:01:00Z", true),
        run("2026-11-25T10:20:00Z", true),
        run("2026-11-25T10:40:00Z", false),
        run("2026-11-25T11:00:00Z", false),
    ];
    assert_eq!(clock_start(&opened_at_ten(&runs)), at("2026-11-25T10:40:00Z"));
}

#[test]
fn a_stopped_last_run_with_no_run_after_it_keeps_the_start_before_it() {
    let runs = [run("2026-11-25T10:01:00Z", false), run("2026-11-25T10:40:00Z", true)];
    assert_eq!(clock_start(&opened_at_ten(&runs)), at("2026-11-25T10:00:00Z"));
}

#[test]
fn a_run_resumed_before_the_round_was_opened_leaves_the_opening_as_the_start() {
    let runs = [run("2026-11-25T09:00:00Z", true), run("2026-11-25T09:30:00Z", false)];
    assert_eq!(clock_start(&opened_at_ten(&runs)), at("2026-11-25T10:00:00Z"));
}

#[test]
fn a_time_before_the_start_of_the_clock_is_never_overdue() {
    let runs = [run("2026-11-25T10:01:00Z", false)];
    assert_eq!(
        round_overdue(&opened_at_ten(&runs), at("2026-11-25T09:00:00Z"), 1),
        Overdue::NotDue
    );
}

#[test]
fn a_wait_for_the_human_is_left_out_of_the_clock() {
    let runs = [run("2026-11-25T10:01:00Z", false)];
    let clock = opened_at_ten(&runs);
    let waits = [HumanWait {
        asked_at: at("2026-11-25T10:30:00Z"),
        answered_at: Some(at("2026-11-25T14:00:00Z")),
    }];
    assert_eq!(
        round_overdue_with_waits(&clock, &waits, at("2026-11-25T14:30:00Z"), 120),
        Overdue::NotDue
    );
    assert_eq!(
        round_overdue_with_waits(&clock, &waits, at("2026-11-25T15:30:00Z"), 120),
        Overdue::Due {
            minutes: 120,
            since: at("2026-11-25T10:00:00Z"),
        }
    );
}

#[test]
fn an_unanswered_question_counts_as_waiting_up_to_now() {
    let runs = [run("2026-11-25T10:01:00Z", false)];
    let waits = [HumanWait {
        asked_at: at("2026-11-25T10:30:00Z"),
        answered_at: None,
    }];
    assert_eq!(
        round_overdue_with_waits(&opened_at_ten(&runs), &waits, at("2026-11-25T18:00:00Z"), 120),
        Overdue::NotDue
    );
}

#[test]
fn a_wait_that_began_before_the_clock_takes_off_only_its_part_after_the_start() {
    let runs = [run("2026-11-25T10:01:00Z", false)];
    let clock = RoundClock {
        held_since: Some(at("2026-11-25T12:00:00Z")),
        ..opened_at_ten(&runs)
    };
    let waits = [HumanWait {
        asked_at: at("2026-11-25T11:00:00Z"),
        answered_at: Some(at("2026-11-25T13:00:00Z")),
    }];
    assert_eq!(
        round_overdue_with_waits(&clock, &waits, at("2026-11-25T14:30:00Z"), 120),
        Overdue::NotDue
    );
    assert_eq!(
        round_overdue_with_waits(&clock, &waits, at("2026-11-25T15:00:00Z"), 120),
        Overdue::Due {
            minutes: 120,
            since: at("2026-11-25T12:00:00Z"),
        }
    );
}

#[test]
fn a_wait_that_ended_before_the_clock_takes_nothing_off() {
    let runs = [run("2026-11-25T10:01:00Z", false)];
    let clock = RoundClock {
        held_since: Some(at("2026-11-25T12:00:00Z")),
        ..opened_at_ten(&runs)
    };
    let waits = [HumanWait {
        asked_at: at("2026-11-25T10:30:00Z"),
        answered_at: Some(at("2026-11-25T11:30:00Z")),
    }];
    assert_eq!(
        round_overdue_with_waits(&clock, &waits, at("2026-11-25T14:00:00Z"), 120),
        Overdue::Due {
            minutes: 120,
            since: at("2026-11-25T12:00:00Z"),
        }
    );
}

#[test]
fn overlapping_waits_are_left_out_once() {
    let runs = [run("2026-11-25T10:01:00Z", false)];
    let waits = [
        HumanWait {
            asked_at: at("2026-11-25T10:30:00Z"),
            answered_at: Some(at("2026-11-25T12:00:00Z")),
        },
        HumanWait {
            asked_at: at("2026-11-25T11:00:00Z"),
            answered_at: Some(at("2026-11-25T13:00:00Z")),
        },
    ];
    assert_eq!(
        round_overdue_with_waits(&opened_at_ten(&runs), &waits, at("2026-11-25T15:00:00Z"), 120),
        Overdue::Due {
            minutes: 150,
            since: at("2026-11-25T10:00:00Z"),
        }
    );
}

#[test]
fn a_limit_of_zero_stays_off_with_waits() {
    let runs = [run("2026-11-25T10:01:00Z", false)];
    let waits = [HumanWait {
        asked_at: at("2026-11-25T10:30:00Z"),
        answered_at: Some(at("2026-11-25T11:00:00Z")),
    }];
    assert_eq!(
        round_overdue_with_waits(&opened_at_ten(&runs), &waits, at("2026-11-25T20:00:00Z"), 0),
        Overdue::NotDue
    );
}
