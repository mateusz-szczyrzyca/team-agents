//! The pauses of accounts as the state keeps them: until when and for which limit. They are absent from states written
//! before them.

use serde_json::error::Category;
use time::Duration;

use super::*;
use crate::config::AccountName;
use crate::decide::limit::Limit;

/// The account of these tests.
fn account(name: &str) -> AccountName {
    name.parse().unwrap()
}

/// A moment of these tests.
fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

/// The state written before the pauses of accounts; it has none.
fn old_state() -> State {
    State::from_json(include_str!("../../tests/fixtures/state-before-account-pauses.json"))
        .expect("a state of main loads")
}

#[test]
fn old_state_without_accounts_loads() {
    let state = old_state();

    assert_eq!(state.account_pause(&account("comp-claude")), None);
    let json = state.to_json().unwrap();
    assert!(!json.contains("\"accounts\""), "{json}");
    assert_eq!(State::from_json(&json).unwrap(), state);
}

#[test]
fn state_with_paused_account_round_trips() {
    let mut state = old_state();
    let quota = AccountPause {
        until: at("2026-11-27T11:01:00Z"),
        reason: Limit::Quota {
            reset_in: Some(Duration::minutes(30) + Duration::milliseconds(250)),
        },
    };
    let rate = AccountPause {
        until: at("2026-11-27T10:03:00Z"),
        reason: Limit::RateLimit,
    };
    state.record_account_pause(account("comp-claude"), Some(quota));
    state.record_account_pause(account("home-codex"), Some(rate));

    let json = state.to_json().unwrap();
    let back = State::from_json(&json).unwrap();

    assert_eq!(back.account_pause(&account("comp-claude")), Some(&quota));
    assert_eq!(back.account_pause(&account("home-codex")), Some(&rate));
    assert_eq!(back.account_pause(&account("other")), None);
    assert_eq!(back, state);
}

#[test]
fn a_recorded_pause_is_forgotten_with_none() {
    let mut state = old_state();
    let pause = AccountPause {
        until: at("2026-11-27T10:03:00Z"),
        reason: Limit::RateLimit,
    };
    state.record_account_pause(account("comp-claude"), Some(pause));

    state.record_account_pause(account("comp-claude"), None);

    assert_eq!(state.account_pause(&account("comp-claude")), None);
    assert!(!state.to_json().unwrap().contains("\"accounts\""));
}

#[test]
fn a_limit_is_written_and_read_in_each_of_its_forms() {
    for limit in [
        Limit::RateLimit,
        Limit::Quota { reset_in: None },
        Limit::Quota {
            reset_in: Some(Duration::seconds(90)),
        },
    ] {
        let json = serde_json::to_string(&limit).unwrap();

        assert_eq!(serde_json::from_str::<Limit>(&json).unwrap(), limit, "{json}");
    }
}

#[test]
fn a_quota_reset_that_is_not_positive_is_refused() {
    for json in [
        r#"{"kind":"quota","reset_in_nanoseconds":0}"#,
        r#"{"kind":"quota","reset_in_nanoseconds":-5}"#,
    ] {
        let error = serde_json::from_str::<Limit>(json).unwrap_err();

        assert_eq!(error.classify(), Category::Data, "{json}");
    }
}

#[test]
fn an_unknown_kind_of_limit_is_refused() {
    let error = serde_json::from_str::<Limit>(r#"{"kind":"overload"}"#).unwrap_err();

    assert_eq!(error.classify(), Category::Data);
}

#[test]
fn an_unknown_key_of_a_paused_account_is_refused() {
    let json = r#"{"until":"2026-11-27T10:03:00Z","reason":{"kind":"rate_limit"},"extra":1}"#;

    let error = serde_json::from_str::<AccountPause>(json).unwrap_err();

    assert_eq!(error.classify(), Category::Data);
}
