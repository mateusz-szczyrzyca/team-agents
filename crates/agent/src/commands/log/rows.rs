//! CLI row predicates, separate from the preserved story command.

use std::io::Write;
use std::path::Path;

use agent_core::ids::{AgentName, RoundId, RunId, TaskId, TeamId};
use agent_core::log::{LogEvent, LogLevel, LogView};
use agent_core::state::Timestamp;
use agent_runtime::history::{LogFilter, LogWindow, Reader};
use clap::Args;

use super::{Format, LogError};

/// Row arguments; their presence opts a scoped story into log queries.
#[derive(Debug, Args)]
pub(crate) struct RowArgs {
    /// Filter by task ID.
    #[arg(long)]
    task: Option<TaskId>,
    /// Filter by run ID.
    #[arg(long)]
    run: Option<RunId>,
    /// Filter by the run's agent.
    #[arg(long)]
    agent: Option<AgentName>,
    /// Exact severity: error, warn, info, debug.
    #[arg(long)]
    level: Option<LogLevel>,
    /// Exact daemon event name.
    #[arg(long)]
    event: Option<LogEvent>,
    /// Include timestamps at or after this RFC 3339 time.
    #[arg(long)]
    since: Option<Timestamp>,
    /// Include timestamps at or before this RFC 3339 time.
    #[arg(long)]
    until: Option<Timestamp>,
    /// Maximum newest rows (default 100); zero prints none.
    #[arg(short = 'n')]
    limit: Option<u32>,
    /// Print one subject of the selected round or team.
    #[arg(long, requires = "what")]
    show: Option<LogView>,
}

impl RowArgs {
    /// Whether any row option was explicitly supplied.
    pub(crate) fn requested(&self) -> bool {
        self.task.is_some()
            || self.run.is_some()
            || self.agent.is_some()
            || self.level.is_some()
            || self.event.is_some()
            || self.since.is_some()
            || self.until.is_some()
            || self.limit.is_some()
            || self.show.is_some()
    }

    /// Prints a bounded query of the directly opened history.
    pub(crate) fn print(
        &self,
        path: &Path,
        round: Option<RoundId>,
        team: Option<TeamId>,
        format: Format,
        out: &mut impl Write,
    ) -> Result<(), LogError> {
        let reader = Reader::open(path)?;
        let filter = LogFilter {
            grep: None,
            team,
            round,
            task: self.task.clone(),
            run: self.run.clone(),
            agent: self.agent.clone(),
            level: self.level,
            event: self.event,
            since: self.since,
            until: self.until,
        };
        for row in reader.log_rows(&filter, LogWindow::Newest(self.limit.unwrap_or(100)))? {
            let text = match format {
                Format::Text(_) => format!("{} {} {} {}", row.at(), row.level(), row.event(), row.message()),
                Format::Json => serde_json::json!({
                    "id": row.id(), "first_at": row.first_at(), "at": row.at(), "level": row.level(),
                    "event": row.event(), "module": row.module(), "team": row.team(), "rounds": row.rounds(),
                    "task": row.task(), "run": row.run(), "message": row.message(), "fields": row.fields(),
                    "count": row.count().get(), "cut": row.cut().to_string(),
                })
                .to_string(),
            };
            writeln!(out, "{text}").map_err(LogError::Output)?;
        }
        out.flush().map_err(LogError::Output)
    }
}
