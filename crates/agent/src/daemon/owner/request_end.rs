//! The record of the end of a worker's request: who asked, who served, how it ended and what it cost, written once at
//! every end to the daemon's log in the history, under the requester's team and round.

use std::collections::BTreeMap;

use std::path::Path;

use agent_core::history::Cut;
use agent_core::ids::{AgentName, RoundId, RunId, TaskId, TeamId};
use agent_core::log::{LogEvent, LogRecord, LogTaskScope, LogTeamScope};
use agent_core::state::Timestamp;
use agent_runtime::agentbus::RequestId;
use agent_runtime::history::{ReadError, Reader};
use agent_runtime::process::error_chain;
use agent_runtime::worker_request::{Pending, Settlement};
use serde_json::Value;

use super::Owner;

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
    pub(super) step: Option<String>,
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

/// The cost in US dollars the history keeps for `run`: none when the run has no usage row, the CLI told no cost or there
/// is no history.
///
/// # Errors
/// The cause, as text, when the history cannot be read.
async fn cost_of_run(history: &Path, run: &RunId) -> Result<Option<f64>, String> {
    if !history.exists() {
        return Ok(None);
    }
    let (path, wanted) = (history.to_path_buf(), run.clone());
    match tokio::task::spawn_blocking(move || Reader::open(&path).and_then(|reader| reader.run_usage(&wanted))).await {
        Err(join) => Err(error_chain(&join)),
        Ok(Err(ReadError::Empty { .. })) => Ok(None),
        Ok(Err(source)) => Err(error_chain(&source)),
        Ok(Ok(usage)) => Ok(usage.and_then(|usage| usage.cost_usd)),
    }
}

/// The request whose end is recorded, as the places that decide an end know it.
#[derive(Debug, Clone, Copy)]
pub(super) struct EndedRequest<'a> {
    /// The request.
    pub(super) request: RequestId,
    /// The requester's team.
    pub(super) team: &'a TeamId,
    /// The member that asked.
    pub(super) requester: &'a AgentName,
    /// Its task.
    pub(super) requester_task: &'a TaskId,
    /// The worker that served it.
    pub(super) worker: &'a AgentName,
    /// The worker's task.
    pub(super) worker_task: &'a TaskId,
    /// How long the worker's run lasted, in seconds.
    pub(super) seconds: u64,
    /// When the worker got the request, where the caller knows it; else the start of the worker's last run is told.
    pub(super) started: Option<Timestamp>,
}

impl<'a> From<&'a Pending> for EndedRequest<'a> {
    fn from(pending: &'a Pending) -> Self {
        Self {
            request: pending.request,
            team: &pending.team,
            requester: &pending.requester,
            requester_task: &pending.requester_task,
            worker: &pending.worker,
            worker_task: &pending.worker_task,
            seconds: pending.run_seconds,
            started: None,
        }
    }
}

impl Owner {
    /// Writes the record of an end of `ended` to the history, once: the caller calls it at the one place where the end is
    /// decided, and never for an end it reads again from a file. A fact that cannot be read leaves its field empty; a
    /// record that cannot be written is told in the log and lost, because the request goes on without it.
    pub(super) async fn log_request_end(&self, ended: EndedRequest<'_>, step: Option<String>, outcome: EndOutcome<'_>) {
        if self.history.is_disabled() {
            return;
        }
        let worker_run = self
            .parts
            .state
            .team(ended.team)
            .and_then(|team| team.find(ended.worker_task))
            .and_then(|(_, member)| member.runs().last().map(|run| (run.id().clone(), run.started_at())));
        let cost_usd = match &worker_run {
            Some((run, _)) => cost_of_run(self.parts.store.home().history_file().as_path(), run)
                .await
                .unwrap_or_else(|cause| {
                    tracing::warn!(
                        event = LogEvent::HistoryRecordIncomplete.as_str(),
                        request = %ended.request,
                        cause,
                        "the cost of the worker's run cannot be read: the record of the end of its request has none"
                    );
                    None
                }),
            None => None,
        };
        let end = RequestEnd {
            request: ended.request,
            team: ended.team.clone(),
            rounds: self
                .requester_round(ended.team, ended.requester_task)
                .into_iter()
                .collect(),
            requester: ended.requester.clone(),
            requester_task: ended.requester_task.clone(),
            step,
            worker: ended.worker.clone(),
            worker_task: ended.worker_task.clone(),
            try_number: self.try_number_of(ended.worker_task, ended.request).number,
            outcome,
            started: ended.started.or(worker_run.map(|(_, started)| started)),
            seconds: ended.seconds,
            cost_usd,
        };
        let at = match Timestamp::now(&self.parts.clock) {
            Ok(at) => at,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::HistoryRecordIncomplete.as_str(),
                    request = %ended.request,
                    error = %error_chain(&error),
                    "the clock gives a time that cannot be recorded: the history has no record of the end of a request"
                );
                return;
            }
        };
        if let Err(error) = self.history.persist_log(end.record(at)).await {
            tracing::warn!(
                event = LogEvent::HistoryRecordIncomplete.as_str(),
                request = %ended.request,
                error = %error_chain(&error),
                "the record of the end of a request cannot be written: the history has none"
            );
        }
    }
}

#[cfg(test)]
mod tests;
