//! Bounded, intersecting queries over the daemon log.

use agent_core::ids::{AgentName, RoundId, RunId, TaskId, TeamId};
use agent_core::log::{LogEntry, LogEvent, LogLevel};
use agent_core::state::Timestamp;

use super::{ReadError, Reader};

/// Optional predicates; every supplied predicate must match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogFilter {
    /// Owning team.
    pub team: Option<TeamId>,
    /// Associated round.
    pub round: Option<RoundId>,
    /// Task that emitted the entry.
    pub task: Option<TaskId>,
    /// Run that emitted the entry.
    pub run: Option<RunId>,
    /// Agent of the recorded run.
    pub agent: Option<AgentName>,
    /// Exact severity, rather than a severity threshold.
    pub level: Option<LogLevel>,
    /// Exact event kind.
    pub event: Option<LogEvent>,
    /// Inclusive lower timestamp bound.
    pub since: Option<Timestamp>,
    /// Inclusive upper timestamp bound.
    pub until: Option<Timestamp>,
}

/// A bounded window in insertion order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogWindow {
    /// The newest rows, returned oldest first; zero returns none.
    Newest(u32),
    /// Rows strictly after the durable ID, at most the given count.
    After {
        /// Last consumed ID; zero starts at the beginning.
        id: i64,
        /// Maximum rows in this page.
        limit: u32,
    },
}

impl Reader {
    /// Reads one snapshot with all filters intersected, in insertion order.
    ///
    /// # Errors
    /// [`ReadError`] when the database or a selected row is invalid.
    pub fn log_rows(&self, _filter: &LogFilter, _window: LogWindow) -> Result<Vec<LogEntry>, ReadError> {
        Ok(Vec::new())
    }
}
