//! Whether a run that holds uncommitted work is reminded to commit it: both bounds of `[commit_reminder]` reached, and the
//! run not reminded at this head already.

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

/// Whether the run seen in `look` is reminded to commit, under the bounds `lines` and `minutes` of `[commit_reminder]`.
///
/// Not when either bound is `0` (the reminder is off) or the run was reminded at this head already: one reminder holds
/// until a commit moves the head. Otherwise the run is reminded when it holds at least `lines` uncommitted lines and
/// the whole minutes from the later of its last commit and its start to now are at least `minutes`. A commit made before
/// the run started does not count as a recent one: only the time the run itself has worked does.
#[must_use]
pub fn reminder_due(look: &CommitLook, lines: u32, minutes: u32) -> ReminderDue {
    ReminderDue::NotDue
}

#[cfg(test)]
mod tests;
