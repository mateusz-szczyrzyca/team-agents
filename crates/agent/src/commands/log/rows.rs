//! CLI row predicates, separate from the preserved story command.

use std::io::Write;
use std::path::Path;

use agent_core::ids::{AgentName, RoundId, RunId, TaskId, TeamId};
use agent_core::log::{LogEvent, LogLevel, LogView};
use agent_core::state::Timestamp;
use agent_runtime::history::{LogFilter, LogWindow, Reader};
use clap::Args;

use super::follow::Feed;
use super::wait::wait_line;
use super::{Format, LogError};

/// How long a follow waits before it reads the history again.
const FOLLOW_PAUSE: std::time::Duration = std::time::Duration::from_millis(100);

/// The newest lines `agent log` prints when `-n` names none.
const DEFAULT_LINES: u32 = 50;

/// Row arguments; their presence opts a scoped story into log queries.
#[derive(Debug, Args)]
pub(crate) struct RowArgs {
    /// Follow later rows once, in insertion order, across daemon restarts.
    #[arg(short = 'f', long)]
    follow: bool,

    /// Search recorded text as a literal phrase (FTS5), not an expression.
    #[arg(long)]
    grep: Option<String>,
    /// Database size, table counts and oldest record.
    #[arg(long)]
    stats: bool,
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
    /// Maximum newest lines (default 50); zero prints none.
    #[arg(short = 'n')]
    limit: Option<u32>,
    /// Print one subject of the selected round or team.
    #[arg(long, requires = "what")]
    show: Option<LogView>,
}

impl RowArgs {
    /// Whether any row option was explicitly supplied.
    pub(crate) fn requested(&self) -> bool {
        self.follow
            || self.stats
            || self.grep.is_some()
            || self.task.is_some()
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
    ///
    /// In text, the rows of the scope and the steps of its runs are one text, the newest `limit` things of it.
    pub(crate) fn print(
        &self,
        path: &Path,
        socket: &Path,
        round: Option<RoundId>,
        team: Option<TeamId>,
        format: Format,
        out: &mut impl Write,
    ) -> Result<(), LogError> {
        let reader = Reader::open(path)?;
        if self.stats {
            writeln!(out, "{}", reader.statistics()?).map_err(LogError::Output)?;
            return out.flush().map_err(LogError::Output);
        }
        if let Some(view) = self.show.filter(|view| *view != LogView::Log) {
            return super::views::print(&reader, round, team.as_ref(), view, format, out);
        }
        let filter = LogFilter {
            grep: self.grep.clone(),
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
        let limit = self.limit.unwrap_or(DEFAULT_LINES);
        let runtime = if self.follow {
            Some(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(LogError::FollowRuntime)?,
            )
        } else {
            None
        };
        match format {
            Format::Json => follow_json(&reader, &filter, limit, runtime.as_ref(), out),
            Format::Text(options) => {
                let (mut feed, text) = Feed::open(&reader, filter, options.steps, limit)?;
                out.write_all(text.as_bytes()).map_err(LogError::Output)?;
                out.flush().map_err(LogError::Output)?;
                let Some(runtime) = &runtime else {
                    return Ok(());
                };
                let mut waits = |round: &RoundId| runtime.block_on(wait_line(socket, round));
                loop {
                    // SQLite and stdout stay outside the async runtime. Only the cancellable timer runs inside it.
                    runtime.block_on(async { tokio::time::sleep(FOLLOW_PAUSE).await });
                    let text = feed.poll(&reader, &mut waits)?;
                    out.write_all(text.as_bytes()).map_err(LogError::Output)?;
                    out.flush().map_err(LogError::Output)?;
                }
            }
        }
    }
}

/// The rows as JSON lines: the newest `limit`, then, when `runtime` is given, every row recorded after them.
fn follow_json(
    reader: &Reader,
    filter: &LogFilter,
    limit: u32,
    runtime: Option<&tokio::runtime::Runtime>,
    out: &mut impl Write,
) -> Result<(), LogError> {
    let mut cursor = reader.log_cursor()?;
    let mut window = LogWindow::Newest(limit);
    loop {
        let rows = reader.log_rows(filter, window)?;
        for row in &rows {
            cursor = cursor.max(row.id());
            print_row(row, Format::Json, out)?;
        }
        out.flush().map_err(LogError::Output)?;
        let Some(runtime) = runtime else {
            return Ok(());
        };
        runtime.block_on(async { tokio::time::sleep(FOLLOW_PAUSE).await });
        window = LogWindow::After {
            id: cursor,
            limit: 1000,
        };
    }
}

/// Writes one row in the same format for the initial window and follow pages.
fn print_row(row: &agent_core::log::LogEntry, format: Format, out: &mut impl Write) -> Result<(), LogError> {
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
    writeln!(out, "{text}").map_err(LogError::Output)
}
