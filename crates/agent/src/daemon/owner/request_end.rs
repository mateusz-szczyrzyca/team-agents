//! The record of the end of a worker's request: who asked, who served, how it ended and what it cost, written once at
//! every end to the daemon's log in the history, under the requester's team and round.

use std::collections::BTreeMap;

use agent_core::history::Cut;
use agent_core::ids::{AgentName, ItemId, RoundId, TaskId, TeamId};
use agent_core::log::{LogEvent, LogRecord, LogTaskScope, LogTeamScope};
use agent_core::state::Timestamp;
use agent_runtime::agentbus::RequestId;
use agent_runtime::worker_request::Settlement;
use serde_json::Value;

/// How a request ended, as the one word of the record's `outcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EndOutcome<'a> {
    /// The result of the worker was settled: its word is the settlement's.
    Settled(&'a Settlement),
    /// An interrupted request kept its workspace and went back to the queue for another run.
    Requeued,
    /// An interrupted request reached its bound of interruptions and was returned to its requester.
    ReturnedUnfinished,
}

impl EndOutcome<'_> {
    /// The word of the record: the settlement's, or the name of the log event that told the same end before.
    pub(super) const fn word(&self) -> &'static str {
        match self {
            Self::Settled(settlement) => settlement.outcome(),
            Self::Requeued => "worker_request_requeued",
            Self::ReturnedUnfinished => "worker_request_returned",
        }
    }
}

/// The facts of one end of a request.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RequestEnd<'a> {
    /// The request.
    pub(super) request: RequestId,
    /// The requester's team.
    pub(super) team: TeamId,
    /// The rounds of the requester's last run; none when it is not known.
    pub(super) rounds: Vec<RoundId>,
    /// The member that asked.
    pub(super) requester: AgentName,
    /// Its task.
    pub(super) requester_task: TaskId,
    /// The step of the requester's plan the request is for; none for a request without one.
    pub(super) step: Option<ItemId>,
    /// The worker that served it.
    pub(super) worker: AgentName,
    /// The worker's task.
    pub(super) worker_task: TaskId,
    /// The number of the try: `1` for the first worker the request went to.
    pub(super) try_number: u32,
    /// How it ended.
    pub(super) outcome: EndOutcome<'a>,
    /// When the worker's last run started; none when it is not known.
    pub(super) started: Option<Timestamp>,
    /// How long the worker's run lasted, in seconds.
    pub(super) seconds: u64,
    /// The cost in US dollars of the worker's last run; none when the CLI did not tell.
    pub(super) cost_usd: Option<f64>,
}

impl RequestEnd<'_> {
    /// The entry of the daemon's log for this end, made at `at`: under the requester's team, rounds and task, with the
    /// fields of the brief. A time, a step or a cost that is not known is the empty text.
    pub(super) fn record(&self, at: Timestamp) -> LogRecord {
        let text = |value: String| Value::String(value);
        let fields = BTreeMap::from([
            ("request".to_owned(), text(self.request.to_string())),
            ("task".to_owned(), text(self.requester_task.to_string())),
            ("requester".to_owned(), text(self.requester.to_string())),
            (
                "step".to_owned(),
                text(self.step.as_ref().map(ToString::to_string).unwrap_or_default()),
            ),
            ("worker".to_owned(), text(self.worker.to_string())),
            ("worker_task".to_owned(), text(self.worker_task.to_string())),
            ("try".to_owned(), Value::from(self.try_number)),
            ("outcome".to_owned(), text(self.outcome.word().to_owned())),
            (
                "started".to_owned(),
                text(self.started.map(|at| at.to_string()).unwrap_or_default()),
            ),
            ("seconds".to_owned(), Value::from(self.seconds)),
            (
                "cost_usd".to_owned(),
                text(self.cost_usd.map(|cost| cost.to_string()).unwrap_or_default()),
            ),
        ]);
        LogRecord {
            at,
            module: module_path!().to_owned(),
            event: LogEvent::WorkerRequestEnded,
            team: Some(LogTeamScope::new(self.team.clone(), self.rounds.iter().cloned())),
            task: Some(LogTaskScope::new(self.requester_task.clone(), None)),
            message: format!("request {} ended: {}", self.request, self.outcome.word()),
            fields,
            cut: Cut::default(),
        }
    }
}

#[cfg(test)]
mod tests;
