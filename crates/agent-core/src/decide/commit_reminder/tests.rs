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

#[test]
fn only_a_solo_agent_a_coordinator_and_a_member_are_reminded() {
    let table = [
        (Role::Solo, true),
        (Role::Coordinator, true),
        (Role::Member, true),
        (Role::Worker, false),
        (Role::Reviewer, false),
    ];
    for (role, expected) in table {
        assert_eq!(is_reminded(role), expected, "{role:?}");
    }
}

/// The time of the reminder in the tests of the cut: 12:00.
fn reminded_at_noon() -> Timestamp {
    at("2026-11-25T12:00:00Z")
}

/// The time `minutes` minutes after the reminder.
fn after_reminder(minutes: i64) -> Timestamp {
    Timestamp::from_utc(reminded_at_noon().as_utc() + time::Duration::minutes(minutes)).expect("a time in range")
}

#[test]
fn a_reminder_ignored_for_cut_after_minutes_cuts_the_run() {
    // (minutes since the reminder, what follows) under cut_after = 20.
    let table = [
        (0, CutDue::No),
        (19, CutDue::No),
        (20, CutDue::Cut { minutes: 20 }),
        (35, CutDue::Cut { minutes: 35 }),
    ];
    for (minutes, expected) in table {
        assert_eq!(
            cut_due(reminded_at_noon(), after_reminder(minutes), 20, false, false, false),
            expected,
            "{minutes} minutes"
        );
    }
}

#[test]
fn a_part_of_a_minute_does_not_count_toward_the_cut() {
    let now = Timestamp::from_utc(after_reminder(19).as_utc() + time::Duration::seconds(59)).expect("a time in range");

    assert_eq!(cut_due(reminded_at_noon(), now, 20, false, false, false), CutDue::No);
}

#[test]
fn cut_after_zero_turns_the_cut_off() {
    assert_eq!(
        cut_due(reminded_at_noon(), after_reminder(600), 0, false, false, false),
        CutDue::No
    );
}

#[test]
fn a_moved_head_means_the_reminder_worked() {
    assert_eq!(
        cut_due(reminded_at_noon(), after_reminder(25), 20, true, false, false),
        CutDue::No
    );
}

#[test]
fn a_busy_worktree_is_waited_for_instead_of_cut() {
    assert_eq!(
        cut_due(reminded_at_noon(), after_reminder(25), 20, false, true, false),
        CutDue::Wait
    );
}

#[test]
fn one_reminder_is_cut_once() {
    assert_eq!(
        cut_due(reminded_at_noon(), after_reminder(25), 20, false, false, true),
        CutDue::No
    );
}

#[test]
fn a_clock_before_the_reminder_does_not_cut() {
    let before =
        Timestamp::from_utc(reminded_at_noon().as_utc() - time::Duration::minutes(5)).expect("a time in range");

    assert_eq!(cut_due(reminded_at_noon(), before, 1, false, false, false), CutDue::No);
}
