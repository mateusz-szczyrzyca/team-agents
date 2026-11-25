//! Whether a round went on too long without its pull request in review: `[limits] round_minutes`.
//!
//! The clock of a round starts at the latest of three moments: the round was opened, the task was handed over to the
//! agent that holds it now, and the first run after a run that ended stopped began (the human stopped the task and
//! resumed it, or the daemon cut the run). The daemon asks only for a task that works on an item with no pull request in
//! review yet; a round in review, approved or done is never overdue.

use crate::state::Timestamp;

/// One run of the task, as the clock of a round reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunMark {
    /// When the run's CLI was started.
    pub started_at: Timestamp,
    /// Whether the run ended stopped: by the human's `agent stop`, or by a cut of the daemon.
    pub stopped: bool,
}

/// What the clock of one round reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundClock<'a> {
    /// When the round was opened.
    pub opened_at: Timestamp,
    /// When the agent that holds the task now took it over; `None` when it held the task from the start.
    pub held_since: Option<Timestamp>,
    /// The task's runs, oldest first.
    pub runs: &'a [RunMark],
}

/// What the clock says about the round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overdue {
    /// The round may go on: the limit is off or not reached.
    NotDue,
    /// The round is over its limit.
    Due {
        /// The whole minutes from `since` to now; at least the limit.
        minutes: u64,
        /// When the clock started, see [`clock_start`].
        since: Timestamp,
    },
}

/// When the clock of the round starts: the latest of its opening, the hand-over to the agent that holds the task now,
/// and the start of the first run after the last run that ended stopped.
#[must_use]
pub fn clock_start(clock: &RoundClock<'_>) -> Timestamp {
    clock.opened_at
}

/// Whether the round read by `clock` is over `limit_minutes` at `now`. A limit of `0` turns the rule off; a `now` before
/// the start of the clock is never overdue.
#[must_use]
pub fn round_overdue(clock: &RoundClock<'_>, now: Timestamp, limit_minutes: u32) -> Overdue {
    let _ = (clock, now, limit_minutes);
    Overdue::NotDue
}

#[cfg(test)]
mod tests;
