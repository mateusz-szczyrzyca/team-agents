use super::*;

/// The head of the worktree in the tests.
const HEAD: &str = "1111111111111111111111111111111111111111";
/// An older head the run was reminded at.
const OLDER: &str = "2222222222222222222222222222222222222222";

fn at(text: &str) -> Timestamp {
    text.parse().expect("a time of the tests")
}

fn commit(text: &str) -> CommitId {
    text.parse().expect("a commit ID of the tests")
}

/// A look at 12:00 of a run started at 10:00 whose last commit was `minutes_ago` minutes before, with `uncommitted` lines.
fn look(uncommitted: u64, minutes_ago: i64) -> CommitLook {
    let now = at("2026-11-25T12:00:00Z");
    let last = Timestamp::from_utc(now.as_utc() - time::Duration::minutes(minutes_ago)).expect("a time in range");
    CommitLook {
        uncommitted,
        last_commit: last,
        run_started: at("2026-11-25T10:00:00Z"),
        now,
        head: commit(HEAD),
        reminded_at: None,
    }
}

#[test]
fn the_bounds_decide_by_both_at_once() {
    // (uncommitted lines, minutes since the last commit, what follows) under lines = 300, minutes = 30.
    let table = [
        (299, 40, ReminderDue::NotDue),
        (
            300,
            30,
            ReminderDue::Due {
                lines: 300,
                minutes: 30,
            },
        ),
        (300, 29, ReminderDue::NotDue),
        (
            500,
            90,
            ReminderDue::Due {
                lines: 500,
                minutes: 90,
            },
        ),
        (0, 90, ReminderDue::NotDue),
    ];
    for (lines, minutes, expected) in table {
        assert_eq!(
            reminder_due(&look(lines, minutes), 300, 30),
            expected,
            "{lines} lines, {minutes} minutes"
        );
    }
}

#[test]
fn a_bound_of_zero_turns_the_reminder_off() {
    assert_eq!(reminder_due(&look(400, 90), 0, 30), ReminderDue::NotDue);
    assert_eq!(reminder_due(&look(400, 90), 300, 0), ReminderDue::NotDue);
}

#[test]
fn the_time_before_the_run_started_is_not_idle_time() {
    // The last commit was two hours ago, the run began ten minutes ago: the run has been idle for ten.
    let mut seen = look(400, 120);
    seen.run_started = at("2026-11-25T11:50:00Z");

    assert_eq!(reminder_due(&seen, 300, 30), ReminderDue::NotDue);
    assert_eq!(
        reminder_due(&seen, 300, 10),
        ReminderDue::Due {
            lines: 400,
            minutes: 10
        }
    );
}

#[test]
fn a_run_reminded_at_the_head_is_not_reminded_again() {
    let mut seen = look(400, 90);
    seen.reminded_at = Some(commit(HEAD));

    assert_eq!(reminder_due(&seen, 300, 30), ReminderDue::NotDue);
}

#[test]
fn a_run_reminded_at_an_older_head_is_reminded_again() {
    let mut seen = look(400, 90);
    seen.reminded_at = Some(commit(OLDER));

    assert_eq!(
        reminder_due(&seen, 300, 30),
        ReminderDue::Due {
            lines: 400,
            minutes: 90
        }
    );
}

#[test]
fn a_last_commit_after_now_counts_as_no_idle_time() {
    assert_eq!(reminder_due(&look(400, -5), 300, 1), ReminderDue::NotDue);
}
