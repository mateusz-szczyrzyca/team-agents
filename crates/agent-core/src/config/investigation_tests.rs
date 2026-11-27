//! Investigation configuration boundaries.
use super::{Config, ConfigError};

#[test]
fn answer_minutes_zero_is_refused() {
    assert!(matches!(
        Config::from_toml_str("[investigations]\nanswer_minutes = 0"),
        Err(ConfigError::Parse(_))
    ));
    assert_eq!(
        Config::from_toml_str("[investigations]\nanswer_minutes = 30")
            .unwrap()
            .investigations()
            .answer_minutes(),
        30
    );
}

#[test]
fn answer_minutes_accepts_only_the_documented_range_and_defaults_to_thirty() {
    assert_eq!(Config::from_toml_str("").unwrap().investigations().answer_minutes(), 30);
    for minutes in [10, 30, 1440] {
        let text = format!("[investigations]\nanswer_minutes = {minutes}");
        assert_eq!(
            Config::from_toml_str(&text).unwrap().investigations().answer_minutes(),
            minutes
        );
    }
    for minutes in [0, 9, 1441] {
        let text = format!("[investigations]\nanswer_minutes = {minutes}");
        assert!(matches!(Config::from_toml_str(&text), Err(ConfigError::Parse(_))));
    }
}
