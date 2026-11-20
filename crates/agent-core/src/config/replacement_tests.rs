//! `[replacement]`: whether a task whose runs go bad is started in a new session, and how often.

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
fn a_file_without_replacement_starts_a_new_session_once() {
    let config = Config::from_toml_str(one_agent()).unwrap();

    assert_eq!(config.replacement().new_sessions().get(), 1);
}

#[test]
fn the_replacement_table_says_how_many_new_sessions_a_task_has() {
    let config = Config::from_toml_str(&with("[replacement]\nnew_sessions = 3\n")).unwrap();
    assert_eq!(config.replacement().new_sessions().get(), 3);

    let off = Config::from_toml_str(&with("[replacement]\nnew_sessions = 0\n")).unwrap();
    assert_eq!(off.replacement().new_sessions().get(), 0);
}

#[test]
fn more_new_sessions_than_a_task_can_take_are_refused() {
    let error = Config::from_toml_str(&with("[replacement]\nnew_sessions = 11\n")).unwrap_err();

    // The parser says which key was refused, at which line and column.
    assert!(
        matches!(&error, ConfigError::Parse(parse) if parse.span().is_some() && parse.message().contains("11")),
        "{error:?}"
    );
}

#[test]
fn a_negative_number_of_new_sessions_is_refused() {
    let error = Config::from_toml_str(&with("[replacement]\nnew_sessions = -1\n")).unwrap_err();

    assert!(matches!(error, ConfigError::Parse(_)), "{error:?}");
}
