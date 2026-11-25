//! The rows of the reminders for `agent log --reminders`: the daemon's log events about reminding a run to commit or
//! telling a session its context is large, each with the model of its run.

use std::collections::HashMap;
use std::str::FromStr;

use agent_core::decide::reminders::{ReminderKind, ReminderRow};
use agent_core::ids::{AgentName, RunId};
use agent_core::log::{LogEntry, LogEvent};

use super::query::{LogFilter, LogWindow};
use super::{ReadError, Reader, Row_};

/// The log events that make rows of the table, and the kind of row each makes.
const EVENTS: [(LogEvent, ReminderKind); 4] = [
    (LogEvent::CommitReminded, ReminderKind::Commit),
    (LogEvent::ContextReminded, ReminderKind::Context),
    (LogEvent::CommitFollowed, ReminderKind::Followed),
    (LogEvent::CommitCut, ReminderKind::Cut),
];

/// The table the rows are read from, for the errors.
const TABLE: &str = "daemon_logs";

impl Reader {
    /// The rows of the log events `commit_reminded`, `context_reminded`, `commit_followed` and `commit_cut`, oldest
    /// first, each with the agent of its field `agent` and the model of the run it names; a history without a log
    /// has none.
    ///
    /// # Errors
    /// [`ReadError::Query`] when the log or the runs cannot be read; [`ReadError::Value`] when a row of these events
    /// has no valid `agent`, or a `commit_followed` row has no whole `minutes`.
    pub fn reminder_rows(&self) -> Result<Vec<ReminderRow>, ReadError> {
        let mut models: HashMap<RunId, Option<String>> = HashMap::new();
        let mut rows = Vec::new();
        for (event, kind) in EVENTS {
            let filter = LogFilter {
                event: Some(event),
                ..LogFilter::default()
            };
            let window = LogWindow::After { id: 0, limit: u32::MAX };
            for entry in self.log_rows(&filter, window)? {
                let model = self.cached_model_of(entry.run(), &mut models)?;
                rows.push(row_of(&entry, kind, model)?);
            }
        }
        rows.sort_by_key(|row| row.at);
        Ok(rows)
    }

    /// The model of `run`, read once for each run: `models` keeps what was read. None for an entry of no run.
    fn cached_model_of(
        &self,
        run: Option<&RunId>,
        models: &mut HashMap<RunId, Option<String>>,
    ) -> Result<Option<String>, ReadError> {
        let Some(run) = run else {
            return Ok(None);
        };
        if let Some(model) = models.get(run) {
            return Ok(model.clone());
        }
        let model = self.model_of(run)?;
        models.insert(run.clone(), model.clone());
        Ok(model)
    }

    /// The model the run `run` was started with; none when its record has none or the history has no record of it.
    fn model_of(&self, run: &RunId) -> Result<Option<String>, ReadError> {
        Ok(self
            .run(run)?
            .and_then(|row| row.start.value.agent.model)
            .map(|model| model.as_str().to_owned()))
    }
}

/// The row of `entry`, an event of `kind` of a run of `model`.
fn row_of(entry: &LogEntry, kind: ReminderKind, model: Option<String>) -> Result<ReminderRow, ReadError> {
    let here = Row_::of(TABLE, entry.id().to_string());
    let agent = entry
        .fields()
        .get("agent")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| here.rule("fields", "a reminder event names its agent"))?;
    let agent = here.value("fields", AgentName::from_str(agent))?;
    let minutes = match kind {
        ReminderKind::Followed => Some(
            entry
                .fields()
                .get("minutes")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| here.rule("fields", "a followed reminder has its whole minutes"))?,
        ),
        ReminderKind::Commit | ReminderKind::Context | ReminderKind::Cut => None,
    };
    Ok(ReminderRow {
        at: entry.at(),
        kind,
        agent,
        model,
        minutes,
    })
}

#[cfg(test)]
mod tests;
