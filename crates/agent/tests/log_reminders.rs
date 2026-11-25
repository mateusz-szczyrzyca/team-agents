//! `agent log --reminders` as a process: the table of the reminders is read from the history without a daemon.
#[cfg(test)]
#[path = "support/reminder_history.rs"]
mod reminder_history;

use rig::{home_with_reminders, log, stdout_of};

/// The table of the history of `reminder_history` counted over a week.
const TABLE: &str = "agent     model       commit  context  followed  median  cuts\n\
                     claude-1  sonnet-5-5  3       1        2         9 min   1\n";

/// The helpers of the tests, which panic as the tests do.
#[cfg(test)]
mod rig {
    use std::path::Path;
    use std::process::{Command, Output};

    use agent_core::state::Timestamp;
    use agent_runtime::clock::SystemClock;

    use super::reminder_history;

    /// Runs `agent log` with `args` against the home `home`.
    pub(crate) fn log(home: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_agent"))
            .env("AGENTS_HOME", home)
            .arg("log")
            .args(args)
            .output()
            .unwrap()
    }

    /// What `output` printed; the command is told to have succeeded.
    pub(crate) fn stdout_of(output: &Output) -> String {
        assert!(
            output.status.success(),
            "agent log failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout.clone()).unwrap()
    }

    /// The home of a history of the reminders of the last three days.
    pub(crate) fn home_with_reminders() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let now = Timestamp::now(&SystemClock).unwrap();
        reminder_history::write_history(&dir.path().join("history.db"), now);
        dir
    }
}

#[test]
fn the_reminders_of_a_week_are_a_table_of_the_agents() {
    let dir = home_with_reminders();
    assert_eq!(stdout_of(&log(dir.path(), &["--reminders"])), TABLE);
    assert_eq!(stdout_of(&log(dir.path(), &["--reminders", "--days", "7"])), TABLE);
}

#[test]
fn a_history_of_the_version_before_the_log_shows_the_empty_line() {
    let dir = tempfile::tempdir().unwrap();
    reminder_history::write_history_without_log(&dir.path().join("history.db"));
    assert_eq!(
        stdout_of(&log(dir.path(), &["--reminders"])),
        "no reminders in the last 7 days\n"
    );
}

#[test]
fn zero_days_show_the_empty_line() {
    let dir = home_with_reminders();
    assert_eq!(
        stdout_of(&log(dir.path(), &["--reminders", "--days", "0"])),
        "no reminders in the last 0 days\n"
    );
}

#[test]
fn the_reminders_take_no_scope_filter_or_format() {
    let dir = home_with_reminders();
    for extra in [
        ["--round", "round-20261125-100000-aaaa"],
        ["--task", "task-20261125-100000-tttt"],
        ["--team", "team-20261125-100000-mmmm"],
    ] {
        let output = log(dir.path(), &["--reminders", extra[0], extra[1]]);
        assert_eq!(output.status.code(), Some(1), "{extra:?}");
    }
    assert_eq!(log(dir.path(), &["--reminders", "--json"]).status.code(), Some(1));
}

#[test]
fn days_without_the_reminders_is_refused() {
    let dir = home_with_reminders();
    let output = log(dir.path(), &["--days", "3"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--reminders"));
}
