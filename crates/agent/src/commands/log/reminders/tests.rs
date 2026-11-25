//! The table of `agent log --reminders`: its text, and what is printed from a history.

#[path = "../../../../tests/support/reminder_history.rs"]
mod reminder_history;

use agent_core::decide::reminders::ReminderLine;
use agent_core::state::Timestamp;

use super::{print_reminders, render};

/// The golden table of the history of `reminder_history` read with 7 days.
const TABLE: &str = "agent     model       commit  context  followed  median  cuts\n\
                     claude-1  sonnet-5-5  3       1        2         9 min   1\n";

fn now() -> Timestamp {
    "2026-11-25T12:00:00Z".parse().unwrap()
}

fn line(agent: &str, model: Option<&str>, median: Option<u64>) -> ReminderLine {
    ReminderLine {
        agent: agent.parse().unwrap(),
        model: model.map(str::to_owned),
        commit: 3,
        context: 1,
        followed: 2,
        median_minutes: median,
        cuts: 1,
    }
}

#[test]
fn the_table_has_its_columns_padded_by_two_spaces() {
    let text = render(&[line("claude-1", Some("sonnet-5-5"), Some(9))], 7);
    assert_eq!(text, TABLE);
}

#[test]
fn a_column_is_as_wide_as_its_widest_cell_and_a_missing_model_or_median_is_a_dash() {
    let text = render(
        &[
            line("claude-1", Some("sonnet-5-5"), Some(120)),
            line("codex-1", None, None),
        ],
        7,
    );
    assert_eq!(
        text,
        "agent     model       commit  context  followed  median   cuts\n\
         claude-1  sonnet-5-5  3       1        2         120 min  1\n\
         codex-1   -           3       1        2         -        1\n"
    );
}

#[test]
fn no_lines_is_one_sentence_naming_the_days() {
    assert_eq!(render(&[], 7), "no reminders in the last 7 days\n");
    assert_eq!(render(&[], 1), "no reminders in the last 1 day\n");
    assert_eq!(render(&[], 0), "no reminders in the last 0 days\n");
}

/// What `print_reminders` prints for the history of `reminder_history` at `now` with `days`.
fn printed(days: u32) -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    reminder_history::write_history(&path, now());
    let mut out = Vec::new();
    print_reminders(&path, days, now(), &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

#[test]
fn a_history_with_reminders_prints_the_table_of_its_rows() {
    assert_eq!(printed(7), TABLE);
}

#[test]
fn the_rows_older_than_the_days_are_not_counted() {
    // The first reminder is 60 hours old, the followed one 59: two days and a half.
    assert_eq!(
        printed(2),
        "agent     model       commit  context  followed  median  cuts\n\
         claude-1  sonnet-5-5  2       1        1         12 min  1\n"
    );
}

#[test]
fn zero_days_print_the_empty_line() {
    assert_eq!(printed(0), "no reminders in the last 0 days\n");
}

#[test]
fn a_history_of_the_version_before_the_log_prints_the_empty_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    reminder_history::write_history_without_log(&path);
    let mut out = Vec::new();
    print_reminders(&path, 7, now(), &mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "no reminders in the last 7 days\n");
}
