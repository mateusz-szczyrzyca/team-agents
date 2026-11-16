//! Whether a team sleeps with its work unfinished: every task waits and none of them will be woken by another.
//!
//! A coordinator that gives work to a member and sleeps, and a member that wakes, does nothing and sleeps again, leave a
//! team that looks like one at work and never moves. [`asleep_step`] tells that apart from the waits that end by
//! themselves:
//!
//! 1. No item of the team is in progress or being fixed: there is nothing to wake anybody for.
//! 2. A task of the crew is not waiting — it is queued, runs, asks the human or is paused: somebody works or will.
//! 3. The lead's last run ended with `STATUS: DONE`: its branch is with the harness (the gate, the pull request), which
//!    says so itself when it cannot go on.
//! 4. The last run of the crew ended less than the time a team may sleep ago: a message may still wake somebody.
//! 5. Otherwise the lead is woken and told that nobody works; when it was woken for it as often as it is, the human is
//!    told instead.

use std::time::Duration;

use crate::status::TaskStatus;

/// What is known of a team at one look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Team<'a> {
    /// Whether an item of the team is in progress or being fixed.
    pub unfinished: bool,
    /// The status of every task of the crew, the lead first: the lead, the members and the workers.
    pub tasks: &'a [TaskStatus],
    /// Whether the lead's last run ended with `STATUS: DONE`.
    pub lead_done: bool,
    /// How long ago the last run of the crew ended.
    pub quiet_for: Duration,
    /// How long every task of a team may sleep before the lead is woken.
    pub may_sleep: Duration,
    /// How many times in a row the lead was woken because the team slept.
    pub woken: u32,
    /// How many times in a row the lead is woken before the human is told.
    pub wake_limit: u32,
}

/// What the harness does about a team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsleepStep {
    /// Nothing: the team works, waits for something that ends by itself, or has nothing unfinished.
    Nothing,
    /// The lead is woken and told that nobody works.
    WakeLead,
    /// The human is told: waking the lead did not help.
    TellHuman,
}

/// What the harness does about `team`.
#[must_use]
pub fn asleep_step(team: &Team<'_>) -> AsleepStep {
    let busy = |status: &TaskStatus| match status {
        TaskStatus::Queued | TaskStatus::Running | TaskStatus::Questions | TaskStatus::Paused => true,
        TaskStatus::Waiting | TaskStatus::Done | TaskStatus::Failed | TaskStatus::Stopped => false,
    };
    // A lead that is over cannot be woken; one that does not wait works, asks or will.
    let lead_sleeps = team.tasks.first() == Some(&TaskStatus::Waiting);
    if !team.unfinished
        || !lead_sleeps
        || team.tasks.iter().any(busy)
        || team.lead_done
        || team.quiet_for < team.may_sleep
    {
        return AsleepStep::Nothing;
    }
    if team.woken >= team.wake_limit {
        AsleepStep::TellHuman
    } else {
        AsleepStep::WakeLead
    }
}

#[cfg(test)]
mod tests;
