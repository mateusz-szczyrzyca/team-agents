use agent_core::config::Limits;
use agent_core::decide::ladder::Trouble;
use agent_core::decide::outcome::{ClassifyError, RESET_MARGIN};
use agent_core::ids::RoundId;
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

#[test]
fn recovery_does_not_queue_a_fourth_no_status_run() {
    for number in [1, 2, 3, 4] {
        for prior in 0..=2 {
            let mut state = team_state();
            let (_, mut member) = state.find_task_mut(&task(number)).unwrap();
            for index in 0..=prior {
                member.transition(TaskStatus::Running).unwrap();
                member
                    .begin_run(run(index + 1), at(10), Some("session".parse().unwrap()))
                    .unwrap();
                if index < prior {
                    member.end_run(at(20), RunEnd::NoStatus { exit_code: Some(0) }).unwrap();
                    member.transition(TaskStatus::Queued).unwrap();
                }
            }
            let recovery = recover(&mut state, at(100), &limits()).unwrap();
            let expected = if prior == 2 && number <= 2 {
                TaskStatus::Failed
            } else {
                TaskStatus::Queued
            };
            assert_eq!(status(&state, number), expected, "task {number}, prior {prior}");
            assert_eq!(recovery.interrupted, [task(number)]);
        }
    }
}

/// Task 2 ran three runs in session `ses-1` that ended with `end`, was started anew after them, and runs one run of its
/// new session in `ses-2`, which ended with `last` (or is going when `last` is `None`).
fn renewed_after_three(end: &RunEnd, last: Option<RunEnd>) -> State {
    let mut state = team_state();
    for n in 1..=3 {
        let (_, mut member) = state.find_task_mut(&task(2)).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member
            .begin_run(run(10 + n), at(10 * n), Some("ses-1".parse().unwrap()))
            .unwrap();
        member.end_run(at(10 * n + 5), end.clone()).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
    }
    state
        .renew_task(&task(2), agent_core::decide::ladder::Trouble::NoStatus, at(40))
        .unwrap();
    let (_, mut member) = state.find_task_mut(&task(2)).unwrap();
    member.transition(TaskStatus::Running).unwrap();
    member
        .begin_run(run(20), at(50), Some("ses-2".parse().unwrap()))
        .unwrap();
    if let Some(last) = last {
        member.end_run(at(55), last).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
    }
    state
}

#[test]
fn a_restart_counts_the_no_status_endings_of_the_new_session_only() {
    let mut state = renewed_after_three(&RunEnd::NoStatus { exit_code: Some(0) }, None);

    let recovery = recover(&mut state, at(100), &limits()).unwrap();

    assert_eq!(recovery.exhausted, Vec::<TaskId>::new());
    assert_eq!(status(&state, 2), TaskStatus::Queued);
}

#[test]
fn a_restart_counts_the_hangs_of_the_new_session_only() {
    let mut state = renewed_after_three(&RunEnd::Hung, Some(RunEnd::Hung));

    let recovery = recover(&mut state, at(100), &limits()).unwrap();

    assert_eq!(recovery.hangs, [(task(2), 1)]);
}

// ---- the session a cut run goes on in ------------------------------------------------------------------------------

/// The open round the cut runs of these tests worked for.
fn round() -> RoundId {
    "round-20261123-111025-aaaa".parse().unwrap()
}

/// What a restart does after task `n`, whose run was cut and whose task is queued, under `setting`.
fn after_cut_of(state: &State, n: u32, setting: RestartSession, round: Option<&RoundId>) -> AfterCut {
    let found = state.find_task(&task(n)).unwrap();
    after_cut(setting, found.role, found.member, round)
}

/// A state in which task `n` ran once, with `session`, and the run was cut by a restart.
fn cut(n: u32, session: Option<&str>) -> State {
    let mut state = team_state();
    start(&mut state, n, session);
    end(&mut state, n, RunEnd::NoStatus { exit_code: None }, TaskStatus::Queued);
    state
}

#[test]
fn a_cut_run_of_a_working_role_goes_on_in_a_new_session_whatever_its_session() {
    for (n, session) in [(1, Some("ses-1")), (2, Some("ses-1")), (2, None)] {
        let state = cut(n, session);

        assert_eq!(
            after_cut_of(&state, n, RestartSession::New, Some(&round())),
            AfterCut::NewSession,
            "task {n} with session {session:?}"
        );
    }
}

#[test]
fn the_key_can_keep_the_session_a_run_was_cut_in() {
    let state = cut(2, Some("ses-1"));

    assert_eq!(
        after_cut_of(&state, 2, RestartSession::Same, Some(&round())),
        AfterCut::SameSession
    );
}

#[test]
fn a_cut_run_of_a_worker_or_a_reviewer_is_resumed_as_before() {
    for n in [3, 4] {
        let state = cut(n, Some("ses-1"));

        assert_eq!(
            after_cut_of(&state, n, RestartSession::New, Some(&round())),
            AfterCut::SameSession,
            "task {n}"
        );
    }
}

#[test]
fn a_run_that_worked_for_no_open_round_is_resumed_as_before() {
    let state = cut(2, Some("ses-1"));

    assert_eq!(
        after_cut_of(&state, 2, RestartSession::New, None),
        AfterCut::SameSession
    );
}

#[test]
fn a_run_that_was_not_cut_by_the_daemon_is_not_started_anew() {
    for end_of_run in [
        RunEnd::Hung,
        RunEnd::TurnLimit,
        RunEnd::NoStatus { exit_code: Some(1) },
        RunEnd::Stopped,
    ] {
        let mut state = team_state();
        start(&mut state, 2, Some("ses-1"));
        end(&mut state, 2, end_of_run.clone(), TaskStatus::Queued);

        assert_eq!(
            after_cut_of(&state, 2, RestartSession::New, Some(&round())),
            AfterCut::SameSession,
            "{end_of_run:?}"
        );
    }
}

