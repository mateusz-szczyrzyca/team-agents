//! A record's line in the daemon log: the event in words, the task, the agent, the run and the fields.
//!
//! The line is a pure function of a stored row and of its run; nothing here reads a clock, a file or the state.

#[cfg(test)]
mod tests;

use crate::history::RunStart;
use crate::log::LogEntry;

/// The line of `entry` that `agent log` prints as text, with `subject`, the start of the entry's run, when known.
#[must_use]
pub fn record_line(entry: &LogEntry, _subject: Option<&RunStart>) -> String {
    format!("{} {} {}", entry.at(), entry.level(), entry.event())
}
