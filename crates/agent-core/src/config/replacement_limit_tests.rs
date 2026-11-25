//! `[replacement] limit_retries` and `limit_wait_minutes`: the retries after a provider's refusal and the longest wait
//! for a named quota reset.

use super::*;

/// The smallest configuration: one normal agent, which is all a task needs.
fn one_agent() -> &'static str {
    "[agents.claude-1]\ncli = \"claude\"\n\n[tiers]\nnormal = [\"claude-1\"]\nworkers = []\n"
}

/// The configuration of [`one_agent`] with `table` after it.
fn with(table: &str) -> String {
    format!("{}{table}", one_agent())
}

#[test]
fn a_file_without_the_limit_keys_retries_once_and_waits_an_hour_for_a_named_reset() {
    let config = Config::from_toml_str(one_agent()).unwrap();

    assert_eq!(config.replacement().limit_retries().get(), 1);
    assert_eq!(config.replacement().limit_wait_minutes(), 60);
}

#[test]
fn the_limit_retries_key_takes_zero_to_ten() {
    let none = Config::from_toml_str(&with("[replacement]\nlimit_retries = 0\n")).unwrap();
    assert_eq!(none.replacement().limit_retries().get(), 0);

    let most = Config::from_toml_str(&with("[replacement]\nlimit_retries = 10\n")).unwrap();
    assert_eq!(most.replacement().limit_retries().get(), 10);
}

#[test]
fn more_limit_retries_than_ten_are_refused() {
    let error = Config::from_toml_str(&with("[replacement]\nlimit_retries = 11\n")).unwrap_err();

    assert!(
        matches!(&error, ConfigError::Parse(parse) if parse.span().is_some() && parse.message().contains("11")),
        "{error:?}"
    );
}

#[test]
fn the_limit_wait_minutes_key_takes_any_number_of_minutes_zero_included() {
    let zero = Config::from_toml_str(&with("[replacement]\nlimit_wait_minutes = 0\n")).unwrap();
    assert_eq!(zero.replacement().limit_wait_minutes(), 0);

    let long = Config::from_toml_str(&with("[replacement]\nlimit_wait_minutes = 90\n")).unwrap();
    assert_eq!(long.replacement().limit_wait_minutes(), 90);
}
