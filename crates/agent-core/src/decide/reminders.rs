//! What the reminders of the harness did, agent by agent: how many times a run was reminded to commit or told its
//! context is large, how many of the commit reminders were followed and after how long, and how many runs were cut.
//!
//! The table is read from the rows of the log events `commit_reminded`, `context_reminded`, `commit_followed` and
//! `commit_cut` (`agent log --reminders`).

use crate::ids::AgentName;
use crate::state::Timestamp;

/// Which log event a row of the table was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderKind {
    /// `commit_reminded`: a run was reminded to commit.
    Commit,
    /// `context_reminded`: a session was told that its context is large.
    Context,
    /// `commit_followed`: a run that was reminded to commit committed.
    Followed,
    /// `commit_cut`: a run that ignored the reminder was cut.
    Cut,
}

/// One log event of a reminder, with the model of its run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderRow {
    /// When the event was recorded.
    pub at: Timestamp,
    /// Which event it is.
    pub kind: ReminderKind,
    /// The agent of the run, from the event's field `agent`.
    pub agent: AgentName,
    /// The model of the run, from its run record; none when the record has none or the run is not in the history.
    pub model: Option<String>,
    /// The whole minutes from the reminder to the commit; only a [`ReminderKind::Followed`] row has them.
    pub minutes: Option<u64>,
}

/// The reminders of one agent with one model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReminderLine {
    /// The agent.
    pub agent: AgentName,
    /// The model; none when its runs have none.
    pub model: Option<String>,
    /// How many times a run was reminded to commit.
    pub commit: u32,
    /// How many times a session was told that its context is large.
    pub context: u32,
    /// How many commit reminders were followed by a commit.
    pub followed: u32,
    /// The median of the minutes from a commit reminder to the commit; none when no reminder was followed.
    pub median_minutes: Option<u64>,
    /// How many runs were cut after they ignored the reminder.
    pub cuts: u32,
}

/// The table of `rows` seen at `now`: one line for each agent and model, sorted by agent and then model, from the
/// rows not older than `days` days.
#[must_use]
pub fn reminder_table(_rows: &[ReminderRow], _now: Timestamp, _days: u32) -> Vec<ReminderLine> {
    Vec::new()
}

#[cfg(test)]
mod tests;
