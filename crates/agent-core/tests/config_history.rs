//! `[history]` of the configuration: whether the daemon records what happened and how long a text it keeps.

use agent_core::config::{Config, ConfigError};
use agent_core::history::DEFAULT_MAX_TEXT_BYTES;

/// The example configuration users copy.
const EXAMPLE: &str = include_str!("../../../config/agents.example.toml");

#[test]
fn without_the_section_the_history_is_written_and_keeps_a_mebibyte_of_text() {
    let config = Config::from_toml_str("").unwrap();

    let history = config.history();
    assert!(history.enabled);
    assert_eq!(history.max_text_bytes.get(), DEFAULT_MAX_TEXT_BYTES);
}

#[test]
fn the_section_turns_the_history_off_and_sets_the_longest_text() {
    let config = Config::from_toml_str("[history]\nenabled = false\nmax_text_bytes = 65536\n").unwrap();

    let history = config.history();
    assert!(!history.enabled);
    assert_eq!(history.max_text_bytes.get(), 65_536);
}

#[test]
fn a_section_with_one_key_leaves_the_other_at_its_default() {
    let only_enabled = Config::from_toml_str("[history]\nenabled = false\n").unwrap();
    let only_limit = Config::from_toml_str("[history]\nmax_text_bytes = 100\n").unwrap();

    assert_eq!(only_enabled.history().max_text_bytes.get(), DEFAULT_MAX_TEXT_BYTES);
    assert!(only_limit.history().enabled);
    assert_eq!(only_limit.history().max_text_bytes.get(), 100);
}

#[test]
fn a_limit_of_zero_or_above_the_largest_is_refused() {
    for text in [
        "[history]\nmax_text_bytes = 0\n",
        "[history]\nmax_text_bytes = 268435457\n",
        "[history]\nmax_text_bytes = -5\n",
        "[history]\nmax_text_bytes = \"big\"\n",
    ] {
        let error = Config::from_toml_str(text).unwrap_err();

        assert!(matches!(error, ConfigError::Parse(_)), "{text}: {error:?}");
    }
}

#[test]
fn an_unknown_key_and_a_wrong_type_are_refused() {
    for text in ["[history]\nenable = true\n", "[history]\nenabled = \"yes\"\n"] {
        let error = Config::from_toml_str(text).unwrap_err();

        assert!(matches!(error, ConfigError::Parse(_)), "{text}: {error:?}");
    }
}

#[test]
fn the_example_configuration_loads_with_the_defaults_of_the_history() {
    let config = Config::from_toml_str(EXAMPLE).unwrap();

    assert!(config.history().enabled);
    assert_eq!(config.history().max_text_bytes.get(), DEFAULT_MAX_TEXT_BYTES);
    assert!(
        EXAMPLE.contains("[history]"),
        "the example documents the section so that users find it"
    );
}
