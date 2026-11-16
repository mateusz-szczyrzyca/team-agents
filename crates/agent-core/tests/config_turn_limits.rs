//! Per-run turn budgets and the number of consecutive session continuations.

use agent_core::config::{Config, ConfigError};

#[test]
fn turn_limit_defaults_are_300_turns_and_three_resumes() {
    let config = Config::from_toml_str("").unwrap();
    assert_eq!(config.limits().max_turns.get(), 300);
    assert_eq!(config.limits().turn_limit_resumes, 3);
}

#[test]
fn turn_limits_accept_boundaries_and_zero_resumes_disables_continuation() {
    for (turns, resumes) in [(1, 0), (50, 1), (u32::MAX, u32::MAX)] {
        let config = Config::from_toml_str(&format!(
            "[limits]\nmax_turns = {turns}\nturn_limit_resumes = {resumes}\n"
        ))
        .unwrap();
        assert_eq!(config.limits().max_turns.get(), turns);
        assert_eq!(config.limits().turn_limit_resumes, resumes);
    }
}

#[test]
fn turn_limits_reject_zero_turns_negative_fractional_and_overflowing_values() {
    for setting in [
        "max_turns = 0",
        "max_turns = -1",
        "max_turns = 1.5",
        "max_turns = 4294967296",
        "turn_limit_resumes = -1",
        "turn_limit_resumes = 1.5",
        "turn_limit_resumes = 4294967296",
    ] {
        let error = Config::from_toml_str(&format!("[limits]\n{setting}\n")).unwrap_err();
        assert!(matches!(error, ConfigError::Parse(_)), "{setting}: {error:?}");
    }
}
