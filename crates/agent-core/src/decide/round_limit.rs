//! Whether a round went on too long without its pull request in review: `[limits] round_minutes`.
//!
//! The clock of a round starts at the latest of three moments: the round was opened, the task was handed over to the
//! agent that holds it now, and the first run after a run that ended stopped began (the human stopped the task and
//! resumed it, or the daemon cut the run to hand it over). A cut for an order to commit, which resumes the run in its
//! session, is not a stop for the clock. The daemon asks only for a task that works on an item with no pull request in
//! review yet; a round in review, approved or done is never overdue. Time waiting for human answers is left out.

use crate::state::Timestamp;

/// One run of the task, as the clock of a round reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunMark {
    /// When the run's CLI was started.
    pub started_at: Timestamp,
    /// Whether the run ended stopped and the clock starts again after it: by the human's `agent stop`, or by a cut of
    /// the daemon other than one for an order to commit.
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
        /// The whole working minutes from `since` to now, excluding human waits; at least the limit.
        minutes: u64,
        /// When the clock started, see [`clock_start`].
        since: Timestamp,
    },
}

/// When the clock of the round starts: the latest of its opening, the hand-over to the agent that holds the task now,
/// and the start of the first run after the last run that ended stopped.
#[must_use]
pub fn clock_start(clock: &RoundClock<'_>) -> Timestamp {
    let resumed = clock
        .runs
        .iter()
        .rposition(|run| run.stopped)
        .and_then(|last_stopped| clock.runs.get(last_stopped + 1))
        .map(|run| run.started_at);
    [Some(clock.opened_at), clock.held_since, resumed]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(clock.opened_at)
}

/// Whether the round read by `clock` is over `limit_minutes` at `now`. A limit of `0` turns the rule off; a `now` before
/// the start of the clock is never overdue.
#[must_use]
pub fn round_overdue(clock: &RoundClock<'_>, now: Timestamp, limit_minutes: u32) -> Overdue {
    round_overdue_with_waits(clock, &[], now, limit_minutes)
}

/// One interval the task waits for the human's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HumanWait {
    /// When the questions were put to the human.
    pub asked_at: Timestamp,
    /// When the answer arrived; `None` counts the wait up to now.
    pub answered_at: Option<Timestamp>,
}

/// Whether the round exceeds its limit after leaving out human question waits.
#[must_use]
pub fn round_overdue_with_waits(
    clock: &RoundClock<'_>,
    waits: &[HumanWait],
    now: Timestamp,
    limit_minutes: u32,
) -> Overdue {
    if limit_minutes == 0 {
        return Overdue::NotDue;
    }
    let since = clock_start(clock);
    if now < since {
        return Overdue::NotDue;
    }
    let mut counted = now.as_utc() - since.as_utc();
    let mut intervals: Vec<_> = waits
        .iter()
        .map(|wait| (wait.asked_at.max(since), wait.answered_at.unwrap_or(now).min(now)))
        .collect();
    intervals.sort_unstable();
    // The cursor merges overlaps as they are subtracted, so each instant is left out only once.
    let mut cursor = since;
    for (start, end) in intervals {
        let start = start.max(cursor);
        if end > start {
            counted = counted.saturating_sub(end.as_utc() - start.as_utc());
            cursor = end;
        }
    }
    let minutes = counted.whole_minutes().unsigned_abs();
    if minutes >= u64::from(limit_minutes) {
        Overdue::Due { minutes, since }
    } else {
        Overdue::NotDue
    }
}

#[cfg(test)]
mod tests;
