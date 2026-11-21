//! CLI row predicates, separate from the preserved story command.

use std::io::Write;
use std::path::Path;

use agent_core::ids::{AgentName, RoundId, RunId, TaskId, TeamId};
use agent_core::log::{LogEvent, LogLevel, LogView};
use agent_core::state::Timestamp;
use agent_runtime::history::{LogFilter, LogWindow, Reader};
use clap::Args;

use super::steps::{TimedStep, agent_steps, run_steps, task_steps};
use super::timeline::{Team, Timeline};
use super::{Format, LogError};

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
        let scoped = self.task.is_some() || self.run.is_some() || self.agent.is_some();
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
        let runtime = if self.follow {
            Some(
                tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                    .map_err(LogError::FollowRuntime)?,
            )
        } else {
            None
        };
        let agents = followed_agents(&reader, &filter)?;
        let steps = if scoped {
            steps_of(&reader, &filter)?
        } else {
            Vec::new()
        };
        let mut teams: Vec<Team> = Vec::new();
        for team in reader.teams()? {
            let mut runs = std::collections::BTreeMap::new();
            for run in reader.runs_of_team(&team.value.team)? {
                runs.insert(run.start.value.run.clone(), run.start.value);
            }
            teams.push(Team::new(team.value.team.clone(), runs));
        }
        let mut cursor = reader.log_cursor()?;
        let mut window = LogWindow::Newest(self.limit.unwrap_or(DEFAULT_LINES));
        loop {
            let rows = reader.log_rows(&filter, window)?;
            for row in &rows {
                cursor = cursor.max(row.id());
            }
            match format {
                Format::Json => {
                    for row in &rows {
                        print_row(row, format, out)?;
                    }
                }
                Format::Text(_) => {
                    let rows: Vec<&agent_core::log::LogEntry> = rows.iter().collect();
                    let steps = steps
                        .iter()
                        .filter(|step| near(&rows, step.at, step.run.task.as_str()))
                        .cloned()
                        .collect();
                    let mut timeline = Timeline::new(rows, steps, agents.clone());
                    timeline.teams.extend(teams.iter().cloned());
                    let text = timeline.newest(self.limit.unwrap_or(DEFAULT_LINES));
                    out.write_all(text.as_bytes()).map_err(LogError::Output)?;
                }
            }
            out.flush().map_err(LogError::Output)?;
            let Some(runtime) = &runtime else {
                return Ok(());
            };
            // SQLite and stdout stay outside the async runtime. Only the cancellable timer runs inside it.
            runtime.block_on(async { tokio::time::sleep(std::time::Duration::from_millis(100)).await });
            window = LogWindow::After {
                id: cursor,
                limit: 1000,
            };
        }
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

/// Whether `step` is one of the rows' task or comes between the first and the last of them in the day.
fn near(rows: &[&agent_core::log::LogEntry], at: Timestamp, task: &str) -> bool {
    let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
        return false;
    };
    rows.iter()
        .any(|row| row.task().is_some_and(|row| row.as_str() == task))
        || (first.at() <= at && at <= last.at())
}

/// The agents whose runs the scope covers: the one of `--agent`, those of the runs of `--task` and `--run`, and those
/// of the steps. A line names its agent when more than one is followed, or when the scope is one task.
fn followed_agents(reader: &Reader, filter: &LogFilter) -> Result<Vec<AgentName>, LogError> {
    let mut names: Vec<String> = Vec::new();
    if let Some(agent) = &filter.agent {
        names.push(agent.to_string());
    }
    if let Some(task) = &filter.task {
        for run in reader.runs_of_task(task)? {
            names.push(run.start.value.agent.name);
        }
    }
    if let Some(id) = &filter.run
        && let Some(run) = reader.run(id)?
    {
        names.push(run.start.value.agent.name);
    }
    for step in steps_of(reader, filter)? {
        names.push(step.run.agent.name);
    }
    let mut agents: Vec<AgentName> = Vec::new();
    for name in names {
        match name.parse() {
            Ok(agent) => agents.push(agent),
            Err(source) => {
                // The history validated the name of a run when it wrote it, so a name that does not parse cannot
                // happen; it is left out of the followed agents, and the steps of its run print without it.
                tracing::warn!(%source, %name, "a run of the scope has no agent name a log can show");
            }
        }
    }
    agents.sort();
    agents.dedup();
    if agents.len() < 2 && filter.task.is_none() {
        agents.clear();
    }
    Ok(agents)
}

/// The steps of the runs `filter` covers, oldest first.
fn steps_of(reader: &Reader, filter: &LogFilter) -> Result<Vec<TimedStep>, LogError> {
    if let Some(id) = &filter.run {
        return Ok(match reader.run(id)? {
            Some(run) => run_steps(reader, &run.start.value)?,
            None => Vec::new(),
        });
    }
    if let Some(task) = &filter.task {
        return Ok(task_steps(reader, task)?);
    }
    if let Some(agent) = &filter.agent {
        return Ok(agent_steps(reader, agent)?);
    }
    Ok(Vec::new())
}
