//! Investigation configuration boundaries.
use super::{Config, ConfigError};

#[test]
fn answer_minutes_zero_is_refused() {
    assert!(matches!(
        Config::from_toml_str("[investigations]\nanswer_minutes = 0"),
        Err(ConfigError::Parse(_))
    ));
    assert!(matches!(
        Config::from_toml_str("[investigations]\nanswer_minutes = 30"),
        Ok(_)
    ));
}
