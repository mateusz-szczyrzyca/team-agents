//! What `agent status --running` says of the workers that served a team: for each request of a member of the team that
//! a worker took, the worker's task, the worker, the step, what the requester made of the result, and when the worker's
//! runs started and ended — in the columns of the team's tasks.

use agent_core::ids::{RunId, TaskId};
use agent_core::kinds::Role;
use agent_core::local_time::Zone;
use agent_core::protocol::TeamSummary;
use agent_core::state::Timestamp;
use agent_runtime::agentbus::{DelegationStatus, HelperRecord, HelperVerdict};
use agent_runtime::run::outcome::OUTCOME_FILE;
use agent_runtime::store::Home;

use super::members::Columns;

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
pub(crate) fn of_team<'a>(team: &TeamSummary, records: &'a [HelperRecord]) -> Vec<&'a HelperRecord> {
    let mut served: Vec<&HelperRecord> = records
        .iter()
        .filter(|record| record.entry.worker.is_some())
        .filter(|record| team.members.iter().any(|member| member.task == record.entry.from_task))
        .collect();
    served.sort_by_key(|record| record.entry.id);
    served
}

/// The width of a task's ID, which the line of a request not yet given a task pads its placeholder to.
const TASK_WIDTH: usize = "task-20261125-075745-w8vj".len();

/// What the line of a worker under its team says, before [`line`] pads it to the columns of the team's tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Served {
    /// The worker's task, where its runs and its log are; `no task yet` before the request was given one.
    task: String,
    /// The worker and the step it took, in the column of the agents: `glm-1  step 6.1`.
    agent: String,
    /// What became of the result, then when the worker's runs went on: `accepted, 2026-11-25 08:57–08:59 +01:00`.
    state: String,
}

impl Served {
    /// The width the column of the agents needs for this worker and its step.
    pub(crate) fn agent_width(&self) -> usize {
        self.agent.chars().count()
    }
}

/// What `agent status --running` says of `record` under its team, with the times of `span` in `zone`: the worker's task,
/// the worker and its step, what became of the result and when the worker's runs went on.
pub(crate) fn served(record: &HelperRecord, span: Span, zone: Zone) -> Served {
    let entry = &record.entry;
    let worker = entry.worker.as_ref().map_or("?", |worker| worker.as_str());
    let step = entry
        .step
        .as_deref()
        .filter(|step| !step.is_empty())
        .map_or_else(String::new, |step| format!("  step {step}"));
    let task = record
        .helper_task
        .as_ref()
        .map_or_else(|| "no task yet".to_owned(), ToString::to_string);
    let working = matches!(entry.status, DelegationStatus::Starting | DelegationStatus::Running);
    Served {
        task,
        agent: format!("{worker}{step}"),
        state: format!(
            "{}, {}",
            result(entry.status, record.verdict),
            times(span, working, zone)
        ),
    }
}

/// The line of `served` padded to `columns`, as the lines of the team's tasks are: the task, the role `worker`, the
/// worker and its step, what became of the result and when it worked.
pub(super) fn line(served: &Served, columns: Columns) -> String {
    let (role_width, agent_width) = (columns.role(), columns.agent());
    format!(
        "  {:<TASK_WIDTH$}  {:<role_width$}  {:<agent_width$}  {}",
        served.task,
        Role::Worker.to_string(),
        served.agent,
        served.state
    )
}

/// When the work of `span` went on, in `zone`: one date when it started and ended on the same day; `since` for work
/// that goes on.
fn times(span: Span, working: bool, zone: Zone) -> String {
    let Some(start) = span.start else {
        return "not started".to_owned();
    };
    let started = zone.stamp_minute(start);
    match span.end {
        None if working => format!("since {started}"),
        None => format!("{started}, end not recorded"),
        Some(end) => {
            let ended = zone.stamp_minute(end);
            match (split_stamp(&started), split_stamp(&ended)) {
                (Some((day, from, label)), Some((end_day, to, end_label))) if day == end_day && label == end_label => {
                    format!("{day} {from}–{to} {label}")
                }
                _ => format!("{started} – {ended}"),
            }
        }
    }
}

/// The date, the clock and the zone of a stamp `2026-11-25 08:57 +01:00`; `None` for another form, such as the RFC
/// 3339 text the UTC zone writes.
fn split_stamp(stamp: &str) -> Option<(&str, &str, &str)> {
    let mut parts = stamp.splitn(3, ' ');
    Some((parts.next()?, parts.next()?, parts.next()?))
}

/// What became of a request in `status` whose requester gave `verdict`.
const fn result(status: DelegationStatus, verdict: Option<HelperVerdict>) -> &'static str {
    match (status, verdict) {
        (DelegationStatus::Queued, _) => "queued",
        (DelegationStatus::Starting | DelegationStatus::Running, _) => "working",
        (DelegationStatus::Done, Some(HelperVerdict::Used)) => "accepted",
        (DelegationStatus::Done, Some(HelperVerdict::Corrected)) => "accepted after correction",
        (DelegationStatus::Done, Some(HelperVerdict::Discarded)) => "rejected",
        (DelegationStatus::Done, None) => "not rated yet",
        (DelegationStatus::Failed, _) => "failed",
        (DelegationStatus::Unclear, _) => "unclear, asked back",
    }
}

/// When the runs of the worker's `task` in `home` started and ended: the start is the time in the ID of its first run,
/// the end the time its last run wrote its outcome. A task without runs, or whose directory cannot be read, has neither.
pub(crate) async fn span_of(home: &Home, task: &TaskId) -> Span {
    let runs_dir = home.runs_dir(task);
    // The view tells what it can: a directory of runs that cannot be read leaves the times out, as one that is not
    // there does, and the status goes on.
    let Ok(mut entries) = tokio::fs::read_dir(&runs_dir).await else {
        return Span::default();
    };
    let mut runs: Vec<RunId> = Vec::new();
    // An entry that cannot be read ends the list, as a directory that cannot be read gives none.
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Some(run) = entry.file_name().to_str().and_then(|name| name.parse::<RunId>().ok()) {
            runs.push(run);
        }
    }
    runs.sort();
    let start = runs
        .first()
        .and_then(RunId::generated_at)
        .and_then(|at| Timestamp::from_utc(at).ok());
    let end = match runs.last() {
        Some(last) => ended_at(&runs_dir.join(last.as_str()).join(OUTCOME_FILE)).await,
        None => None,
    };
    Span { start, end }
}

/// When the outcome file at `path` was written; `None` when it is not there (the run goes on, or was cut off before
/// it wrote it) or its time cannot be read.
async fn ended_at(path: &std::path::Path) -> Option<Timestamp> {
    // Not there and not readable both leave the end out: the line then says that it was not recorded.
    let modified = tokio::fs::metadata(path).await.ok()?.modified().ok()?;
    Timestamp::from_utc(time::UtcDateTime::from(modified)).ok()
}
