//! `[log]` of the configuration: which of the daemon's own log events are kept.

use agent_core::config::{Config, ConfigError};
use agent_core::log::LogLevel;

/// The example configuration users copy.
const EXAMPLE: &str = include_str!("../../../config/agents.example.toml");

#[test]
fn without_the_section_the_daemon_log_level_is_info() {
    let config = Config::from_toml_str("").unwrap();

    assert_eq!(config.log().level, LogLevel::Info);
}

#[test]
fn every_documented_log_level_loads() {
    for (text, expected) in [
        ("[log]\nlevel = \"error\"\n", LogLevel::Error),
        ("[log]\nlevel = \"warn\"\n", LogLevel::Warn),
        ("[log]\nlevel = \"info\"\n", LogLevel::Info),
        ("[log]\nlevel = \"debug\"\n", LogLevel::Debug),
    ] {
        let config = Config::from_toml_str(text).unwrap();

        assert_eq!(config.log().level, expected, "{text}");
    }
}

#[test]
fn an_unknown_level_or_key_in_log_is_refused() {
    for text in [
        "[log]\nlevel = \"trace\"\n",
        "[log]\nlevel = \"warning\"\n",
        "[log]\nlevel = 3\n",
        "[log]\nlevle = \"info\"\n",
    ] {
        let error = Config::from_toml_str(text).unwrap_err();

        assert!(matches!(error, ConfigError::Parse(_)), "{text}: {error:?}");
    }
}

#[test]
fn the_example_configuration_documents_the_log_section_with_its_default() {
    let config = Config::from_toml_str(EXAMPLE).unwrap();

    assert_eq!(config.log().level, LogLevel::Info);
    assert!(
        EXAMPLE.contains("[log]"),
        "the example documents the [log] section so that users find it"
    );
}
