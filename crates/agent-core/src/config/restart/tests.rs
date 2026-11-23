//! `[restart]`: which session a run cut by a restart of the daemon goes on in.

use super::super::{Config, ConfigError};
use super::RestartSession;

/// The smallest configuration: one normal agent, which is all a task needs.
fn one_agent() -> &'static str {
    "[agents.claude-1]\ncli = \"claude\"\n\n[tiers]\nnormal = [\"claude-1\"]\nworkers = []\n"
}

#[test]
fn a_file_without_restart_goes_on_in_a_new_session() {
    let config = Config::from_toml_str(one_agent()).unwrap();

    assert_eq!(config.restart().session, RestartSession::New);
}

#[test]
fn the_restart_table_names_the_session_a_cut_run_goes_on_in() {
    for (word, expected) in [("new", RestartSession::New), ("same", RestartSession::Same)] {
        let text = format!("{}\n[restart]\nsession = \"{word}\"\n", one_agent());

        let config = Config::from_toml_str(&text).unwrap();

        assert_eq!(config.restart().session, expected, "session = {word}");
    }
}

#[test]
fn an_empty_restart_table_keeps_the_default() {
    let config = Config::from_toml_str(&format!("{}\n[restart]\n", one_agent())).unwrap();

    assert_eq!(config.restart().session, RestartSession::New);
}

#[test]
fn a_session_that_is_neither_new_nor_same_is_refused() {
    let text = format!("{}\n[restart]\nsession = \"sometimes\"\n", one_agent());

    let refused = Config::from_toml_str(&text).unwrap_err();

    assert!(matches!(refused, ConfigError::Parse(_)), "{refused:?}");
}

#[test]
fn a_key_the_restart_table_does_not_know_is_refused() {
    let text = format!("{}\n[restart]\nsessions = \"new\"\n", one_agent());

    let refused = Config::from_toml_str(&text).unwrap_err();

    assert!(matches!(refused, ConfigError::Parse(_)), "{refused:?}");
}
