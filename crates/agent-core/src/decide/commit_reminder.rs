//! Whether a run that holds uncommitted work is reminded to commit it: both bounds of `[commit_reminder]` reached, and the
//! run not reminded at this head already.

use crate::kinds::Role;
use crate::state::{CommitId, Timestamp};

/// What the daemon saw of a run's worktree at one look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitLook {
    /// The lines the worktree differs from its head by: added plus removed, over every changed file.
    pub uncommitted: u64,
    /// When the head was committed.
    pub last_commit: Timestamp,
    /// When the CLI of the run was started.
    pub run_started: Timestamp,
    /// The time of the look.
    pub now: Timestamp,
    /// The commit the worktree is at.
    pub head: CommitId,
    /// The head the run was reminded at, if it was.
    pub reminded_at: Option<CommitId>,
}

/// What the look says about reminding the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderDue {
    /// The run is not reminded now.
    NotDue,
    /// The run is reminded: it holds `lines` uncommitted lines and has made no commit for `minutes` whole minutes.
    Due {
        /// The uncommitted lines the look found.
        lines: u64,
        /// The whole minutes since the later of the last commit and the start of the run.
        minutes: u64,
    },
}

/// Whether a run of a task with `role` is reminded to commit at all: a solo agent, a coordinator and a member are; a
/// worker is not, because the harness checks its result already, and a reviewer is not, because it commits nothing.
#[must_use]
pub const fn is_reminded(role: Role) -> bool {
    match role {
        Role::Solo | Role::Coordinator | Role::Member => true,
        Role::Worker | Role::Reviewer => false,
    }
}

/// Whether the run seen in `look` is reminded to commit, under the bounds `lines` and `minutes` of `[commit_reminder]`.
///
/// Not when either bound is `0` (the reminder is off) or the run was reminded at this head already: one reminder holds
/// until a commit moves the head. Otherwise the run is reminded when it holds at least `lines` uncommitted lines and
/// the whole minutes from the later of its last commit and its start to now are at least `minutes`. A commit made before
/// the run started does not count as a recent one: only the time the run itself has worked does.
#[must_use]
pub fn reminder_due(look: &CommitLook, lines: u32, minutes: u32) -> ReminderDue {
    if lines == 0 || minutes == 0 || look.reminded_at.as_ref() == Some(&look.head) {
        return ReminderDue::NotDue;
    }
    let idle = idle_minutes(look);
    if look.uncommitted >= u64::from(lines) && idle >= u64::from(minutes) {
        ReminderDue::Due {
            lines: look.uncommitted,
            minutes: idle,
        }
    } else {
        ReminderDue::NotDue
    }
}

/// The whole minutes from the later of the last commit and the start of the run to the time of the look; none when the
/// look is before that time (a commit dated in the future).
fn idle_minutes(look: &CommitLook) -> u64 {
    let since = look.last_commit.max(look.run_started);
    let minutes = (look.now.as_utc() - since.as_utc()).whole_minutes();
    if minutes <= 0 { 0 } else { minutes.unsigned_abs() }
}

/// What the time since a reminder says about cutting the run that was reminded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutDue {
    /// The run is not cut, and nothing is waited for.
    No,
    /// The run would be cut, but a gate or a test run of its worktree is going: the look is made again later.
    Wait,
    /// The run is cut: the reminder is `minutes` whole minutes old and was ignored.
    Cut {
        /// The whole minutes since the reminder.
        minutes: u64,
    },
}

/// Whether a run reminded at `reminded` is cut at `now`, when `[commit_reminder] cut_after` is `cut_after`.
///
/// Not when `cut_after` is `0` (the cut is off), when the head moved since the reminder (the run committed: the reminder
/// worked) or when the run was cut for this reminder already: one cut per reminder. Otherwise a run whose worktree is
/// `busy` with a gate or a test run is waited for, and any other run is cut when at least `cut_after` whole minutes passed
/// since the reminder.
#[must_use]
pub fn cut_due(
    reminded: Timestamp,
    now: Timestamp,
    cut_after: u32,
    head_moved: bool,
    busy: bool,
    cut_already: bool,
) -> CutDue {
    if cut_after == 0 || head_moved || cut_already {
        return CutDue::No;
    }
    if busy {
        return CutDue::Wait;
    }
    let minutes = minutes_between(reminded, now);
    if minutes >= u64::from(cut_after) {
        CutDue::Cut { minutes }
    } else {
        CutDue::No
    }
}

/// The whole minutes from `earlier` to `later`; none when `later` is not after it.
fn minutes_between(earlier: Timestamp, later: Timestamp) -> u64 {
    let minutes = (later.as_utc() - earlier.as_utc()).whole_minutes();
    if minutes <= 0 { 0 } else { minutes.unsigned_abs() }
}

#[cfg(test)]
mod tests;
