//! The requests for workers that `agent status` shows under a task, read from the bus when the tasks are shown.

use std::collections::BTreeMap;

use agent_core::ids::TaskId;
use agent_runtime::agentbus::{AgentBus, TaskRequest};
use agent_runtime::process::error_chain;

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
    let _ = request;
    String::new()
}

/// The line that says why the requests of a task cannot be told.
fn unknown_line(why: &str) -> String {
    format!("    requests unknown ({why})")
}

#[cfg(test)]
mod tests;
