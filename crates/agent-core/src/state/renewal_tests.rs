//! A task started again in a new session: what the state keeps of it, and how it reads a state written before it.

use super::*;
use crate::decide::ladder::Trouble;
use crate::status::{RunOutcome, TaskStatus};

/// The session the member of these tests ran in.
const OLD_SESSION: &str = "5f3c9a3e-86f3-4b0e-9d2c-0a8f1c7d5b21";

/// The task of these tests: its member ended seven runs without a status and is failed.
fn task() -> TaskId {
    "task-20261120-144101-r7q2".parse().unwrap()
}

/// A state written by `main` before renewals existed, with that failed member in it.
fn old_state() -> State {
    State::from_json(include_str!("../../tests/fixtures/state-before-renewals.json")).expect("a state of main loads")
}

/// When these tests' runs start.
fn started() -> Timestamp {
    "2026-11-20T20:00:00Z".parse().unwrap()
}

#[test]
fn a_failed_member_of_main_with_the_retries_used_loads_without_a_renewal_limit() {
    let state = old_state();
    let found = state.find_task(&task()).expect("the failed task is there");

    assert_eq!(found.member.status(), TaskStatus::Failed);
    assert_eq!(found.member.renewals(), None);
    assert_eq!(found.member.last_session().map(SessionId::as_str), Some(OLD_SESSION));
    assert_eq!(found.member.runs().len(), 7);
}

/// Puts the failed task of the old state in its session again, with a run that has ended done: the state the
/// daemon reaches just before it starts a task anew.
fn renewed(member: &mut Member) {
    let renewed = member
        .renew(Trouble::NoStatus, started())
        .expect("a failed task may be started anew");
    assert!(renewed);
    member
        .transition(TaskStatus::Queued)
        .expect("a failed task may be queued");
    member.transition(TaskStatus::Running).expect("a queued task runs");
    member
        .begin_run("run-20261120-200001-n1n1".parse().unwrap(), started(), None)
        .expect("a run starts");
}

#[test]
fn a_task_started_anew_forgets_the_session_of_its_runs_before_and_keeps_the_new_one() {
    let mut state = old_state();
    let (_, mut member) = state.find_task_mut(&task()).expect("the task");
    renewed(member.value);

    // The new run began in no session that the CLI named yet: nothing is resumed.
    assert_eq!(member.last_session(), None);
    let new_session: SessionId = "00000000-0000-4000-8000-000000000002".parse().unwrap();
    member
        .attach_session(new_session.clone())
        .expect("the CLI names its session");
    member
        .end_run(
            started(),
            RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
        )
        .expect("the run ends");

    assert_eq!(member.last_session(), Some(&new_session));
    assert_eq!(
        member.renewals().map(|renewals| renewals.used()),
        Some(1),
        "the renewal is counted"
    );
}

#[test]
fn a_renewal_is_kept_and_round_trips_through_the_state() {
    let mut state = old_state();
    renewed(state.find_task_mut(&task()).expect("the task").1.value);
    let saved = state.to_json().expect("the state saves");
    let loaded = State::from_json(&saved).expect("it loads again");

    let found = loaded.find_task(&task()).expect("the task");
    assert_eq!(found.member.renewals().map(|renewals| renewals.used()), Some(1));
    assert!(found.member.last_session().is_none());
}

#[test]
fn a_task_started_anew_before_its_last_run_started_is_refused() {
    let mut state = old_state();
    let (_, mut member) = state.find_task_mut(&task()).expect("the task");
    renewed(member.value);
    member
        .end_run(started(), RunEnd::NoStatus { exit_code: Some(0) })
        .expect("the run ends");

    let refused = member.renew(Trouble::Hang, started());

    assert!(matches!(refused, Err(RenewalError::NotAfterTheLastRun { .. })));
}

#[test]
fn a_state_whose_renewal_does_not_follow_its_run_is_refused() {
    let mut state = old_state();
    let (_, mut member) = state.find_task_mut(&task()).expect("the task");
    renewed(member.value);
    let mut written: serde_json::Value = serde_json::from_str(&state.to_json().expect("saves")).expect("json");
    let mut slot = &mut written;
    for key in ["teams", "0", "crew", "members", "0", "renewals", "at"] {
        slot = slot.get_mut(key).expect("the renewal's time");
    }
    *slot = serde_json::Value::from("2026-11-20T19:00:00Z");

    let refused = State::from_json(&written.to_string());

    assert!(refused.is_err());
}
