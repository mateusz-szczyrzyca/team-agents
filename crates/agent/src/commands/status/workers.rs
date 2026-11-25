//! What `agent status --running` says of the workers that served a team: for each request of a member of the team that
//! a worker took, the worker, the step, when its runs started and ended, and what the requester made of the result.

use agent_core::ids::TaskId;
use agent_core::local_time::Zone;
use agent_core::protocol::TeamSummary;
use agent_core::state::Timestamp;
use agent_runtime::agentbus::HelperRecord;
use agent_runtime::store::Home;

#[cfg(test)]
mod tests;

/// When the runs of a worker's task started and ended, as its directory of runs tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Span {
    /// When its first run started; `None` before it ran.
    pub(crate) start: Option<Timestamp>,
    /// When its last run ended; `None` while that run goes on, or when its end was not written.
    pub(crate) end: Option<Timestamp>,
}

/// The requests of `records` that a worker took for a member of `team`, by their numbers.
pub(crate) fn of_team<'a>(_team: &TeamSummary, _records: &'a [HelperRecord]) -> Vec<&'a HelperRecord> {
    Vec::new()
}

/// The line `agent status --running` prints of `record` under its team, with the times of `span` in `zone`.
pub(crate) fn line(_record: &HelperRecord, _span: Span, _zone: Zone) -> String {
    String::new()
}

/// When the runs of the worker's `task` in `home` started and ended: the start is the time in the ID of its first run,
/// the end the time its last run wrote its outcome. A task without runs, or whose directory cannot be read, has neither.
pub(crate) async fn span_of(_home: &Home, _task: &TaskId) -> Span {
    Span::default()
}
