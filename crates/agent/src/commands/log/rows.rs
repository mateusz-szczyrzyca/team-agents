//! CLI row predicates, separate from the preserved story command.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;

use agent_core::ids::{AgentName, RoundId, RunId, TaskId, TeamId};
use agent_core::log::{LogEvent, LogLevel, LogView};
use agent_core::state::Timestamp;
use agent_runtime::history::{LogFilter, LogWindow, Reader};
use clap::Args;

use super::steps::{TimedStep, agent_steps, run_steps, task_steps};
use super::team::{Team, teams};
use super::timeline::Timeline;
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
    pub(crate) fn print(
        &self,
        path: &Path,
        round: Option<RoundId>,
        team: Option<TeamId>,
        format: Format,
        steps: bool,
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
        let agents = Vec::new();
        let steps: Vec<TimedStep> = Vec::new();
        let known: Vec<Team> = Vec::new();
        let mut cursor = reader.log_cursor()?;
        let mut window = LogWindow::Newest(self.limit.unwrap_or(100));
        let mut first = true;
        let mut printed: BTreeSet<i64> = BTreeSet::new();
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
                    print_text(
                        &rows,
                        &steps,
                        &known,
                        &agents,
                        self.limit.unwrap_or(DEFAULT_LINES),
                        &mut printed,
                        first,
                        self.follow,
                        out,
                    )?;
                }
            }
            first = false;
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

/// Writes one row as JSON.
fn print_row(row: &agent_core::log::LogEntry, format: Format, out: &mut impl Write) -> Result<(), LogError> {
    let text = match format {
        Format::Text(_) => String::new(),
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

/// Writes the text of `page`'s rows and of their steps: the newest `limit` things of it, or, when following, the
/// things that were not printed yet.
fn print_text(
    page: &[agent_core::log::LogEntry],
    steps: &[TimedStep],
    teams: &[Team],
    followed: &[AgentName],
    limit: u32,
    printed: &mut BTreeSet<i64>,
    first: bool,
    following: bool,
    out: &mut impl Write,
) -> Result<(), LogError> {
    let rows: Vec<&agent_core::log::LogEntry> = page
        .iter()
        .filter(|row| first || !following || !printed.contains(&row.id()))
        .collect();
    let scoped = rows
        .iter()
        .any(|row| row.task().is_some() || row.run().is_some() || row.team().is_some());
    let nearby = steps
        .iter()
        .filter(|step| !scoped || near(&rows, step.at, step.run.task.as_str()))
        .collect();
    let mut timeline = Timeline::new(rows, nearby, followed.to_vec());
    timeline.teams.extend(
        teams
            .iter()
            .filter(|team| page.iter().any(|row| row.team() == Some(&team.team)))
            .cloned(),
    );
    let text = if following && !first {
        timeline.fresh()
    } else {
        timeline.newest(limit)
    };
    for row in page {
        printed.insert(row.id());
    }
    out.write_all(text.as_bytes()).map_err(LogError::Output)
}

/// Whether `step` is one of the rows' task or comes between the first and the last of them in the day.
fn near(rows: &[&agent_core::log::LogEntry], at: agent_core::state::Timestamp, task: &str) -> bool {
    let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
        return false;
    };
    rows.iter()
        .any(|row| row.task().is_some_and(|row| row.as_str() == task))
        || (first.at() <= at && at <= last.at())
}

/// The agents whose runs are followed: the one of `--agent`, or those of the runs of `--task` and `--run`, and those of
/// the rows themselves.
fn followed_agents(reader: &Reader, filter: &LogFilter) -> Result<Vec<AgentName>, LogError> {
    let mut agents: Vec<AgentName> = Vec::new();
    if let Some(agent) = &filter.agent {
        agents.push(agent.clone());
    }
    if let Some(task) = &filter.task {
        agents.extend(step_agents(task_steps(reader, task)?));
    }
    if let Some(id) = &filter.run {
        if let Some(run) = reader.run(id)? {
            agents.extend(name(&run.start.value.agent.name));
        }
    }
    agents.extend(step_agents(steps_of(reader, filter)?));
    agents.sort();
    agents.dedup();
    // A log names an agent only when more than one run is followed; one name alone says nothing of the text. A scope
    // of one task keeps it: its lines name the task and the agent that runs it.
    if agents.len() < 2 && filter.task.is_none() {
        agents.clear();
    }
    Ok(agents)
}

/// The agent of `name`, which a run records as text and the log prints as its name.
fn name(name: &str) -> Option<AgentName> {
    match name.parse() {
        Ok(agent) => Some(agent),
        Err(source) => {
            // The history validated the name of a run when it wrote it, so this cannot happen; a name that does not
            // parse is left out of the followed agents, and its steps print without it.
            tracing::warn!(%source, name, "a run of the scope has no agent name a log can show");
            None
        }
    }
}

/// The agents of `steps`, each as often as its steps are.
fn step_agents(steps: Vec<TimedStep>) -> Vec<AgentName> {
    let mut agents: Vec<AgentName> = Vec::new();
    for step in steps {
        agents.extend(name(&step.run.agent.name));
    }
    agents
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
