//! What the reminders of the harness did, agent by agent: how many times a run was reminded to commit or told its
//! context is large, how many of the commit reminders were followed and after how long, and how many runs were cut.
//!
//! The table is read from the rows of the log events `commit_reminded`, `context_reminded`, `commit_followed` and
//! `commit_cut` (`agent log --reminders`).

use std::collections::BTreeMap;

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

/// What is counted for one agent and model while the rows are read.
#[derive(Default)]
struct Counts {
    /// Commit reminders.
    commit: u32,
    /// Context reminders.
    context: u32,
    /// The minutes of the followed reminders.
    followed: Vec<u64>,
    /// Cuts.
    cuts: u32,
}

/// The table of `rows` seen at `now`: one line for each agent and model, sorted by agent and then model (a run without
/// a model first), from the rows not older than `days` days. A number of days that reaches before the first year
/// the times can have takes every row.
#[must_use]
pub fn reminder_table(rows: &[ReminderRow], now: Timestamp, days: u32) -> Vec<ReminderLine> {
    let cutoff = now.as_utc().checked_sub(time::Duration::days(i64::from(days)));
    let mut counts: BTreeMap<(AgentName, Option<String>), Counts> = BTreeMap::new();
    for row in rows
        .iter()
        .filter(|row| cutoff.is_none_or(|cutoff| row.at.as_utc() >= cutoff))
    {
        let line = counts.entry((row.agent.clone(), row.model.clone())).or_default();
        match row.kind {
            ReminderKind::Commit => line.commit = line.commit.saturating_add(1),
            ReminderKind::Context => line.context = line.context.saturating_add(1),
            ReminderKind::Followed => line.followed.extend(row.minutes),
            ReminderKind::Cut => line.cuts = line.cuts.saturating_add(1),
        }
    }
    counts
        .into_iter()
        .map(|((agent, model), mut counted)| {
            counted.followed.sort_unstable();
            ReminderLine {
                agent,
                model,
                commit: counted.commit,
                context: counted.context,
                // A count above u32::MAX saturates, as the other counts do.
                followed: u32::try_from(counted.followed.len()).unwrap_or(u32::MAX),
                median_minutes: lower_median(&counted.followed),
                cuts: counted.cuts,
            }
        })
        .collect()
}

/// The median of `sorted`; of an even count, the lower of the two middle values; none for no values.
fn lower_median(sorted: &[u64]) -> Option<u64> {
    let last = sorted.len().checked_sub(1)?;
    sorted.get(last / 2).copied()
}

#[cfg(test)]
mod tests;
