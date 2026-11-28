//! The holds of accounts as the state records them: until when each lasts. They are absent from states written before
//! them.

use super::*;
use crate::config::AccountName;

/// The account of these tests.
fn account(name: &str) -> AccountName {
    name.parse().unwrap()
}

/// A moment of these tests.
fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

/// The state written before the holds were recorded: it keeps a reading of an account and no hold.
fn old_state() -> State {
    State::from_json(include_str!("../../tests/fixtures/state-before-account-holds.json"))
        .expect("a state of main loads")
}

#[test]
fn old_state_without_holds_loads_and_writes_none() {
    let state = old_state();

    assert_eq!(state.account_hold_until(&account("comp-claude")), None);
    assert_eq!(state.account_holds().count(), 0);
    assert_eq!(state.limit_readings().live(at("2026-11-28T10:00:00Z")).len(), 1);
    let json = state.to_json().unwrap();
    assert!(!json.contains("account_holds"), "{json}");
    assert_eq!(State::from_json(&json).unwrap(), state);
}

#[test]
fn a_recorded_hold_survives_a_round_trip() {
    let mut state = old_state();
    state.record_account_hold(account("comp-claude"), Some(at("2026-11-28T15:30:00Z")));
    state.record_account_hold(account("home-codex"), Some(at("2026-11-29T00:00:00Z")));

    let back = State::from_json(&state.to_json().unwrap()).unwrap();

    assert_eq!(back, state);
    assert_eq!(
        back.account_hold_until(&account("comp-claude")),
        Some(at("2026-11-28T15:30:00Z"))
    );
    assert_eq!(
        back.account_holds().collect::<Vec<_>>(),
        vec![
            (&account("comp-claude"), at("2026-11-28T15:30:00Z")),
            (&account("home-codex"), at("2026-11-29T00:00:00Z")),
        ]
    );
    assert_eq!(back.account_hold_until(&account("other")), None);
}

#[test]
fn forgetting_a_hold_removes_it_from_the_state_and_the_file() {
    let mut state = old_state();
    state.record_account_hold(account("comp-claude"), Some(at("2026-11-28T15:30:00Z")));

    state.record_account_hold(account("comp-claude"), None);

    assert_eq!(state.account_hold_until(&account("comp-claude")), None);
    assert!(!state.to_json().unwrap().contains("account_holds"));
}