#[test]
fn a_task_that_is_not_queued_is_left_to_its_own_status() {
    for status in [TaskStatus::Failed, TaskStatus::Stopped, TaskStatus::Paused] {
        let mut state = team_state();
        start(&mut state, 2, Some("ses-1"));
        end(&mut state, 2, RunEnd::NoStatus { exit_code: None }, status);

        assert_eq!(
            after_cut_of(&state, 2, RestartSession::New, Some(&round())),
            AfterCut::SameSession,
            "{status:?}"
        );
    }
}

#[test]
fn a_task_that_already_awaits_a_new_session_is_not_started_anew_twice() {
    let mut state = cut(2, Some("ses-1"));
    state
        .begin_holder_session(&task(2), Trouble::DaemonDown, at(30))
        .unwrap();

    assert_eq!(
        after_cut_of(&state, 2, RestartSession::New, Some(&round())),
        AfterCut::SameSession
    );
}

// ---- a restart that begins a new session ---------------------------------------------------------------------------

/// A state in which task `n` ran in an open round: `ended` runs ended with `end_of_run`, and one more is going, as when
/// the daemon died.
fn dying_in_a_round(n: u32, ended: u32, end_of_run: &RunEnd) -> State {
    let mut state = team_state();
    state
        .team_mut(&crate::fixtures::team_id(0))
        .unwrap()
        .open_round(round(), Some("1.1".parse().unwrap()), at(1))
        .unwrap();
    for number in 1..=ended + 1 {
        state
            .find_task_mut(&task(n))
            .unwrap()
            .1
            .transition(TaskStatus::Running)
            .unwrap();
        state
            .team_mut(&crate::fixtures::team_id(0))
            .unwrap()
            .begin_run(
                &task(n),
                agent_core::state::NewRun {
                    id: run(number),
                    at: at(10 * number),
                    session: Some("ses-1".parse().unwrap()),
                    rounds: vec![round()],
                },
            )
            .unwrap();
        if number <= ended {
            let (_, mut member) = state.find_task_mut(&task(n)).unwrap();
            member.end_run(at(10 * number + 5), end_of_run.clone()).unwrap();
            member.transition(TaskStatus::Queued).unwrap();
        }
    }
    state
}

#[test]
fn a_run_the_daemon_cut_begins_a_new_session_that_counts_no_retry() {
    let mut state = dying_in_a_round(2, 0, &RunEnd::Hung);

    let recovery = recover_with(&mut state, at(500), &limits(), RestartSession::New).unwrap();

    let member = state.find_task(&task(2)).unwrap().member;
    assert_eq!(recovery.renewed, [task(2)]);
    assert_eq!(member.status(), TaskStatus::Queued);
    assert!(member.awaits_new_session());
    let renewals = member.renewals().expect("the new session is recorded");
    assert_eq!(renewals.reason(), Trouble::DaemonDown);
    assert_eq!(renewals.used(), 0);
}

#[test]
fn two_endings_without_status_and_a_cut_by_the_daemon_do_not_fail_the_task() {
    let no_status = RunEnd::NoStatus { exit_code: Some(0) };
    let mut state = dying_in_a_round(2, 2, &no_status);

    let recovery = recover_with(&mut state, at(500), &limits(), RestartSession::New).unwrap();

    assert_eq!(recovery.exhausted, Vec::<TaskId>::new());
    assert_eq!(recovery.renewed, [task(2)]);
    assert_eq!(status(&state, 2), TaskStatus::Queued);
}

#[test]
fn with_the_same_session_the_cut_still_counts_as_the_third_ending_without_status() {
    let no_status = RunEnd::NoStatus { exit_code: Some(0) };
    let mut state = dying_in_a_round(2, 2, &no_status);

    let recovery = recover_with(&mut state, at(500), &limits(), RestartSession::Same).unwrap();

    assert_eq!(recovery.exhausted, [task(2)]);
    assert_eq!(recovery.renewed, Vec::<TaskId>::new());
    assert_eq!(status(&state, 2), TaskStatus::Failed);
}

#[test]
fn a_run_a_signal_ended_earlier_is_not_a_cut_by_this_restart() {
    // Recorded as ended with no exit code, as the daemon's own cut is; but nothing was going when this daemon started.
    let mut state = dying_in_a_round(2, 1, &RunEnd::NoStatus { exit_code: None });

    let recovery = recover_with(&mut state, at(500), &limits(), RestartSession::New);

    let recovery = recovery.unwrap();
    // The one going run is the cut; the earlier signal-ended run is only history.
    assert_eq!(recovery.renewed, [task(2)]);
    let mut quiet = dying_in_a_round(2, 1, &RunEnd::NoStatus { exit_code: None });
    let (_, mut member) = quiet.find_task_mut(&task(2)).unwrap();
    member.end_run(at(60), RunEnd::NoStatus { exit_code: None }).unwrap();
    member.transition(TaskStatus::Queued).unwrap();

    let recovery = recover_with(&mut quiet, at(500), &limits(), RestartSession::New).unwrap();

    assert_eq!(recovery.renewed, Vec::<TaskId>::new());
    assert_eq!(quiet.find_task(&task(2)).unwrap().member.renewals(), None);
}

#[test]
fn a_cut_run_of_a_reviewer_or_a_worker_begins_no_new_session() {
    for n in [3, 4] {
        let mut state = dying_in_a_round(n, 0, &RunEnd::Hung);

        let recovery = recover_with(&mut state, at(500), &limits(), RestartSession::New).unwrap();

        assert_eq!(recovery.renewed, Vec::<TaskId>::new(), "task {n}");
    }
}
