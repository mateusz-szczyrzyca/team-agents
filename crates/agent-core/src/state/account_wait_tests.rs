//! The tasks that wait for the pause of another account than their own agent's, as the state keeps them: after a
//! restart they are woken at the end of that pause, not of their own agent's.

use super::*;
use crate::config::AccountName;
use crate::decide::limit::Limit;

/// A task ID of these tests.
fn task(text: &str) -> TaskId {
    text.parse().unwrap()
}

/// A state of main with a paused account.
fn state_with_a_paused_account() -> State {
    let mut state = State::from_json(include_str!("../../tests/fixtures/state-before-account-pauses.json"))
        .expect("a state of main loads");
    let account: AccountName = "comp-claude".parse().unwrap();
    let pause = AccountPause {
        until: "2026-11-27T11:01:00Z".parse().unwrap(),
        reason: Limit::RateLimit,
    };
    state.record_account_pause(account, Some(pause));
    state
}

#[test]
fn a_waiting_task_keeps_its_wake_time_over_a_round_trip() {
    let mut state = state_with_a_paused_account();
    state.hold_task_for_account(task("task-20261127-100000-aaaa"));
    state.hold_task_waiting(
        task("task-20261127-100000-bbbb"),
        "2026-11-27T12:00:00Z".parse().unwrap(),
    );

    let back = State::from_json(&state.to_json().unwrap()).unwrap();

    assert_eq!(
        back.wait_of_task(&task("task-20261127-100000-bbbb")),
        Some("2026-11-27T12:00:00Z".parse().unwrap())
    );
    assert_eq!(back.wait_of_task(&task("task-20261127-100000-aaaa")), None);
    assert_eq!(back.tasks_held_by_account().count(), 2);
    assert_eq!(back, state);
}

#[test]
fn a_released_task_waits_for_nothing_and_the_state_omits_the_waits() {
    let mut state = state_with_a_paused_account();
    state.hold_task_waiting(
        task("task-20261127-100000-bbbb"),
        "2026-11-27T12:00:00Z".parse().unwrap(),
    );

    state.release_task_from_account(&task("task-20261127-100000-bbbb"));

    assert_eq!(state.wait_of_task(&task("task-20261127-100000-bbbb")), None);
    assert!(!state.to_json().unwrap().contains("account_waits"));
}
