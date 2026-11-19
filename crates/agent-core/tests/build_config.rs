//! Managed builds default to small caches while existing configuration files remain readable.

use agent_core::config::{Config, ConfigError};

#[test]
fn incremental_defaults_off_in_old_and_empty_build_configurations() {
    for text in ["", "[build]\n", include_str!("fixtures/config-before-build.toml")] {
        let config = Config::from_toml_str(text).unwrap();
        assert!(!config.build().incremental, "configuration: {text}");
    }
}

#[test]
fn incremental_accepts_explicit_boolean_overrides() {
    for value in [false, true] {
        let config = Config::from_toml_str(&format!("[build]\nincremental = {value}\n")).unwrap();
        assert_eq!(config.build().incremental, value);
    }
}

#[test]
fn build_rejects_unknown_keys_and_non_boolean_incremental_values() {
    for text in [
        "[build]\nunknown = true",
        "[build]\nincremental = 1",
        "[build]\nincremental = \"false\"",
    ] {
        assert!(matches!(Config::from_toml_str(text), Err(ConfigError::Parse(_))));
    }
}
