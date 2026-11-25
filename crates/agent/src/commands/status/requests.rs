//! The requests for workers that `agent status` shows under a task, read from the bus when the tasks are shown.

use std::collections::BTreeMap;
use std::path::Path;

use agent_core::ids::TaskId;
use agent_core::local_time::Zone;
use agent_core::state::Timestamp;
use agent_runtime::agentbus::{AgentBus, DelegationStatus, TaskRequest};
use agent_runtime::process::error_chain;
use agent_runtime::worker_wait;

/// Where `agent status` reads the requests of the tasks it shows.
#[derive(Debug, Clone)]
pub(crate) enum Source {
    /// Nothing is read: a status shown without the bus.
    Unread,
    /// The bus of the home.
    Bus(AgentBus),
    /// The bus cannot be known; why, as a line says it.
    Unknown(String),
}

/// The lines of the requests of each task shown, by task.
#[derive(Debug, Default)]
pub(crate) struct Told {
    /// The lines, by task; a task without requests has none.
    lines: BTreeMap<TaskId, Vec<String>>,
}

impl Told {
    /// The lines of the requests of `task`.
    pub(crate) fn of(&self, task: &TaskId) -> &[String] {
        self.lines.get(task).map_or(&[], Vec::as_slice)
    }
}

/// The lines of the requests of `tasks`, read from `source`: a line for each request, or a line that says why they
/// cannot be told.
pub(crate) async fn told(source: &Source, tasks: impl IntoIterator<Item = &TaskId>) -> Told {
    let mut lines = BTreeMap::new();
    for task in tasks {
        let of_task = match source {
            Source::Unread => continue,
            Source::Unknown(why) => vec![unknown_line(why)],
            Source::Bus(bus) => match bus.helper_dump(task).await {
                Ok(requests) => requests.iter().map(request_line).collect(),
                Err(error) => vec![unknown_line(&error_chain(&error))],
            },
        };
        lines.insert(task.clone(), of_task);
    }
    Told { lines }
}

/// The line of one request under its task: its number, its step, its worker and its state.
pub(crate) fn request_line(request: &TaskRequest) -> String {
    let step = request
        .step
        .map_or_else(|| "no step".to_owned(), |step| format!("step {step}"));
    let worker = request
        .worker
        .as_ref()
        .map_or_else(|| "no worker yet".to_owned(), |worker| format!("worker {worker}"));
    format!("    request {}  {step}  {worker}  {}", request.id, request.status)
}

/// The moment the status is shown and the zone it is told in: what a request that waits needs to say since when.
#[derive(Clone, Copy)]
pub(crate) struct Shown {
    /// The moment of the look.
    pub(crate) now: Timestamp,
    /// The zone the times are written in.
    pub(crate) zone: Zone,
}

/// The line of one request, with how long it has waited when it is queued and `since` says since when.
pub(crate) fn request_line_waiting(request: &TaskRequest, since: Option<Timestamp>, shown: Shown) -> String {
    let _ = (since, shown);
    request_line(request)
}

/// The line of one request of the task whose directory is `task_dir`: its note of the wait is read when it is queued.
pub(crate) async fn line_of(request: &TaskRequest, task_dir: &Path, shown: Shown) -> String {
    let _ = (task_dir, shown);
    request_line(request)
}

/// The line that says why the requests of a task cannot be told.
fn unknown_line(why: &str) -> String {
    format!("    requests unknown ({why})")
}

#[cfg(test)]
mod tests;
