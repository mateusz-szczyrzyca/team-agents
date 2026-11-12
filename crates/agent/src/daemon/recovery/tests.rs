use agent_core::config::Limits;
use agent_core::decide::outcome::{ClassifyError, RESET_MARGIN};
use agent_core::state::{RunEnd, RunState, SessionId, State, Timestamp};
use agent_core::status::{RunOutcome, TaskStatus};

use super::*;
use crate::fixtures::{at, run, task, team_state};

fn limits() -> Limits {
    Limits::default()
}

/// Task `n` starts run `n` at 10 seconds, with `session`.
fn start(state: &mut State, n: u32, session: Option<&str>) {
    let (_, mut member) = state.find_task_mut(&task(n)).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    member
        .begin_run(run(n), at(10), session.map(|id| id.parse().unwrap()))
        .unwrap();
}

/// Ends the run of task `n` at 20 seconds, and the task goes to `status`.
fn end(state: &mut State, n: u32, end: RunEnd, status: TaskStatus) {
    let (_, mut member) = state.find_task_mut(&task(n)).unwrap();
    member.end_run(at(20), end).unwrap();
    member.transition(status).unwrap();
}

fn status(state: &State, n: u32) -> TaskStatus {
    state.find_task(&task(n)).unwrap().member.status()
}

#[test]
fn a_run_that_was_going_is_ended_as_interrupted_and_its_task_queued_with_its_session() {
    let mut state = team_state();
    start(&mut state, 2, Some("ses-1"));

    let recovery = recover(&mut state, at(100), &limits()).unwrap();

    assert_eq!(recovery.interrupted, [task(2)]);
    assert_eq!(status(&state, 2), TaskStatus::Queued);
    let member = state.find_task(&task(2)).unwrap().member;
    let run = member.runs().last().unwrap();
    assert_eq!(
        run.state(),
        &RunState::Ended {
            at: at(100),
            end: RunEnd::NoStatus { exit_code: None }
        }
    );
    assert_eq!(member.last_session().map(SessionId::as_str), Some("ses-1"));
}

#[test]
fn a_task_that_runs_without_a_run_is_queued() {
    let mut state = team_state();
    state
        .find_task_mut(&task(2))
        .unwrap()
        .1
        .transition(TaskStatus::Running)
        .unwrap();

    let recovery = recover(&mut state, at(100), &limits()).unwrap();

    assert_eq!(recovery.interrupted, [task(2)]);
    assert_eq!(status(&state, 2), TaskStatus::Queued);
}

#[test]
fn reviewers_are_recovered_like_the_crew() {
    let mut state = team_state();
    start(&mut state, 4, None);

    let recovery = recover(&mut state, at(100), &limits()).unwrap();

    assert_eq!(recovery.interrupted, [task(4)]);
    assert_eq!(status(&state, 4), TaskStatus::Queued);
}

#[test]
fn everything_else_is_left_as_it_is() {
    let mut state = team_state();
    start(&mut state, 1, Some("ses-a"));
    end(
        &mut state,
        1,
        RunEnd::Reported {
            outcome: RunOutcome::Sleeping,
        },
        TaskStatus::Waiting,
    );
    start(&mut state, 2, None);
    end(
        &mut state,
        2,
        RunEnd::NoStatus { exit_code: Some(1) },
        TaskStatus::Failed,
    );
    let before = state.to_json().unwrap();

    let recovery = recover(&mut state, at(100), &limits()).unwrap();

    assert_eq!(recovery, Recovery::default());
    assert_eq!(state.to_json().unwrap(), before);
}

#[test]
fn a_task_paused_by_a_limit_is_woken_when_the_limit_is_over() {
    let mut state = team_state();
    start(&mut state, 2, Some("ses-1"));
    let reset: Timestamp = "2026-11-11T22:15:00Z".parse().unwrap();
    end(
        &mut state,
        2,
        RunEnd::QuotaExhausted { resets_at: Some(reset) },
        TaskStatus::Paused,
    );
    start(&mut state, 3, None);
    end(&mut state, 3, RunEnd::RateLimited, TaskStatus::Paused);

    let recovery = recover(&mut state, at(100), &limits()).unwrap();

    let quota_wake = Timestamp::from_utc(reset.as_utc() + RESET_MARGIN).unwrap();
    let rate_wake = Timestamp::from_utc(at(20).as_utc() + time::Duration::minutes(5) + RESET_MARGIN).unwrap();
    assert_eq!(recovery.wakes, [(quota_wake, task(2)), (rate_wake, task(3))]);
    assert_eq!(
        status(&state, 2),
        TaskStatus::Paused,
        "a pause is not ended by recovery"
    );
}

