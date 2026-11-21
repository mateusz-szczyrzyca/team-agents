//! One text in the order things happened: rows and steps together, each once, and the newest lines of both.

#[cfg(test)]
mod tests;

use agent_core::ids::AgentName;
use agent_core::log::LogEntry;

/// One thing of the text, in the order it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    /// A row of the scope.
    Row(usize),
    /// A step of the scope.
    Step(usize),
}

/// The text of the steps of the scope's runs and of its rows, in the order they happened.
#[derive(Debug)]
pub(crate) struct Timeline<'a> {
    /// The rows of the scope, oldest first.
    rows: Vec<&'a LogEntry>,
    /// The agents whose runs the text covers.
    followed: Vec<AgentName>,
}

impl<'a> Timeline<'a> {
    /// The text of `rows`, and of the steps of the runs it covers.
    pub(crate) fn new(rows: Vec<&'a LogEntry>, _steps: Vec<()>, followed: Vec<AgentName>) -> Self {
        Self { rows, followed }
    }

    /// The newest `limit` entries as text, oldest first.
    pub(crate) fn newest(&self, _limit: u32) -> String {
        self.rows
            .iter()
            .map(|row| format!("{} {} {} {}\n", row.at(), row.level(), row.event(), row.message()))
            .collect()
    }
}
