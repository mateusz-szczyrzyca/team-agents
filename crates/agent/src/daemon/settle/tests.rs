use agent_core::config::Limits;
use agent_core::decide::outcome::{ClassifyError, MAX_PAUSE, Pause, PauseReason, RESET_MARGIN};
use agent_core::kinds::Role;
use agent_core::state::{RunEnd, Timestamp};
use agent_core::status::{RunOutcome, TaskStatus};

use super::*;
use crate::fixtures::at;

fn reported(outcome: RunOutcome) -> RunEnd {
    RunEnd::Reported { outcome }
}

fn pause(until: u32) -> Pause {
    Pause {
        until: at(until),
        reason: PauseReason::Quota,
        evidence: "usage limit".to_owned(),
    }
}

#[test]
fn done_finishes_a_solo_task_and_a_review_and_lets_the_rest_of_a_team_wait_for_more() {
    let cases = [
        (Role::Solo, TaskStatus::Done),
        (Role::Reviewer, TaskStatus::Done),
        (Role::Coordinator, TaskStatus::Waiting),
        (Role::Member, TaskStatus::Waiting),
        (Role::Worker, TaskStatus::Waiting),
    ];

    for (role, status) in cases {
        assert_eq!(
            settle(role, &reported(RunOutcome::Done), None, Some("ok")),
            Settled::Status(status),
            "{role:?}"
        );
    }
}

#[test]
fn sleeping_waits_excluded_is_over_and_questions_ask() {
    assert_eq!(
        settle(Role::Member, &reported(RunOutcome::Sleeping), None, None),
        Settled::Status(TaskStatus::Waiting)
    );
    assert_eq!(
        settle(Role::Member, &reported(RunOutcome::Excluded), None, None),
        Settled::Status(TaskStatus::Done)
    );
    assert_eq!(
        settle(Role::Solo, &reported(RunOutcome::Questions), None, None),
        Settled::Questions
    );
}

#[test]
fn a_reported_failure_fails_the_task_and_tells_the_human_what_the_agent_said() {
    let settled = settle(
        Role::Solo,
        &reported(RunOutcome::Failed),
        None,
        Some("It broke.\nSTATUS: FAILED"),
    );

    assert_eq!(
        settled,
        Settled::Failed(Notice::Failed {
            message: "It broke.\nSTATUS: FAILED".to_owned()
        })
    );
}

#[test]
fn only_the_end_of_a_long_message_is_kept_for_the_human() {
    let long = format!("{}{}", "a".repeat(MAX_MESSAGE_CHARS), "tail żółć");

    let settled = settle(Role::Solo, &reported(RunOutcome::Failed), None, Some(&long));

    let Settled::Failed(Notice::Failed { message }) = settled else {
        panic!("{settled:?}")
    };
    assert_eq!(message.chars().count(), MAX_MESSAGE_CHARS);
    assert!(message.ends_with("tail żółć"), "{message}");
}

#[test]
fn a_failure_without_a_message_has_an_empty_one() {
    let settled = settle(Role::Solo, &reported(RunOutcome::Failed), None, None);

    assert_eq!(settled, Settled::Failed(Notice::Failed { message: String::new() }));
}

#[test]
fn an_end_without_a_status_fails_the_task_with_its_exit_code() {
    for exit in [Some(3), None] {
        assert_eq!(
            settle(Role::Member, &RunEnd::NoStatus { exit_code: exit }, None, None),
            Settled::Failed(Notice::NoStatus { exit })
        );
    }
}

#[test]
fn a_limit_pauses_the_task_with_the_pause_the_classifier_gave() {
    let given = pause(60);

    for end in [
        RunEnd::QuotaExhausted {
            resets_at: Some(at(50)),
        },
        RunEnd::RateLimited,
    ] {
        assert_eq!(
            settle(Role::Solo, &end, Some(&given), None),
            Settled::Paused(given.clone())
        );
    }
}

#[test]
fn a_limit_without_a_pause_cannot_be_waited_for_and_fails_the_task() {
    assert_eq!(
        settle(Role::Solo, &RunEnd::RateLimited, None, None),
        Settled::Failed(Notice::NoStatus { exit: None })
    );
}

#[test]
fn a_hang_is_left_to_the_watchdogs_strikes_and_a_stop_stops() {
    assert_eq!(settle(Role::Solo, &RunEnd::Hung, None, None), Settled::Hung);
    assert_eq!(
        settle(Role::Solo, &RunEnd::Stopped, None, None),
        Settled::Status(TaskStatus::Stopped)
    );
}

fn limits() -> Limits {
    Limits::default()
}

fn plus(base: Timestamp, span: time::Duration) -> Timestamp {
    Timestamp::from_utc(base.as_utc() + span).unwrap()
}

#[test]
fn a_quota_with_a_reset_time_wakes_the_task_a_margin_after_it() {
    let reset = plus(at(0), time::Duration::hours(3));

    let wake = wake_time(&RunEnd::QuotaExhausted { resets_at: Some(reset) }, at(0), &limits()).unwrap();

    assert_eq!(wake, Some(plus(reset, RESET_MARGIN)));
}

#[test]
fn a_quota_without_a_reset_time_waits_the_configured_pause() {
    let wake = wake_time(&RunEnd::QuotaExhausted { resets_at: None }, at(0), &limits()).unwrap();

    assert_eq!(wake, Some(plus(at(0), time::Duration::minutes(30) + RESET_MARGIN)));
}

#[test]
fn a_quota_whose_reset_time_is_not_after_the_end_waits_the_configured_pause() {
    let wake = wake_time(&RunEnd::QuotaExhausted { resets_at: Some(at(0)) }, at(0), &limits()).unwrap();

    assert_eq!(wake, Some(plus(at(0), time::Duration::minutes(30) + RESET_MARGIN)));
}

#[test]
fn a_reset_time_far_away_is_cut_to_the_longest_pause() {
    let reset = plus(at(0), time::Duration::days(30));

    let wake = wake_time(&RunEnd::QuotaExhausted { resets_at: Some(reset) }, at(0), &limits()).unwrap();

    assert_eq!(wake, Some(plus(at(0), MAX_PAUSE + RESET_MARGIN)));
}

#[test]
fn a_rate_limit_waits_the_configured_minutes() {
    let wake = wake_time(&RunEnd::RateLimited, at(0), &limits()).unwrap();

    assert_eq!(wake, Some(plus(at(0), time::Duration::minutes(5) + RESET_MARGIN)));
}

#[test]
fn an_end_that_is_no_limit_wakes_nothing() {
    let ends = [
        reported(RunOutcome::Done),
        RunEnd::NoStatus { exit_code: None },
        RunEnd::Hung,
        RunEnd::Stopped,
    ];

    for end in ends {
        assert_eq!(wake_time(&end, at(0), &limits()).unwrap(), None, "{end:?}");
    }
}

#[test]
fn a_pause_that_would_end_after_the_year_9999_is_an_error() {
    let late: Timestamp = "9999-12-31T23:59:30Z".parse().unwrap();

    let error = wake_time(&RunEnd::RateLimited, late, &limits()).unwrap_err();

    assert!(
        matches!(error, ClassifyError::PauseOverflow { now } if now == late),
        "{error:?}"
    );
}
