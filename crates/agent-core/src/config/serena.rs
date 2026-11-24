//! `[serena]`: after how many searches of code without Serena a run is reminded.

use super::{Config, ConfigError, remind_after};
use crate::serena::{DEFAULT_REMIND_AFTER, RemindAfter};

/// The text of a configuration with the agents the tests name.
fn config_with(section: &str) -> String {
    format!("[agents.claude-1]\ncli = \"claude\"\n\n[tiers]\nnormal = [\"claude-1\"]\n\n{section}")
}

#[test]
fn a_file_without_the_section_keeps_the_default_of_forty_searches() {
    let config = Config::from_toml_str(&config_with("")).expect("a file without [serena] loads");

    assert_eq!(config.serena().reminder(), Some(RemindAfter::default()));
    assert_eq!(remind_after(&config).map(RemindAfter::get), Some(40));
    assert_eq!(DEFAULT_REMIND_AFTER, 40);
}

#[test]
fn the_section_sets_the_count_at_which_a_run_is_reminded() {
    let config = Config::from_toml_str(&config_with("[serena]\nremind_after = 12\n")).expect("12 loads");

    assert_eq!(config.serena().remind_after(), 12);
    assert_eq!(remind_after(&config).map(RemindAfter::get), Some(12));
}

#[test]
fn zero_turns_the_reminder_off() {
    let config = Config::from_toml_str(&config_with("[serena]\nremind_after = 0\n")).expect("0 turns the reminder off");

    assert_eq!(remind_after(&config), None);
}

#[test]
fn a_key_of_the_section_that_is_not_its_own_is_refused() {
    let err = Config::from_toml_str(&config_with("[serena]\nremind_after = 5\necho = true\n"))
        .expect_err("[serena] has no key echo");

    assert!(
        matches!(&err, ConfigError::Parse(_)),
        "a key the section does not have is refused as the file's parse: {err:?}"
    );
}

#[test]
fn a_count_that_is_negative_is_refused() {
    let err =
        Config::from_toml_str(&config_with("[serena]\nremind_after = -1\n")).expect_err("a count is never negative");

    assert!(matches!(&err, ConfigError::Parse(_)), "{err:?}");
}
