//! Defaults and inclusive bounds of the findings configuration.
use super::*;
use crate::config::{Config, ConfigError};

fn config(table: &str) -> Result<Config, ConfigError> {
    Config::from_toml_str(&format!(
        "[agents.agent-a]\ncli = \"codex\"\n[tiers]\nnormal = [\"agent-a\"]\nworkers = []\n{table}"
    ))
}

#[test]
fn missing_section_and_each_missing_key_keep_defaults() {
    for table in [
        "",
        "[findings]",
        "[findings]\nkeep_days = 30",
        "[findings]\nmax_shown = 8",
        "[findings]\nmax_chars = 3000",
    ] {
        let settings = *config(table).unwrap().findings();
        assert_eq!(
            (settings.keep_days(), settings.max_shown(), settings.max_chars()),
            (30, 8, 3000)
        );
    }
}

#[test]
fn each_findings_bound_is_inclusive() {
    for (key, values) in [
        ("keep_days", [0, 365]),
        ("max_shown", [1, 20]),
        ("max_chars", [500, 10000]),
    ] {
        for value in values {
            let settings = *config(&format!("[findings]\n{key} = {value}")).unwrap().findings();
            let actual = match key {
                "keep_days" => settings.keep_days(),
                "max_shown" => settings.max_shown(),
                "max_chars" => settings.max_chars(),
                _ => panic!("table key"),
            };
            assert_eq!(i64::from(actual), value, "{key}");
        }
    }
}

#[test]
fn each_out_of_bounds_value_identifies_the_key_and_value() {
    for (key, name, values) in [
        (FindingsKey::KeepDays, "keep_days", [-1, 366, i64::MAX]),
        (FindingsKey::MaxShown, "max_shown", [0, 21, -1]),
        (FindingsKey::MaxChars, "max_chars", [499, 10001, -1]),
    ] {
        for value in values {
            let err = config(&format!("[findings]\n{name} = {value}")).expect_err("outside the bound");
            assert!(
                matches!(err, ConfigError::Findings(InvalidFindings { key: actual_key, value: actual_value, .. }) if actual_key == key && actual_value == value)
            );
        }
        assert_eq!(key.to_string(), name);
    }
}

#[test]
fn unknown_findings_key_and_non_numeric_value_are_rejected() {
    for table in ["[findings]\nunknown = 1", "[findings]\nmax_shown = \"eight\""] {
        assert!(matches!(config(table), Err(ConfigError::Parse(_))));
    }
}