#[test]
fn a_task_paused_by_the_human_is_not_woken() {
    let mut state = team_state();
    start(&mut state, 2, None);
    end(&mut state, 2, RunEnd::Stopped, TaskStatus::Paused);
    state
        .find_task_mut(&task(3))
        .unwrap()
        .1
        .transition(TaskStatus::Paused)
        .unwrap();

    let recovery = recover(&mut state, at(100), &limits()).unwrap();

    assert_eq!(recovery.wakes, []);
}

#[test]
fn recovering_twice_changes_nothing_the_second_time() {
    let mut state = team_state();
    start(&mut state, 2, Some("ses-1"));
    recover(&mut state, at(100), &limits()).unwrap();
    let before = state.to_json().unwrap();

    let again = recover(&mut state, at(200), &limits()).unwrap();

    assert_eq!(again, Recovery::default());
    assert_eq!(state.to_json().unwrap(), before);
}

#[test]
fn a_pause_that_cannot_be_timed_is_an_error_naming_the_task() {
    let mut state = team_state();
    start(&mut state, 2, None);
    let (_, mut member) = state.find_task_mut(&task(2)).unwrap();
    let late: Timestamp = "9999-12-31T23:59:30Z".parse().unwrap();
    member.end_run(late, RunEnd::RateLimited).unwrap();
    member.transition(TaskStatus::Paused).unwrap();

    let error = recover(&mut state, at(100), &limits()).unwrap_err();

    assert!(
        matches!(&error, RecoveryError::Pause { task: t, source: ClassifyError::PauseOverflow { .. } } if *t == task(2)),
        "{error:?}"
    );
}

#[test]
fn a_clock_set_back_does_not_end_a_run_before_it_started() {
    let mut state = team_state();
    start(&mut state, 2, Some("ses-1"));

    let recovery = recover(&mut state, at(5), &limits()).unwrap();

    assert_eq!(recovery.interrupted, [task(2)]);
    let member = state.find_task(&task(2)).unwrap().member;
    assert_eq!(
        member.runs().last().unwrap().state(),
        &RunState::Ended {
            at: at(10),
            end: RunEnd::NoStatus { exit_code: None }
        },
        "the run ends when it started at the earliest"
    );
}

/// Task 1 ends a run with each of `ends`, one after the other, and is queued after each.
fn queued_after(ends: &[RunEnd]) -> State {
    let mut state = team_state();
    let (_, mut member) = state.find_task_mut(&task(1)).unwrap();
    for (n, end) in (1..).zip(ends) {
        member.transition(TaskStatus::Running).unwrap();
        member.begin_run(run(n), at(10), None).unwrap();
        member.end_run(at(20), end.clone()).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
    }
    state
}

#[test]
fn the_hangs_in_a_row_of_a_queued_task_are_counted_again() {
    let mut state = queued_after(&[
        RunEnd::Reported {
            outcome: RunOutcome::Done,
        },
        RunEnd::Hung,
        RunEnd::Hung,
    ]);

    let recovery = recover(&mut state, at(100), &limits()).unwrap();

    assert_eq!(recovery.hangs, [(task(1), 2)]);
}

#[test]
fn a_run_a_restart_interrupted_does_not_end_the_row_of_hangs() {
    let mut state = queued_after(&[RunEnd::Hung, RunEnd::NoStatus { exit_code: None }]);

    let recovery = recover(&mut state, at(100), &limits()).unwrap();

    assert_eq!(recovery.hangs, [(task(1), 1)]);
}

#[test]
fn a_task_whose_last_run_did_not_hang_has_no_hangs() {
    let mut state = queued_after(&[RunEnd::Hung, RunEnd::NoStatus { exit_code: Some(1) }]);

    let recovery = recover(&mut state, at(100), &limits()).unwrap();

    assert_eq!(recovery.hangs, []);
}
