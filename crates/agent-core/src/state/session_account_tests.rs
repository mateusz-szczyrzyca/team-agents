//! The account a task's session began under, as the state keeps it: it is absent from states written before accounts,
//! and a change of it starts a new session without counting a renewal.

use super::*;
use crate::config::AccountName;
use crate::decide::ladder::Trouble;

/// The failed lead of the state of `main` before renewals: it has seven runs and a session.
fn task() -> TaskId {
    "task-20261120-144101-r7q2".parse().unwrap()
}

/// A state written by `main` before accounts and before renewals existed.
fn old_state() -> State {
    State::from_json(include_str!("../../tests/fixtures/state-before-renewals.json")).expect("a state of main loads")
}

/// When the account changes, after every run of the fixture.
fn late() -> Timestamp {
    "2026-11-21T00:00:00Z".parse().unwrap()
}

/// The account named `name`.
fn account(name: &str) -> AccountName {
    name.parse().unwrap()
}

#[test]
fn old_state_without_session_account_loads() {
    let state = old_state();

    assert_eq!(state.session_account(&task()), None);
    let json = state.to_json().unwrap();
    assert!(!json.contains("session_accounts"), "{json}");
    assert_eq!(State::from_json(&json).unwrap(), state);
}

#[test]
fn a_changed_account_starts_a_new_session_and_counts_no_renewal() {
    let mut state = old_state();

    let begun = state
        .begin_account_session(&task(), Some(account("comp-claude")), late())
        .unwrap();

    assert!(begun);
    let member = state.find_task(&task()).unwrap().member;
    let renewals = member.renewals().expect("the new session is recorded");
    assert_eq!(renewals.reason(), Trouble::AccountChanged);
    assert_eq!(renewals.used(), 0, "no run went bad");
    assert!(member.awaits_new_session());
    assert_eq!(member.last_session(), None, "the session before is not resumed");
    assert_eq!(state.session_account(&task()), Some(&account("comp-claude")));
}

#[test]
fn the_account_of_a_session_survives_a_round_trip() {
    let mut state = old_state();
    state
        .begin_account_session(&task(), Some(account("comp-claude")), late())
        .unwrap();

    let back = State::from_json(&state.to_json().unwrap()).unwrap();

    assert_eq!(back.session_account(&task()), Some(&account("comp-claude")));
    assert_eq!(back, state);
}

#[test]
fn a_change_to_no_account_forgets_the_account() {
    let mut state = old_state();
    state.record_session_account(&task(), Some(account("comp-claude")));

    let begun = state.begin_account_session(&task(), None, late()).unwrap();

    assert!(begun);
    assert_eq!(state.session_account(&task()), None);
    assert!(!state.to_json().unwrap().contains("session_accounts"));
}

#[test]
fn recording_the_account_of_a_session_starts_no_session() {
    let mut state = old_state();

    state.record_session_account(&task(), Some(account("comp-claude")));

    assert_eq!(state.session_account(&task()), Some(&account("comp-claude")));
    let member = state.find_task(&task()).unwrap().member;
    assert_eq!(member.renewals(), None);
    assert!(member.last_session().is_some(), "the session goes on");
}

#[test]
fn removing_the_team_forgets_the_account_of_its_tasks() {
    let mut state = old_state();
    state.record_session_account(&task(), Some(account("comp-claude")));
    let team = state.find_task(&task()).unwrap().team.id().clone();

    assert_eq!(state.session_account(&task()), Some(&account("comp-claude")));

    state.remove_team(&team).expect("the team");

    assert_eq!(state.session_account(&task()), None);
    assert!(!state.to_json().unwrap().contains("session_accounts"));
}

#[test]
fn a_task_that_is_not_there_is_refused() {
    let mut state = old_state();
    let missing: TaskId = "task-20261120-144101-zzzz".parse().unwrap();

    let refused = state.begin_account_session(&missing, Some(account("comp-claude")), late());

    assert!(matches!(refused, Err(RenewTaskError::NoTask { .. })), "{refused:?}");
}
