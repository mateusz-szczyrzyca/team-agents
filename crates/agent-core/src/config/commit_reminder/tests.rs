//! `[commit_reminder]`: the defaults of a file without it, the values, and what is refused.

use super::super::{Config, ConfigError};
use super::DEFAULT_CUT_AFTER;
use super::{DEFAULT_CONTEXT_PERCENT, DEFAULT_LINES, DEFAULT_MINUTES};

/// The smallest configuration: one normal agent, which is all a task needs.
fn one_agent() -> &'static str {
    "[agents.claude-1]\ncli = \"claude\"\n\n[tiers]\nnormal = [\"claude-1\"]\nworkers = []\n"
}

fn with(table: &str) -> String {
    format!("{}{table}", one_agent())
}

#[test]
fn a_file_without_the_section_reminds_at_300_lines_and_30_minutes() {
    let config = Config::from_toml_str(one_agent()).expect("a file without [commit_reminder] loads");

    assert_eq!(config.commit_reminder().lines, 300);
    assert_eq!(config.commit_reminder().minutes, 30);
    assert_eq!((DEFAULT_LINES, DEFAULT_MINUTES), (300, 30));
}

#[test]
fn the_section_sets_both_bounds() {
    let config = Config::from_toml_str(&with("[commit_reminder]\nlines = 120\nminutes = 10\n")).expect("it loads");

    assert_eq!(config.commit_reminder().lines, 120);
    assert_eq!(config.commit_reminder().minutes, 10);
}

#[test]
fn a_key_left_out_keeps_its_default() {
    let config = Config::from_toml_str(&with("[commit_reminder]\nlines = 120\n")).expect("it loads");

    assert_eq!(config.commit_reminder().lines, 120);
    assert_eq!(config.commit_reminder().minutes, 30);
}

#[test]
fn a_key_at_zero_is_accepted_and_turns_the_reminder_off() {
    let config = Config::from_toml_str(&with("[commit_reminder]\nlines = 0\nminutes = 0\n")).expect("zero loads");

    assert_eq!(config.commit_reminder().lines, 0);
    assert_eq!(config.commit_reminder().minutes, 0);
}

#[test]
fn a_key_the_section_does_not_have_is_refused() {
    let err = Config::from_toml_str(&with("[commit_reminder]\nlines = 5\necho = true\n"))
        .expect_err("[commit_reminder] has no key echo");

    assert!(matches!(&err, ConfigError::Parse(_)), "{err:?}");
}

#[test]
fn a_negative_bound_is_refused() {
    let err = Config::from_toml_str(&with("[commit_reminder]\nminutes = -1\n")).expect_err("a bound is never negative");

    assert!(matches!(&err, ConfigError::Parse(_)), "{err:?}");
}

#[test]
fn the_template_names_the_section_with_the_defaults() {
    let template = include_str!("../../../../../config/agents.example.toml");
    let config = Config::from_toml_str(template).expect("the example loads");

    assert!(template.contains("[commit_reminder]"));
    assert_eq!(config.commit_reminder().lines, DEFAULT_LINES);
    assert_eq!(config.commit_reminder().minutes, DEFAULT_MINUTES);
}

#[test]
fn a_file_without_the_section_reminds_a_session_at_80_percent_of_its_bound() {
    let config = Config::from_toml_str(one_agent()).expect("a file without [commit_reminder] loads");

    assert_eq!(config.commit_reminder().context_percent, 80);
    assert_eq!(DEFAULT_CONTEXT_PERCENT, 80);
}

#[test]
fn the_section_sets_the_context_percent_from_0_to_100() {
    for percent in [0_u8, 1, 80, 100] {
        let config = Config::from_toml_str(&with(&format!("[commit_reminder]\ncontext_percent = {percent}\n")))
            .expect("a percent up to 100 loads");

        assert_eq!(config.commit_reminder().context_percent, percent);
        assert_eq!(
            config.commit_reminder().lines,
            300,
            "the other keys keep their defaults"
        );
    }
}

#[test]
fn a_context_percent_above_100_is_refused() {
    let err = Config::from_toml_str(&with("[commit_reminder]\ncontext_percent = 101\n"))
        .expect_err("a session cannot hold more than all of its bound");

    assert!(
        matches!(&err, ConfigError::ContextPercentAbove { percent: 101 }),
        "{err:?}"
    );
}

#[test]
fn a_context_percent_that_is_no_byte_or_is_negative_is_refused() {
    for value in ["256", "-1"] {
        let err = Config::from_toml_str(&with(&format!("[commit_reminder]\ncontext_percent = {value}\n")))
            .expect_err("a percent is a small number");

        assert!(matches!(&err, ConfigError::Parse(_)), "{value}: {err:?}");
    }
}

#[test]
fn a_file_without_the_section_cuts_a_run_that_ignores_the_reminder_after_20_minutes() {
    let config = Config::from_toml_str(one_agent()).expect("a file without [commit_reminder] loads");

    assert_eq!(config.commit_reminder().cut_after, 20);
    assert_eq!(DEFAULT_CUT_AFTER, 20);
}

#[test]
fn the_section_sets_cut_after_and_zero_turns_the_cut_off() {
    let set = Config::from_toml_str(&with("[commit_reminder]\ncut_after = 5\n")).expect("it loads");
    let off = Config::from_toml_str(&with("[commit_reminder]\ncut_after = 0\n")).expect("zero loads");

    assert_eq!(set.commit_reminder().cut_after, 5);
    assert_eq!(off.commit_reminder().cut_after, 0);
    assert_eq!(off.commit_reminder().lines, 300);
}

#[test]
fn a_negative_cut_after_is_refused() {
    let err = Config::from_toml_str(&with("[commit_reminder]\ncut_after = -1\n")).expect_err("never negative");

    assert!(matches!(&err, ConfigError::Parse(_)), "{err:?}");
}
