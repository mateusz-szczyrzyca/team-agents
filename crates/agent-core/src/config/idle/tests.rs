//! Compatibility and boundaries of optional idle-team limits.

use crate::config::{Config, ConfigError};
use std::time::Duration;

#[test]
fn old_configuration_defaults_to_two_hours_and_one_restart() {
    let config = Config::from_toml_str("").unwrap();
    assert_eq!(
        config.limits().idle_team_minutes.as_duration(),
        Duration::from_secs(7200)
    );
    assert_eq!(config.limits().idle_team_restarts, 1);
}

#[test]
fn idle_limits_accept_disabled_and_largest_intervals() {
    for minutes in [0_u32, 1, 120, u32::MAX] {
        let config = Config::from_toml_str(&format!(
            "[limits]\nidle_team_minutes = {minutes}\nidle_team_restarts = {minutes}\n"
        ))
        .unwrap();
        assert_eq!(
            config.limits().idle_team_minutes.as_duration(),
            Duration::from_secs(u64::from(minutes) * 60)
        );
        assert_eq!(config.limits().idle_team_restarts, minutes);
    }
}

#[test]
fn idle_limits_reject_negative_fractional_and_overflowing_values() {
    for key in ["idle_team_minutes", "idle_team_restarts"] {
        for value in ["-1", "1.5", "4294967296"] {
            let error = Config::from_toml_str(&format!("[limits]\n{key} = {value}\n")).unwrap_err();
            assert!(matches!(error, ConfigError::Parse(_)));
        }
    }
}

#[test]
fn template_names_both_idle_limits_with_their_defaults() {
    let text = include_str!("../../../../../config/agents.example.toml");
    let value: toml::Value = toml::from_str(text).unwrap();
    let limits = value.get("limits").unwrap();
    assert_eq!(
        limits.get("idle_team_minutes").and_then(toml::Value::as_integer),
        Some(120)
    );
    assert_eq!(
        limits.get("idle_team_restarts").and_then(toml::Value::as_integer),
        Some(1)
    );
    let config = Config::from_toml_str(text).unwrap();
    assert_eq!(
        config.limits().idle_team_minutes.as_duration(),
        Duration::from_secs(7200)
    );
    assert_eq!(config.limits().idle_team_restarts, 1);
}
