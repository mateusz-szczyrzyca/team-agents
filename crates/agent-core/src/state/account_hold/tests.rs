//! The tasks the pause of an account holds, as the state keeps them: they are woken by the end of that pause only.
//! They are absent from states written before them.

use super::*;
use crate::config::AccountName;
use crate::decide::limit::Limit;

/// A task ID of these tests.
fn task(text: &str) -> TaskId {
    text.parse().unwrap()
}

/// The state of the item before this one: it has no held task, and a paused account.
fn state_with_a_paused_account() -> State {
    let mut state = State::from_json(include_str!("../../../tests/fixtures/state-before-account-pauses.json"))
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
fn a_held_task_is_listed_and_survives_a_round_trip() {
    let mut state = state_with_a_paused_account();
    state.hold_task_for_account(task("task-20261127-100000-aaaa"));
    state.hold_task_for_account(task("task-20261127-100000-bbbb"));

    let json = state.to_json().unwrap();
    let back = State::from_json(&json).unwrap();

    let held: Vec<&TaskId> = back.tasks_held_by_account().collect();
    assert_eq!(
        held,
        [&task("task-20261127-100000-aaaa"), &task("task-20261127-100000-bbbb")]
    );
    assert_eq!(back, state);
}

#[test]
fn a_released_task_is_no_longer_held_and_the_others_stay() {
    let mut state = state_with_a_paused_account();
    state.hold_task_for_account(task("task-20261127-100000-aaaa"));
    state.hold_task_for_account(task("task-20261127-100000-bbbb"));

    state.release_task_from_account(&task("task-20261127-100000-aaaa"));

    let held: Vec<&TaskId> = state.tasks_held_by_account().collect();
    assert_eq!(held, [&task("task-20261127-100000-bbbb")]);
    state.release_task_from_account(&task("task-20261127-100000-bbbb"));
    assert!(!state.to_json().unwrap().contains("account_held_tasks"));
}

#[test]
fn a_state_with_a_paused_account_and_no_held_task_loads() {
    let json = state_with_a_paused_account().to_json().unwrap();
    assert!(!json.contains("account_held_tasks"), "{json}");

    let state = State::from_json(&json).unwrap();

    assert_eq!(state.tasks_held_by_account().count(), 0);
    let paused: Vec<&AccountName> = state.account_pauses().map(|(name, _)| name).collect();
    assert_eq!(paused, [&"comp-claude".parse::<AccountName>().unwrap()]);
}
