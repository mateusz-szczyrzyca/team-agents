//! Bounded, intersecting queries over the daemon log.

use agent_core::ids::{AgentName, RoundId, RunId, TaskId, TeamId};
use agent_core::log::{LogEntry, LogEvent, LogLevel};
use agent_core::state::Timestamp;

use rusqlite::{params_from_iter, types::Value};

use super::{ReadError, Reader, decode_daemon_log, query};
use crate::history::migrate::version_of;
use crate::history::schema::DAEMON_LOGS_VERSION;

/// UTC text padded to nanoseconds, so text order is timestamp order.
const TIME_KEY: &str = "substr(l.at,1,19)||'.'||substr(CASE WHEN substr(l.at,20,1)='.' THEN substr(l.at,21,length(l.at)-21) ELSE '' END||'000000000',1,9)";

/// Normalizes the variable fractional precision of the persisted timestamp.
fn time_key(at: Timestamp) -> String {
    let text = at.to_string();
    let utc = text.trim_end_matches('Z');
    let (seconds, fraction) = utc.split_once('.').unwrap_or((utc, ""));
    format!("{seconds}.{fraction:0<9}")
}

/// SQL and its bound values; only fixed fragments enter the SQL text.
#[derive(Debug)]
struct Selection {
    /// Predicates combined with AND.
    predicates: Vec<String>,
    /// Values in placeholder order.
    values: Vec<Value>,
}

impl Selection {
    fn add(&mut self, predicate: &str, value: impl Into<Value>) {
        self.predicates.push(predicate.to_owned());
        self.values.push(value.into());
    }

    fn new(filter: &LogFilter) -> Self {
        let mut selection = Self {
            predicates: Vec::new(),
            values: Vec::new(),
        };
        for (predicate, value) in [
            ("l.team = ?", filter.team.as_ref().map(ToString::to_string)),
            (
                "l.id IN (SELECT log FROM daemon_log_rounds WHERE round = ?)",
                filter.round.as_ref().map(ToString::to_string),
            ),
            ("l.task = ?", filter.task.as_ref().map(ToString::to_string)),
            ("l.run = ?", filter.run.as_ref().map(ToString::to_string)),
            (
                "l.run IN (SELECT run FROM runs WHERE agent = ?)",
                filter.agent.as_ref().map(ToString::to_string),
            ),
            ("l.level = ?", filter.level.map(|v| v.to_string())),
            ("l.event = ?", filter.event.map(|v| v.to_string())),
        ] {
            if let Some(value) = value {
                selection.add(predicate, value);
            }
        }
        if let Some(at) = filter.since {
            selection.add(&format!("({TIME_KEY}) >= ?"), time_key(at));
        }
        if let Some(at) = filter.until {
            selection.add(&format!("({TIME_KEY}) <= ?"), time_key(at));
        }
        selection
    }
}

/// Optional predicates; every supplied predicate must match.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogFilter {
    /// Literal phrase searched across recorded text.
    pub grep: Option<String>,
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
    pub fn log_rows(&self, filter: &LogFilter, window: LogWindow) -> Result<Vec<LogEntry>, ReadError> {
        let snapshot = self.snapshot("daemon_logs")?;
        if version_of(&snapshot).map_err(query("daemon_logs"))? < DAEMON_LOGS_VERSION {
            return Ok(Vec::new());
        }
        if filter.grep.is_some() && version_of(&snapshot).map_err(query("daemon_logs"))? < 5 {
            return Ok(Vec::new());
        }
        let mut selection = Selection::new(filter);
        let (order, limit) = match window {
            LogWindow::Newest(limit) => ("DESC", limit),
            LogWindow::After { id, limit } => {
                selection.add("l.id > ?", id);
                ("ASC", limit)
            }
        };
        if selection.predicates.is_empty() {
            selection.predicates.push("l.id > 0".to_owned());
        }
        let prefix = if let Some(text) = &filter.grep {
            selection
                .values
                .insert(0, format!("\"{}\"", text.replace('"', "\"\"")).into());
            selection.predicates.push("(l.id IN (SELECT log FROM hits) OR l.run IN (SELECT run FROM hits) OR l.task IN (SELECT task FROM hits WHERE run IS NULL) OR l.id IN (SELECT d.log FROM daemon_log_rounds d JOIN history_record_rounds h ON h.round=d.round WHERE h.sequence IN (SELECT sequence FROM hits WHERE run IS NULL AND task IS NULL)))".to_owned());
            "WITH hits AS MATERIALIZED (SELECT sequence,log,run,task FROM history_search WHERE history_search MATCH ?) "
        } else {
            ""
        };
        let sql = format!(
            "{prefix}SELECT l.id,l.first_at,l.at,l.level,l.module,l.event,l.team,l.task,l.run,l.message,l.fields,l.count,l.cut \
             FROM daemon_logs l WHERE {} ORDER BY l.id {order} LIMIT ?",
            selection.predicates.join(" AND ")
        );
        selection.values.push(i64::from(limit).into());
        let mut statement = snapshot.prepare(&sql).map_err(query("daemon_logs"))?;
        let mut rows = statement
            .query(params_from_iter(selection.values))
            .map_err(query("daemon_logs"))?;
        let mut entries = Vec::new();
        while let Some(row) = rows.next().map_err(query("daemon_logs"))? {
            entries.push(decode_daemon_log(&snapshot, row)?);
        }
        if matches!(window, LogWindow::Newest(_)) {
            entries.reverse();
        }
        Ok(entries)
    }
}
