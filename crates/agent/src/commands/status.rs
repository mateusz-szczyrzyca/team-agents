//! `agent status`: the teams the daemon knows, their rounds, and what each of their tasks is doing — or, for a round, its
//! item, the tasks that worked for it, its pull request and its review passes.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::SystemTime;

use std::num::NonZeroUsize;

use agent_core::cli::RunUsage;
use agent_core::history::HistoryHealth;
use agent_core::ids::{AgentName, RoundId, TaskId, TeamId};
use agent_core::local_time::Zone;
use agent_core::plan::judge::JudgementState;
use agent_core::protocol::{
    Client, Command, PlanStepSummary, Reply, RoundDetail, ShowRequest, StatusRequest, Target, TeamSummary, Versions,
};
use agent_core::review_findings::Severity;
use agent_core::state::{Round, RoundLife, RunState, Timestamp};
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::{AgentBus, AgentBusError, HelperEntry, HelperRecord};
use agent_runtime::clock::{SharedClock, SystemClock};
use agent_runtime::debug::{DebugRuntimeError, load_debug_state};
use agent_runtime::disk::GIBIBYTE;
use agent_runtime::history::{ReadError, Reader};
use agent_runtime::process::error_chain;
use agent_runtime::run::activity::last_activity;
use agent_runtime::settings::{Settings, SettingsError};
use agent_runtime::store::Home;
use time::UtcOffset;

use crate::client::{ClientError, Connection};
use crate::versions::of_this_build;

/// Why `agent status` failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StatusError {
    /// The daemon could not be asked.
    #[error("cannot ask the daemon for the status")]
    Client(#[from] ClientError),
    /// The state file is there but cannot be read, so the refused agents cannot be shown.
    #[error("cannot read the state to show the refused agents")]
    Refusals(#[source] DebugRuntimeError),
    /// The clock of the system gives a time that the state cannot hold.
    #[error("cannot tell the time to show the refused agents")]
    Clock(#[source] agent_core::state::InvalidTimestamp),
    /// The daemon answered with something else than a list of teams.
    #[error("the daemon answered a status request with {kind}")]
    UnexpectedReply {
        /// What it answered.
        kind: &'static str,
    },
    /// The settings of the home, which name the investigators, cannot be read.
    #[error("cannot read the settings of the home")]
    Settings(#[source] SettingsError),
    /// The configuration file cannot be read.
    #[error("cannot read the configuration file {path:?}")]
    ConfigRead {
        /// The file whose configuration could not be read.
        path: PathBuf,
        /// The I/O failure, kept as the cause.
        source: io::Error,
    },
    /// The bus could not be queried.
    #[error("cannot read worker requests from the bus")]
    Bus(#[source] AgentBusError),
    /// The configuration file is not valid.
    #[error("cannot parse the configuration file {path:?}")]
    ConfigParse {
        /// The file whose configuration could not be parsed.
        path: PathBuf,
        /// The parsed configuration's reason, kept as the cause.
        source: agent_core::config::ConfigError,
    },
    /// The output cannot be written.
    #[error("cannot write the status")]
    Output(#[source] io::Error),
    /// The teams cannot be written as JSON.
    #[error("cannot write the status as JSON")]
    Json(#[source] serde_json::Error),
}

/// Where `agent status` reads when a task that runs was last active, and the zone it tells the time in.
#[derive(Debug, Clone)]
pub(crate) struct Activity {
    /// The home whose runs are looked at.
    home: Home,
    /// The zone the times are written in.
    zone: Zone,
    /// Where the requests for workers of the tasks shown are read.
    requests: requests::Source,
    /// The clock used to decide which provider cooldowns are still active.
    clock: SharedClock,
}

impl Activity {
    /// The activity of the tasks of `home`, told in the local time `zone` gives.
    pub(crate) fn new(home: Home, zone: Zone) -> Self {
        Self {
            home,
            zone,
            requests: requests::Source::Unread,
            clock: SharedClock::new(SystemClock),
        }
    }

    /// The same, with the requests for workers of the tasks shown read from `source`.
    pub(crate) fn with_requests(self, source: requests::Source) -> Self {
        Self {
            requests: source,
            ..self
        }
    }

    /// What the line of the running task `task` says of its last activity: the time the output of its runs last changed,
    /// or that it cannot be told; `None` when its runs have written nothing yet.
    fn of(&self, task: &TaskId) -> Option<String> {
        match last_activity(&self.home, task) {
            Ok(Some(changed)) => Some(format!("last activity {}", clock_time(changed, self.zone))),
            Ok(None) => None,
            Err(error) => Some(format!("last activity unknown ({})", error_chain(&error))),
        }
    }

    /// The line about low free disk space when below threshold, or the reason the space or the limit cannot be read.
    /// `None` before the daemon has measured the space, and when the space is above the limit.
    pub(crate) fn disk_line(&self) -> Option<String> {
        let content = match std::fs::read_to_string(self.home.free_space_file()) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(error) => return Some(format!("disk: cannot read the free space ({})\n", error_chain(&error))),
        };
        let free_bytes = match content.trim().parse::<u64>() {
            Ok(bytes) => bytes,
            Err(error) => return Some(format!("disk: cannot read the free space ({error})\n")),
        };
        let low_free_gb = match self.low_free_gb() {
            Ok(gb) => gb,
            Err(message) => return Some(message),
        };
        let free_gb = free_bytes / GIBIBYTE;
        if free_gb < low_free_gb {
            Some(format!("disk: {free_gb} GB free, limit {low_free_gb} GB\n"))
        } else {
            None
        }
    }

    /// The `disk.low_free_gb` of the configuration: the default when the file is absent, the message to show when it
    /// cannot be read or parsed.
    fn low_free_gb(&self) -> Result<u64, String> {
        match std::fs::read_to_string(self.home.config_file()) {
            Ok(text) => agent_core::config::Config::from_toml_str(&text)
                .map(|config| config.disk().low_free_gb)
                .map_err(|error| format!("disk: cannot read the configuration ({})\n", error_chain(&error))),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(agent_core::config::Disk::default().low_free_gb)
            }
            Err(error) => Err(format!(
                "disk: cannot read the configuration ({})\n",
                error_chain(&error)
            )),
        }
    }

    /// The state of the daemon and the time of the status, read once; `None` when no state is written yet.
    ///
    /// # Errors
    /// [`StatusError::Refusals`] when the state file exists but cannot be read, [`StatusError::Clock`] when the time
    /// cannot be read.
    async fn state_now(&self) -> Result<Option<(agent_core::state::State, Timestamp)>, StatusError> {
        let state = match load_debug_state(&self.home).await {
            Ok(state) => state,
            Err(DebugRuntimeError::Missing { .. }) => return Ok(None),
            Err(source) => return Err(StatusError::Refusals(source)),
        };
        let now = Timestamp::now(&self.clock).map_err(StatusError::Clock)?;
        Ok(Some((state, now)))
    }

    /// The line of the agents their provider refused at the time of the status, read from the state of the daemon; `None`
    /// when no state is written yet or no agent is refused.
    ///
    /// # Errors
    /// [`StatusError::Refusals`] when the state file exists but cannot be read.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the status reads both lines with `Activity::reads`; the tests of the line use this one"
        )
    )]
    pub(crate) async fn refused_line(&self) -> Result<Option<String>, StatusError> {
        Ok(self
            .state_now()
            .await?
            .and_then(|(state, now)| refusals::refused_line(&state.refusals_at(now), self.zone)))
    }

    /// What one render of the status takes from the configuration and from the state of the daemon, each file read once:
    /// the line counting the runs going per configured CLI across `teams` (when at least one run is going), the account
    /// each agent and investigator runs with, the lines of the paused accounts, of the agents their provider refused
    /// and of the limit readings the state keeps (`None` when there is none of them), and what each queued task of `teams` whose account has its `max_runs`
    /// going says of it. A missing configuration or state file is the only reason one of them is omitted.
    ///
    /// # Errors
    /// [`StatusError::ConfigRead`] when the configuration cannot be read, [`StatusError::ConfigParse`] when it is
    /// invalid, [`StatusError::Refusals`] when the state file exists but cannot be read.
    pub(crate) async fn reads(&self, teams: &[TeamSummary]) -> Result<Reads, StatusError> {
        let config = self.config().await?;
        let state = self.state_now().await?;
        let provider_lines = state
            .as_ref()
            .and_then(|(state, now)| provider_lines(state, *now, self.zone));
        let account_waits = match (&config, &state) {
            (Some(config), Some((state, _))) => accounts::waits(config, state, teams),
            (None, _) | (_, None) => BTreeMap::new(),
        };
        let shifts = match (&config, &state) {
            (Some(config), Some((state, now))) => shifts(config, state, *now, teams),
            (None, _) | (_, None) => BTreeMap::new(),
        };
        let (cli_runs, accounts) = config.map_or_else(
            || (None, BTreeMap::new()),
            |config| {
                (
                    cli_runs_line(&config, teams),
                    agent_core::account_view::agent_accounts(&config),
                )
            },
        );
        Ok(Reads {
            cli_runs,
            accounts,
            provider_lines,
            account_waits,
            shifts,
        })
    }

    /// The line counting the runs going per configured CLI across `teams`, when at least one run is going. A missing
    /// configuration has no configured CLIs and is the only reason a configuration is omitted. What the status prints
    /// reads the file once, through [`Activity::reads`].
    ///
    /// # Errors
    /// [`StatusError::ConfigRead`] when the file cannot be read, or [`StatusError::ConfigParse`] when it is invalid.
    #[cfg(test)]
    pub(crate) async fn cli_runs_line(&self, teams: &[TeamSummary]) -> Result<Option<String>, StatusError> {
        Ok(self.config().await?.and_then(|config| cli_runs_line(&config, teams)))
    }

    /// The account each agent and investigator of the configuration runs with; none when there is no configuration.
    ///
    /// # Errors
    /// [`StatusError::ConfigRead`] when the file cannot be read, or [`StatusError::ConfigParse`] when it is invalid.
    async fn accounts(&self) -> Result<members::Accounts, StatusError> {
        Ok(self.config().await?.map_or_else(BTreeMap::new, |config| {
            agent_core::account_view::agent_accounts(&config)
        }))
    }

    /// The configuration, read asynchronously so status rendering never blocks its runtime thread; `None` when the file
    /// does not exist, the only reason a configuration is omitted.
    ///
    /// # Errors
    /// [`StatusError::ConfigRead`] when the file cannot be read, or [`StatusError::ConfigParse`] when it is invalid.
    async fn config(&self) -> Result<Option<agent_core::config::Config>, StatusError> {
        let path = self.home.config_file();
        match tokio::fs::read_to_string(&path).await {
            Ok(text) => agent_core::config::Config::from_toml_str(&text)
                .map(Some)
                .map_err(|source| StatusError::ConfigParse { path, source }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(StatusError::ConfigRead { path, source }),
        }
    }
}

/// `at` as a person reads it in `zone`: the date, the time with its seconds and the zone.
pub(crate) fn clock_time(at: SystemTime, zone: Zone) -> String {
    let utc = time::UtcDateTime::from(at);
    match Timestamp::from_utc(utc) {
        Ok(at) => zone.stamp(at),
        // A time the state cannot hold (outside the years 0 to 9999) is shown in UTC and marked as UTC.
        Err(
            agent_core::state::InvalidTimestamp::YearOutOfRange { .. }
            | agent_core::state::InvalidTimestamp::NotRfc3339 { .. },
        ) => {
            let year = utc.year();
            let year_text = if year < 0 {
                format!("-{:04}", year.unsigned_abs())
            } else {
                format!("{year:04}")
            };
            format!(
                "{year_text}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
                u8::from(utc.month()),
                utc.day(),
                utc.hour(),
                utc.minute(),
                utc.second(),
            )
        }
    }
}

/// `at` as a person reads it in `zone`: the time of a recorded activity.
pub(crate) fn clock_of(at: Timestamp, zone: Zone) -> String {
    zone.stamp(at)
}

/// What a list of the teams prints of each team: the short view of the team, its rounds, its tasks that are alive and
/// a count of its tasks, or every task with its plan, and the parts, as before the short view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Detail {
    /// The team, its rounds, a line for each of its tasks (its coordinator, members, workers and reviewers) and a
    /// count of all its tasks, with the questions that wait for answers named.
    Short,
    /// The team, its rounds, its tasks with their plans, and its parts.
    Tasks,
}

/// What a list of `agent status` says, before it is written: its text, the versions of the daemon that answered and
/// what the history says when it is not whole.
struct Shown {
    /// The lines of the list, with what the rounds say after their lines.
    text: String,
    /// The line of runs going per configured CLI, when at least one run is going.
    cli_runs: Option<String>,
    /// The line of the agents their provider refused and their return times, when any is refused.
    refused: Option<String>,
    /// The lines describing pool workers, when configured.
    pool: Option<String>,
    /// The versions of the daemon that answered; absent from a daemon before it told them.
    daemon: Option<Versions>,
    /// What the history is doing, when it is not whole.
    history: Option<HistoryHealth>,
}

/// Whether a status request negotiates the daemon's crate versions for the opening list header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionsQuery {
    /// Negotiates protocol seven so the daemon attaches `daemon_versions` to the status reply.
    Include,
    /// Uses ordinary executor-aware status negotiation without requesting `daemon_versions`.
    Omit,
}

/// The context line of `task`: what its last run's stream told of its context, read from the history of `home`;
/// "context: not told" when the last run has no usage, and a line that says the history cannot be read when it cannot
/// be (the rest of the status is shown all the same).
pub(crate) fn context_line(home: &Home, task: &TaskId) -> String {
    match last_usage(home, task) {
        Ok(Some(usage)) => format!(
            "context: first {} tokens, last {} tokens (last run)\n",
            told_part(usage.first_context),
            told_part(usage.last_context),
        ),
        Ok(None) => "context: not told\n".to_owned(),
        Err(error) => format!("context: cannot be read ({})\n", error_chain(&error)),
    }
}

/// The usage of the last run of `task` as its row of the history says; `None` when the task has no run or the run has
/// no usage row.
///
/// # Errors
/// [`ReadError`] when the history cannot be read.
fn last_usage(home: &Home, task: &TaskId) -> Result<Option<RunUsage>, ReadError> {
    let reader = Reader::open(&home.history_file())?;
    let newest = reader.newest_runs_of_task(task, NonZeroUsize::MIN)?;
    let Some(last) = newest.runs.last() else {
        return Ok(None);
    };
    reader.run_usage(&last.start.value.run)
}

/// A count of tokens as the status shows it, or "not told" when the CLI did not tell it.
fn told_part(value: Option<u64>) -> String {
    value.map_or_else(|| "not told".to_owned(), |value| value.to_string())
}

/// Asks the daemon for the status of `target` (every team without one) and writes it to `out`, its versions first; the
/// tasks that run say when they were last active. A team or a task is shown with its tasks whatever `detail` is.
///
/// # Errors
/// [`StatusError`].
pub(crate) async fn status_shown(
    connection: &mut Connection,
    target: Option<Target>,
    activity: &Activity,
    detail: Detail,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    if let Some(Target::Round(round)) = target {
        return show_round(connection, round, Some(activity), out).await;
    }
    let is_list = target.is_none();
    let query = if is_list {
        VersionsQuery::Include
    } else {
        VersionsQuery::Omit
    };
    let task = match &target {
        Some(Target::Task(task)) => Some(task.clone()),
        _ => None,
    };
    let shown = listing_text(connection, target, activity, detail, query).await?;
    let prefix = if is_list {
        versions::block(&of_this_build(), shown.daemon.as_ref())
    } else {
        String::new()
    };
    write_shown(out, &prefix, &shown, activity)?;
    if let Some(task) = task {
        // SQLite is read off the async runtime: the history may be busy for as long as its timeout.
        let home = activity.home.clone();
        let line = match tokio::task::spawn_blocking(move || context_line(&home, &task)).await {
            Ok(line) => line,
            // The line is the only part of the status that waits for the history: it says why it is missing.
            Err(error) => format!("context: cannot be read ({})\n", error_chain(&error)),
        };
        out.write_all(line.as_bytes()).map_err(StatusError::Output)?;
    }
    Ok(())
}

/// Asks the daemon for every team and writes to `out` those that have a round that goes on, with those rounds only:
/// what is being worked on now, its versions first. `detail` says whether the tasks of the teams are written too.
///
/// # Errors
/// [`StatusError`].
pub(crate) async fn status_running_shown(
    connection: &mut Connection,
    activity: &Activity,
    detail: Detail,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    let shown = running_text(connection, activity, detail, VersionsQuery::Include).await?;
    write_shown(
        out,
        &versions::block(&of_this_build(), shown.daemon.as_ref()),
        &shown,
        activity,
    )
}

/// [`status_shown`] without the versions, with the tasks: what the tests of the status compare.
#[cfg(test)]
pub(crate) async fn status_active(
    connection: &mut Connection,
    target: Option<Target>,
    activity: &Activity,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    if let Some(Target::Round(round)) = target {
        return show_round(connection, round, Some(activity), out).await;
    }
    let shown = listing_text(connection, target, activity, Detail::Tasks, VersionsQuery::Omit).await?;
    write_shown(out, "", &shown, activity)
}

/// [`status_running_shown`] without the versions, with the tasks: what the tests of the status compare.
#[cfg(test)]
pub(crate) async fn status_running(
    connection: &mut Connection,
    activity: &Activity,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    let shown = running_text(connection, activity, Detail::Tasks, VersionsQuery::Omit).await?;
    write_shown(out, "", &shown, activity)
}

/// What the teams of `target` (every team without one) say, in `detail`, with their rounds and what the rounds say.
async fn listing_text(
    connection: &mut Connection,
    target: Option<Target>,
    activity: &Activity,
    detail: Detail,
    query: VersionsQuery,
) -> Result<Shown, StatusError> {
    let is_targeted = target.is_some();
    // A team or a task is shown with its tasks, as it always was.
    let detail = if is_targeted { Detail::Tasks } else { detail };
    let listing = teams(connection, target, query).await?;
    let told = rounds::told(connection, activity, &listing.teams).await?;
    let requests = requests_of(activity, &listing.teams, detail).await;
    let reads = activity.reads(&listing.teams).await?;
    let text = render_with_notes(
        &listing.teams,
        View::Full,
        detail,
        &|task| activity.of(task),
        &|round| told.of(round),
        Notes {
            extras: Extras {
                under: Under {
                    requests: &|task| requests.of(task).to_vec(),
                    workers: &|_| Vec::new(),
                },
                accounts: &reads.accounts,
            },
            waits: &|task| reads.account_waits.get(task).cloned(),
            shifts: &|task| reads.shifts.get(task).cloned(),
        },
        activity.zone,
    );
    let pool = if !is_targeted && detail == Detail::Short {
        pool_line(activity, &listing.teams).await?
    } else {
        None
    };
    Ok(Shown {
        text: format!("{text}{}", told.unread()),
        cli_runs: reads.cli_runs,
        refused: reads.provider_lines,
        pool,
        daemon: listing.daemon,
        history: listing.history,
    })
}

/// What the teams with a round that goes on say, in `detail`, with those rounds only.
async fn running_text(
    connection: &mut Connection,
    activity: &Activity,
    detail: Detail,
    query: VersionsQuery,
) -> Result<Shown, StatusError> {
    let listing = teams(connection, None, query).await?;
    let reads = activity.reads(&listing.teams).await?;
    // A team is listed when it has a round that goes on; the parts it prints come from the whole team, since a merged
    // part's round is one that is over.
    let listed: Vec<TeamSummary> = listing
        .teams
        .iter()
        .filter(|team| team.rounds.iter().any(|round| *round.life() == RoundLife::Open))
        .cloned()
        .collect();
    // The short view names the workers that serve: under a listed team those that took its requests, in the pool the
    // busy ones of the other teams; a free worker is not named.
    let observed = if detail == Detail::Short {
        observe_pool(activity).await?
    } else {
        None
    };
    let pool = observed.as_ref().and_then(|observed| {
        let helpers = observed.entries();
        pool::render_busy(
            &observed.workers,
            &listing.teams,
            Some(&helpers),
            Some(&observed.unsettled),
            &listed,
        )
    });
    let served = served_lines(activity, &listed, observed.as_ref()).await;
    let text = if listed.is_empty() {
        NO_OPEN_ROUNDS.to_owned()
    } else {
        let told = rounds::told(connection, activity, &listed).await?;
        let requests = requests_of(activity, &listed, detail).await;
        let text = render_with_notes(
            &listed,
            View::Running,
            detail,
            &|task| activity.of(task),
            &|round| told.of(round),
            Notes {
                extras: Extras {
                    under: Under {
                        requests: &|task| requests.of(task).to_vec(),
                        workers: &|team| {
                            served
                                .iter()
                                .find(|(id, _)| *id == team.id)
                                .map(|(_, lines)| lines.clone())
                                .unwrap_or_default()
                        },
                    },
                    accounts: &reads.accounts,
                },
                waits: &|task| reads.account_waits.get(task).cloned(),
                shifts: &|task| reads.shifts.get(task).cloned(),
            },
            activity.zone,
        );
        format!("{text}{}", told.unread())
    };
    Ok(Shown {
        text,
        cli_runs: reads.cli_runs,
        refused: reads.provider_lines,
        pool,
        daemon: listing.daemon,
        history: listing.history,
    })
}

/// The requests for workers of the tasks of `teams` that a view in `detail` shows: those of every task when the tasks
/// are shown with their plans, none in the short view.
async fn requests_of(activity: &Activity, teams: &[TeamSummary], detail: Detail) -> requests::Told {
    match detail {
        Detail::Short => requests::Told::default(),
        Detail::Tasks => {
            let tasks = teams
                .iter()
                .flat_map(|team| team.members.iter().map(|member| &member.task));
            match Timestamp::now(&SystemClock) {
                Ok(now) => {
                    let shown = requests::Shown {
                        now,
                        zone: activity.zone,
                    };
                    requests::told_waiting(&activity.requests, tasks, &activity.home, shown).await
                }
                Err(error) => requests::told(&requests::Source::Unknown(error_chain(&error)), tasks).await,
            }
        }
    }
}

/// What the pool of workers of a home is doing: its workers, the requests the bus lists and the workspaces that are
/// not settled.
struct PoolObservation {
    /// The workers of `[tiers] workers`.
    workers: Vec<AgentName>,
    /// Every request for a worker the bus lists, the finished ones too.
    records: Vec<HelperRecord>,
    /// The workspaces of workers whose end is not settled.
    unsettled: Vec<pool::UnsettledWorker>,
}

impl PoolObservation {
    /// The requests as the pool reads them.
    fn entries(&self) -> Vec<HelperEntry> {
        self.records.iter().map(|record| record.entry.clone()).collect()
    }
}

/// Reads the configured pool of workers of the home of `activity` and the requests for them; `None` when the home has
/// no configuration or configures no worker.
async fn observe_pool(activity: &Activity) -> Result<Option<PoolObservation>, StatusError> {
    let settings = match Settings::load(&activity.home).await {
        Ok(s) => s,
        Err(SettingsError::MissingConfig { .. }) => return Ok(None),
        Err(err) => return Err(StatusError::Settings(err)),
    };
    let workers = settings.config.worker_agents().to_vec();
    if workers.is_empty() {
        return Ok(None);
    }
    let bus = AgentBus::new(
        settings.config.binaries().agentbus.clone(),
        activity.home.absolute_root().clone(),
    );
    let records = bus.helper_records(true).await.map_err(StatusError::Bus)?;
    let observed = PoolObservation {
        workers,
        records,
        unsettled: Vec::new(),
    };
    let unsettled = scan_unsettled_workspaces(&activity.home, &observed.entries()).await;
    Ok(Some(PoolObservation { unsettled, ..observed }))
}

/// Observes the configured pool of workers and reads helper requests asynchronously.
async fn pool_line(activity: &Activity, teams: &[TeamSummary]) -> Result<Option<String>, StatusError> {
    let Some(observed) = observe_pool(activity).await? else {
        return Ok(None);
    };
    Ok(pool::render_pool_with_unsettled(
        &observed.workers,
        teams,
        Some(&observed.entries()),
        Some(&observed.unsettled),
    ))
}

/// The lines of the workers that served each team of `teams`, as `observed` lists their requests, with the times of
/// their runs in the home of `activity`.
async fn served_lines(
    activity: &Activity,
    teams: &[TeamSummary],
    observed: Option<&PoolObservation>,
) -> Vec<(TeamId, Vec<workers::Served>)> {
    let Some(observed) = observed else {
        return Vec::new();
    };
    let mut served = Vec::new();
    for team in teams {
        let mut lines = Vec::new();
        for record in workers::of_team(team, &observed.records) {
            let span = match &record.helper_task {
                Some(task) => workers::span_of(&activity.home, task).await,
                None => workers::Span::default(),
            };
            lines.push(workers::served(record, span, activity.zone));
        }
        served.push((team.id.clone(), lines));
    }
    served
}

/// Scans task directories in `home` for unsettled workspace records and records damaged entries.
async fn scan_unsettled_workspaces(
    home: &Home,
    helpers: &[agent_runtime::agentbus::HelperEntry],
) -> Vec<pool::UnsettledWorker> {
    let tasks_dir = home.tasks_dir();
    let mut entries = match tokio::fs::read_dir(&tasks_dir).await {
        Ok(entries) => entries,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(source) => return vec![damaged_pool_workspace(&tasks_dir, &source)],
    };
    let mut result = Vec::new();
    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(source) => {
                result.push(damaged_pool_workspace(&tasks_dir, &source));
                break;
            }
        };
        let task_path = entry.path();
        match entry.file_type().await {
            Ok(kind) if !kind.is_dir() => continue,
            Ok(_) => {}
            Err(source) => {
                result.push(damaged_pool_workspace(&task_path, &source));
                continue;
            }
        }
        match agent_runtime::request_workspace::read_each(&task_path).await {
            Ok(records) => {
                for rec_res in records {
                    match rec_res {
                        Ok(ws) => {
                            let rec = ws.record();
                            let step = helpers
                                .iter()
                                .find(|h| h.id == rec.request)
                                .and_then(|h| h.step.clone());
                            result.push(pool::UnsettledWorker {
                                worker: Some(rec.worker.clone()),
                                from_task: rec.requester_task.to_string(),
                                step,
                                damaged: false,
                            });
                        }
                        Err(source) => result.push(damaged_pool_workspace(&task_path, &source)),
                    }
                }
            }
            Err(source) => result.push(damaged_pool_workspace(&task_path, &source)),
        }
    }
    result
}

/// A damaged record cannot prove any worker free; report the cause and retain that uncertainty.
fn damaged_pool_workspace(path: &std::path::Path, source: &dyn std::error::Error) -> pool::UnsettledWorker {
    tracing::warn!(
        event = agent_core::log::LogEvent::WorkerRequestDamaged.as_str(),
        path = %path.display(),
        error = %error_chain(source),
        "cannot scan worker workspace: pool availability is unknown"
    );
    pool::UnsettledWorker {
        worker: None,
        from_task: String::new(),
        step: None,
        damaged: true,
    }
}

/// Writes `prefix`, then what `shown` says, then the line of runs per CLI if any, the line about low disk space if any
/// and the line about the history when it is not whole.
fn write_shown(out: &mut impl Write, prefix: &str, shown: &Shown, activity: &Activity) -> Result<(), StatusError> {
    let cli_runs = shown.cli_runs.as_deref().unwrap_or_default();
    let refused = shown.refused.as_deref().unwrap_or_default();
    let pool = shown.pool.as_deref().unwrap_or_default();
    write(
        out,
        &format!("{prefix}{}{cli_runs}{refused}{pool}", shown.text),
        shown.history.as_ref(),
        activity.disk_line().as_deref(),
    )
}

/// [`status_shown`] without the versions and with the teams as the test helper of the status compares them: what the
/// tests of the status ask for.
#[cfg(test)]
pub(crate) async fn status(
    connection: &mut Connection,
    target: Option<Target>,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    if let Some(Target::Round(round)) = target {
        return show_round(connection, round, None, out).await;
    }
    let listing = teams(connection, target, VersionsQuery::Omit).await?;
    write(out, &render(&listing.teams), listing.history.as_ref(), None)
}

/// Asks the daemon for every team and writes to `out` what `investigators::render` says of the investigators configured in
/// `home`: for each of them its last two activities in the teams that are not closed, and the one that has none as free.
///
/// # Errors
/// [`StatusError`]: the settings of `home` cannot be read, the daemon cannot be asked or answers with something else
/// than a list of teams, or the output cannot be written.
pub(crate) async fn status_investigators(
    connection: &mut Connection,
    home: &Home,
    zone: Zone,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    let settings = Settings::load(home).await.map_err(StatusError::Settings)?;
    // The configuration keeps the investigators by name: the view lists them in that order.
    let configured: Vec<_> = settings.config.investigators().keys().cloned().collect();
    let listing = teams(connection, None, VersionsQuery::Omit).await?;
    let text = investigators::render(&listing.teams, &configured, zone);
    out.write_all(text.as_bytes()).map_err(StatusError::Output)?;
    out.flush().map_err(StatusError::Output)
}

/// The teams of `teams` that have an open round, each with its open rounds only.
#[cfg(test)]
pub(crate) fn running(teams: Vec<TeamSummary>) -> Vec<TeamSummary> {
    teams
        .into_iter()
        .filter_map(|mut team| {
            team.rounds.retain(|round| *round.life() == RoundLife::Open);
            (!team.rounds.is_empty()).then_some(team)
        })
        .collect()
}

/// Asks the daemon for the status of `target` (every team without one) and writes the teams to `out` as one JSON array,
/// each team with its description.
///
/// # Errors
/// [`StatusError`].
pub(crate) async fn status_json(
    connection: &mut Connection,
    target: Option<Target>,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    let listing = teams(connection, target, VersionsQuery::Omit).await?;
    out.write_all(render_json(&listing.teams)?.as_bytes())
        .map_err(StatusError::Output)?;
    out.flush().map_err(StatusError::Output)
}

/// The teams as one JSON array and a newline.
///
/// # Errors
/// [`StatusError::Json`] when the teams cannot be written as JSON.
pub(crate) fn render_json(teams: &[TeamSummary]) -> Result<String, StatusError> {
    let mut text = serde_json::to_string(teams).map_err(StatusError::Json)?;
    text.push('\n');
    Ok(text)
}

/// What the daemon answered to a status request.
struct Listing {
    /// The teams asked for.
    teams: Vec<TeamSummary>,
    /// What the history is doing, when it is not whole.
    history: Option<HistoryHealth>,
    /// The versions of the daemon; absent from a daemon before it told them.
    daemon: Option<Versions>,
}

/// The teams the daemon answers a status request for `target` with, what it says of the history, and its versions.
async fn teams(
    connection: &mut Connection,
    target: Option<Target>,
    query: VersionsQuery,
) -> Result<Listing, StatusError> {
    let command = Command::Status(StatusRequest { target });
    let reply = match query {
        VersionsQuery::Include => connection.request_versions(Client::Cli, command).await?,
        VersionsQuery::Omit => connection.request(Client::Cli, command).await?,
    };
    // Read before any other request: the next answer replaces the versions of this one.
    let daemon = connection.daemon_versions().cloned();
    match reply {
        Reply::Teams { teams } => Ok(Listing {
            teams,
            history: None,
            daemon,
        }),
        Reply::Overview { teams, history } => Ok(Listing {
            teams,
            history: Some(history),
            daemon,
        }),
        other => Err(StatusError::UnexpectedReply { kind: kind_of(&other) }),
    }
}

/// Writes `text`, then the line about low disk space if any, then the line about the history when it is not whole.
fn write(
    out: &mut impl Write,
    text: &str,
    history: Option<&HistoryHealth>,
    disk: Option<&str>,
) -> Result<(), StatusError> {
    out.write_all(text.as_bytes()).map_err(StatusError::Output)?;
    if let Some(disk) = disk {
        out.write_all(disk.as_bytes()).map_err(StatusError::Output)?;
    }
    out.write_all(render_history(history).as_bytes())
        .map_err(StatusError::Output)?;
    out.flush().map_err(StatusError::Output)
}

/// Asks the daemon to show the round `round`.
async fn round_detail(connection: &mut Connection, round: RoundId) -> Result<RoundDetail, StatusError> {
    let reply = connection
        .request(
            Client::Cli,
            Command::Show(ShowRequest {
                target: Target::Round(round),
            }),
        )
        .await?;
    match reply {
        Reply::Round { round } => Ok(*round),
        other => Err(StatusError::UnexpectedReply { kind: kind_of(&other) }),
    }
}

/// Asks the daemon to show the round `round` and writes it to `out`.
async fn show_round(
    connection: &mut Connection,
    round: RoundId,
    activity: Option<&Activity>,
    out: &mut impl Write,
) -> Result<(), StatusError> {
    let detail = round_detail(connection, round.clone()).await?;
    let zone = activity.map_or(Zone::Fixed(UtcOffset::UTC), |activity| activity.zone);
    let accounts = match activity {
        Some(activity) => activity.accounts().await?,
        None => BTreeMap::new(),
    };
    let mut text = render_round_with_accounts(&detail, zone, &accounts);
    if let Some(activity) = activity {
        text.push_str(&snapshots::snapshots_of(&activity.home, &round, activity.zone));
    }
    out.write_all(text.as_bytes()).map_err(StatusError::Output)?;
    out.flush().map_err(StatusError::Output)
}

/// The name of a kind of reply, for an error message.
pub(super) const fn kind_of(reply: &Reply) -> &'static str {
    match reply {
        Reply::Started { .. } => "started",
        Reply::Teams { .. } => "teams",
        Reply::Overview { .. } => "overview",
        Reply::Team { .. } => "team",
        Reply::Round { .. } => "round",
        Reply::Task { .. } => "task",
        Reply::Applied { .. } => "applied",
        Reply::Gate { .. } => "gate",
        Reply::GateRecorded { .. } => "gate_recorded",
        Reply::Subscribed { .. } => "subscribed",
        Reply::Removed { .. } => "removed",
    }
}

/// The teams as text: a line for each team, and under it a line for each of its tasks, without the last activity of the
/// tasks: what the tests of the layout compare.
#[cfg(test)]
pub(crate) fn render(teams: &[TeamSummary]) -> String {
    render_active(teams, &|_| None)
}

/// The teams as text: a line for each team, and under it a line for each of its tasks, with what `activity` says of each
/// task that runs after its status; the rounds say nothing beyond their lines.
pub(crate) fn render_active(teams: &[TeamSummary], activity: &dyn Fn(&TaskId) -> Option<String>) -> String {
    render_at(teams, activity, &|_| String::new(), Zone::Fixed(UtcOffset::UTC))
}

/// [`render_active`] with what `rounds` says of each round after its line, the times in UTC: what the tests of the
/// rounds compare.
#[cfg(test)]
fn render_with(
    teams: &[TeamSummary],
    activity: &dyn Fn(&TaskId) -> Option<String>,
    rounds: &dyn Fn(&Round) -> String,
) -> String {
    render_at(teams, activity, rounds, Zone::Fixed(UtcOffset::UTC))
}

/// [`render_active`] with what `rounds` says of each round after its line, the times of the recorded activities told in
/// `zone`.
fn render_at(
    teams: &[TeamSummary],
    activity: &dyn Fn(&TaskId) -> Option<String>,
    rounds: &dyn Fn(&Round) -> String,
    zone: Zone,
) -> String {
    render_in(teams, View::Full, Detail::Tasks, activity, rounds, zone)
}

/// What a view of the status prints, the running view being the same teams with their open rounds only.
#[cfg(test)]
pub(crate) fn render_running(teams: &[TeamSummary]) -> String {
    render_in(
        teams,
        View::Running,
        Detail::Tasks,
        &|_| None,
        &|_| String::new(),
        Zone::Fixed(UtcOffset::UTC),
    )
}

/// Which view of the status is printed: the full one, or the running one that limits the round lines to the open ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum View {
    /// Every round of a team.
    Full,
    /// The open rounds only.
    Running,
}

fn render_in(
    teams: &[TeamSummary],
    view: View,
    detail: Detail,
    activity: &dyn Fn(&TaskId) -> Option<String>,
    rounds: &dyn Fn(&Round) -> String,
    zone: Zone,
) -> String {
    let notes = Notes {
        extras: Extras {
            under: Under {
                requests: &|_| Vec::new(),
                workers: &|_| Vec::new(),
            },
            accounts: &BTreeMap::new(),
        },
        waits: &|_| None,
        shifts: &|_| None,
    };
    render_with_notes(teams, view, detail, activity, rounds, notes, zone)
}

/// What [`render_with_notes`] writes under the lines of a team: the requests for workers under each task's plan,
/// and the workers that served the team before its count of tasks.
#[derive(Clone, Copy)]
struct Under<'a> {
    /// The lines of the requests for workers of a task.
    requests: &'a dyn Fn(&TaskId) -> Vec<String>,
    /// What the lines of the workers that served a team say, padded to the columns of the tasks when written.
    workers: &'a dyn Fn(&TeamSummary) -> Vec<workers::Served>,
}

/// What [`render_with_notes`] writes besides the tasks themselves: what goes under them, and the account of each
/// agent that has one, which the line of its task shows after its name.
#[derive(Clone, Copy)]
struct Extras<'a> {
    /// What goes under the lines of a team.
    under: Under<'a>,
    /// The account of each agent and investigator that runs with one.
    accounts: &'a members::Accounts,
}

/// [`render_in`] with the lines of the requests for workers of each task under its plan, as `requests` gives them, and no
/// account told: what the tests of those lines compare.
#[cfg(test)]
fn render_with_requests(
    teams: &[TeamSummary],
    view: View,
    detail: Detail,
    activity: &dyn Fn(&TaskId) -> Option<String>,
    rounds: &dyn Fn(&Round) -> String,
    under: Under<'_>,
    zone: Zone,
) -> String {
    let waiting = Waiting {
        under,
        accounts: &|_| None,
    };
    render_with_waits(teams, view, detail, activity, rounds, waiting, zone)
}

/// What [`render_with_waits`] writes besides the lines of the tasks: what goes under them, and what a queued task whose
/// account has its most runs going says of it.
#[cfg(test)]
#[derive(Clone, Copy)]
struct Waiting<'a> {
    /// The lines under the lines of a team.
    under: Under<'a>,
    /// What a queued task says of the account it waits for, such as `waiting for account comp-claude (1/1 runs)`.
    accounts: &'a dyn Fn(&TaskId) -> Option<String>,
}

/// [`render_with_requests`] with what the tasks that wait for their account say, and no account told after an agent.
#[cfg(test)]
fn render_with_waits(
    teams: &[TeamSummary],
    view: View,
    detail: Detail,
    activity: &dyn Fn(&TaskId) -> Option<String>,
    rounds: &dyn Fn(&Round) -> String,
    waiting: Waiting<'_>,
    zone: Zone,
) -> String {
    let notes = Notes {
        extras: Extras {
            under: waiting.under,
            accounts: &BTreeMap::new(),
        },
        waits: waiting.accounts,
        shifts: &|_| None,
    };
    render_with_notes(teams, view, detail, activity, rounds, notes, zone)
}

/// `render_with_requests` with the account of each agent that has one told after its name, and no task told to wait for
/// its account.
#[cfg(test)]
fn render_with_accounts(
    teams: &[TeamSummary],
    view: View,
    detail: Detail,
    activity: &dyn Fn(&TaskId) -> Option<String>,
    rounds: &dyn Fn(&Round) -> String,
    extras: Extras<'_>,
    zone: Zone,
) -> String {
    let notes = Notes {
        extras,
        waits: &|_| None,
        shifts: &|_| None,
    };
    render_with_notes(teams, view, detail, activity, rounds, notes, zone)
}

/// What [`render_with_notes`] writes besides the tasks themselves: what [`Extras`] holds, and what a queued task whose
/// account has its most runs going says of it.
#[derive(Clone, Copy)]
struct Notes<'a> {
    /// What goes under the lines of a team and the account of each agent.
    extras: Extras<'a>,
    /// What a queued task says of the account it waits for, such as `waiting for account comp-claude (1/1 runs)`.
    waits: &'a dyn Fn(&TaskId) -> Option<String>,
    /// What a lead task says of its shift time worked, such as `shift: 1h40m of 2h worked`.
    shifts: &'a dyn Fn(&TaskId) -> Option<String>,
}

/// The teams as text, with every note of `notes`.
fn render_with_notes(
    teams: &[TeamSummary],
    view: View,
    detail: Detail,
    activity: &dyn Fn(&TaskId) -> Option<String>,
    rounds: &dyn Fn(&Round) -> String,
    notes: Notes<'_>,
    zone: Zone,
) -> String {
    let Notes {
        extras: Extras { under, accounts },
        waits,
        shifts,
    } = notes;
    if teams.is_empty() {
        return NO_TEAMS.to_owned();
    }
    // The lines of the workers share the columns of the tasks: a worker and its step can widen the column of the agents.
    let served: Vec<Vec<workers::Served>> = teams.iter().map(|team| (under.workers)(team)).collect();
    let columns = served
        .iter()
        .flatten()
        .fold(members::Columns::of(teams, accounts), |columns, worker| {
            columns.fit(agent_core::kinds::Role::Worker.to_string().len(), worker.agent_width())
        });
    let mut lines: Vec<String> = Vec::new();
    for (team, served) in teams.iter().zip(&served) {
        let items = &team.items;
        let task = team
            .short_desc
            .as_ref()
            .map_or_else(String::new, |desc| format!("  \"{desc}\""));
        lines.push(format!(
            "{}  {}  {}  {} pending, {} in progress, {} in review, {} approved, {} done{task}",
            team.id,
            team.repo,
            team.merge.as_str(),
            items.pending,
            items.in_progress,
            items.in_review,
            items.approved,
            items.done,
        ));
        lines.extend(
            team.rounds
                .iter()
                .filter(|round| view == View::Full || *round.life() == RoundLife::Open)
                .map(|round| {
                    format!(
                        "  {}{}{}",
                        round_line(round),
                        returned::round_note(round, &team.members),
                        rounds(round)
                    )
                }),
        );
        if detail == Detail::Tasks {
            lines.extend(splits::lines_of(team, view));
            for (plan, _) in splits::shown(team, view) {
                lines.extend(
                    crate::split_context::lines(plan, &team.rounds, &team.part_assignments)
                        .into_iter()
                        .map(|(_, text)| format!("      {text}")),
                );
            }
        }
        for (item, tier) in &team.tier_waits {
            if detail == Detail::Tasks && team.splits.iter().any(|plan| plan.parent() == *item) {
                continue;
            }
            lines.push(format!("    {item}  waiting for {tier} agent"));
        }
        if detail == Detail::Short {
            // The tasks that are alive, each as the full view writes it, without its plan; the count ends the team.
            lines.extend(
                team.members
                    .iter()
                    .filter(|member| members::listed(member))
                    .map(|member| members::line(member, team, columns, accounts, activity, waits, shifts, zone)),
            );
            lines.extend(served.iter().map(|worker| workers::line(worker, columns)));
            lines.push(tasks_line(team));
            continue;
        }
        for member in &team.members {
            lines.push(members::line(member, team, columns, accounts, activity, waits, shifts, zone));
            if let Some(steps) = team.task_plans.get(&member.task) {
                for step in steps {
                    lines.push(render_plan_step(step));
                }
            }
            lines.extend((under.requests)(&member.task));
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// The line `agent status` shows about the history when it is not whole: what it is doing and how many records it lost.
pub(crate) fn render_history(history: Option<&HistoryHealth>) -> String {
    history.map_or_else(String::new, |health| format!("{health}\n"))
}

/// What one render of the status takes from the configuration and the state of the daemon, as [`Activity::reads`] reads
/// them.
pub(crate) struct Reads {
    /// The line counting the runs going per configured CLI, when at least one run is going.
    cli_runs: Option<String>,
    /// The account each agent and investigator of the configuration runs with.
    accounts: members::Accounts,
    /// The lines of the paused accounts, of the agents their provider refused and of the kept limit readings, when there are any.
    provider_lines: Option<String>,
    /// What each queued task whose account has its `max_runs` going says of it, by task.
    account_waits: BTreeMap<TaskId, String>,
    /// What each lead task with shifts says of its worked time, by task.
    shifts: BTreeMap<TaskId, String>,
}

/// What each lead task with shifts says of its worked time, by task.
pub(crate) fn shifts(
    config: &agent_core::config::Config,
    state: &agent_core::state::State,
    now: Timestamp,
    teams: &[TeamSummary],
) -> BTreeMap<TaskId, String> {
    let limit_minutes = u64::from(config.limits().lead_shift_minutes);
    if limit_minutes == 0 {
        return BTreeMap::new();
    }
    teams
        .iter()
        .flat_map(|team| &team.members)
        .filter(|member| matches!(member.role, agent_core::kinds::Role::Solo | agent_core::kinds::Role::Coordinator))
        .filter_map(|member| {
            let found = state.find_task(&member.task)?;
            let worked_secs = agent_core::decide::shift::worked_secs(&found.member, now);
            let worked_minutes = worked_secs / 60;
            Some((
                member.task.clone(),
                format!(
                    "shift: {} of {} worked",
                    agent_core::decide::shift::format_span(worked_minutes),
                    agent_core::decide::shift::format_span(limit_minutes)
                ),
            ))
        })
        .collect()
}

/// The lines of the accounts that are paused at `now`, the line of the agents their provider refused and the lines of
/// the limit readings that have not reset, in `zone`; `None` when there is none of them.
fn provider_lines(state: &agent_core::state::State, now: Timestamp, zone: Zone) -> Option<String> {
    let accounts = accounts::account_lines(state.account_pauses(), now, zone);
    let agents = refusals::refused_line(&state.refusals_at(now), zone);
    let readings = accounts::reading_lines(state.limit_readings(), now, zone);
    match (accounts, agents, readings) {
        (None, None, None) => None,
        (accounts, agents, readings) => Some(format!(
            "{}{}{}",
            accounts.unwrap_or_default(),
            agents.unwrap_or_default(),
            readings.unwrap_or_default()
        )),
    }
}

/// The line that counts the runs going on each CLI configured in `config` across `teams`, in the order of
/// [`agent_core::kinds::CliKind`]; `None` when no run is going or `config` configures no agent.
#[must_use]
pub(crate) fn cli_runs_line(config: &agent_core::config::Config, teams: &[TeamSummary]) -> Option<String> {
    use agent_core::kinds::CliKind;
    use agent_core::state::Participant;

    let cli_of = |participant: &Participant| match participant {
        Participant::Normal(name) | Participant::Worker(name) => config.agent(name).map(|agent| agent.cli),
        Participant::Investigator(name) => config.investigator(name).map(|agent| agent.cli),
    };
    let configured = |kind: CliKind| {
        config.agents().values().any(|agent| agent.cli == kind)
            || config.investigators().values().any(|agent| agent.cli == kind)
    };
    let running: Vec<CliKind> = teams
        .iter()
        .flat_map(|team| &team.members)
        .filter(|member| member.status == TaskStatus::Running)
        .filter_map(|member| cli_of(&member.agent))
        .collect();
    if running.is_empty() {
        return None;
    }
    let parts: Vec<String> = [CliKind::Claude, CliKind::Codex, CliKind::Gemini, CliKind::Opencode]
        .into_iter()
        .filter(|kind| configured(*kind))
        .map(|kind| {
            let count = running.iter().filter(|&&cli| cli == kind).count();
            format!("{kind} {count}")
        })
        .collect();
    (!parts.is_empty()).then(|| format!("runs: {}\n", parts.join(", ")))
}

/// Tallies of a team's tasks by status, in the order the short view prints them: the tasks that act or wait first.
#[derive(Default)]
struct StatusCounts {
    /// Tasks currently in a model run.
    running: usize,
    /// Tasks waiting for a message, assignment or merge.
    waiting: usize,
    /// Tasks waiting for the human's answers.
    questions: usize,
    /// Tasks queued to start their next run.
    queued: usize,
    /// Tasks paused by the human or a quota limit.
    paused: usize,
    /// Tasks stopped by the human.
    stopped: usize,
    /// Tasks whose last run failed.
    failed: usize,
    /// Finished tasks.
    done: usize,
}

impl StatusCounts {
    /// Increments the counter for `status` through an exhaustive match so no [`TaskStatus`] variant can be omitted.
    const fn record(&mut self, status: TaskStatus) {
        let slot = match status {
            TaskStatus::Running => &mut self.running,
            TaskStatus::Waiting => &mut self.waiting,
            TaskStatus::Questions => &mut self.questions,
            TaskStatus::Queued => &mut self.queued,
            TaskStatus::Paused => &mut self.paused,
            TaskStatus::Stopped => &mut self.stopped,
            TaskStatus::Failed => &mut self.failed,
            TaskStatus::Done => &mut self.done,
        };
        *slot = slot.saturating_add(1);
    }

    /// The non-zero counts in display order, each paired with its status label.
    fn non_zero(&self) -> Vec<String> {
        [
            (self.running, count_word(TaskStatus::Running)),
            (self.waiting, count_word(TaskStatus::Waiting)),
            (self.questions, count_word(TaskStatus::Questions)),
            (self.queued, count_word(TaskStatus::Queued)),
            (self.paused, count_word(TaskStatus::Paused)),
            (self.stopped, count_word(TaskStatus::Stopped)),
            (self.failed, count_word(TaskStatus::Failed)),
            (self.done, count_word(TaskStatus::Done)),
        ]
        .into_iter()
        .filter(|(count, _)| *count > 0)
        .map(|(count, word)| format!("{count} {word}"))
        .collect()
    }
}

/// The line that counts the tasks of `team` by what they do, in the order of [`StatusCounts`], and names each question
/// that waits for the human's answer with its number, the agent and its task: what the human must act on.
fn tasks_line(team: &TeamSummary) -> String {
    let mut tally = StatusCounts::default();
    for member in &team.members {
        tally.record(member.status);
    }
    let counts = tally.non_zero();
    let mut text = if counts.is_empty() {
        String::from("  tasks: none")
    } else {
        format!("  tasks: {}", counts.join(", "))
    };
    let questions: Vec<String> = team
        .members
        .iter()
        .filter_map(|member| {
            member.open_question.map(|number| {
                format!(
                    "questions number {number} wait for answers ({}, {})",
                    member.agent.name(),
                    member.task
                )
            })
        })
        .collect();
    if !questions.is_empty() {
        text.push_str("; ");
        text.push_str(&questions.join("; "));
    }
    text
}

/// The word a count of tasks writes for a status.
const fn count_word(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Running => "running",
        TaskStatus::Waiting => "waiting",
        TaskStatus::Questions => "with questions",
        TaskStatus::Queued => "queued",
        TaskStatus::Paused => "paused",
        TaskStatus::Stopped => "stopped",
        TaskStatus::Failed => "failed",
        TaskStatus::Done => "done",
    }
}

/// A round as one line: its ID, its item and whether it is open or how it ended.
pub(crate) fn round_line(round: &Round) -> String {
    let item = round
        .item()
        .map_or_else(|| "whole task list".to_owned(), |item| format!("item {item}"));
    let state = match (round.delivered_by(), round.life()) {
        (Some(pr), _) if round.delivery_closed_at().is_some() => {
            format!("closed, abandoned, delivery PR #{} closed without merging", pr.number)
        }
        (Some(pr), RoundLife::Open) => format!("delivered by PR #{}, waits for its merge", pr.number),
        (
            Some(pr),
            RoundLife::Closed {
                how: agent_core::state::RoundEnd::Merged,
                ..
            },
        ) => format!("done, delivered by PR #{}", pr.number),
        (
            Some(pr),
            RoundLife::Closed {
                how: agent_core::state::RoundEnd::Abandoned,
                ..
            },
        ) => format!("closed, abandoned, delivered by PR #{}", pr.number),
        (Some(_) | None, life) => match life {
            RoundLife::Open => "open".to_owned(),
            RoundLife::Closed { how, .. } => format!("closed, {how}"),
        },
    };
    format!("{}  {item}  {state}", round.id())
}

/// A round in full as text, its times in UTC: what the tests of the layout compare.
#[cfg(test)]
pub(crate) fn render_round(detail: &RoundDetail) -> String {
    render_round_in(detail, Zone::Fixed(UtcOffset::UTC))
}

/// A round in full as text: its line, the item, the pull request, the review passes, and the tasks that worked for it
/// with where they work and their runs, its times in `zone`.
#[cfg(test)]
pub(crate) fn render_round_in(detail: &RoundDetail, zone: Zone) -> String {
    render_round_with_accounts(detail, zone, &BTreeMap::new())
}

/// `render_round_in` with the account each agent runs with: an agent that has one is followed by it in brackets.
pub(crate) fn render_round_with_accounts(detail: &RoundDetail, zone: Zone, accounts: &members::Accounts) -> String {
    let waiting = detail.waiting.as_deref().map_or_else(String::new, |waiting| {
        format!(", waits for {}", zone.stamp_waiting(waiting))
    });
    let mut lines = vec![format!("{}  team {}{waiting}", round_line(&detail.round), detail.team)];
    lines.push(format!("  opened {}", zone.stamp(detail.round.opened_at())));
    if let RoundLife::Closed { at, how } = detail.round.life() {
        lines.push(format!("  closed {}, {how}", zone.stamp(*at)));
    }
    if let Some(item) = &detail.item {
        lines.push(format!("  item {}: {} ({})", item.id, item.title, item.status));
    }
    if let Some(pr) = &detail.pr {
        lines.push(format!("  pull request #{} on {}", pr.number, pr.branch));
    }
    if !detail.findings.is_empty() {
        let count = |severity| {
            detail
                .findings
                .iter()
                .filter(|entry| {
                    entry.status == agent_core::state::FindingStatus::Open && entry.finding.severity == severity
                })
                .count()
        };
        lines.push(format!(
            "  open findings: {} BLOCKER, {} MAJOR, {} MINOR",
            count(Severity::Blocker),
            count(Severity::Major),
            count(Severity::Minor)
        ));
    }
    for (index, pass) in detail.review_passes.iter().enumerate() {
        let reviews: Vec<String> = pass
            .reviews
            .iter()
            .map(|review| match &review.outcome {
                agent_core::state::ReviewOutcome::Pending => format!("{} pending", review.reviewer),
                agent_core::state::ReviewOutcome::Submitted { verdict, .. } => {
                    format!("{} {verdict}", review.reviewer)
                }
            })
            .collect();
        lines.push(format!(
            "  review pass {}: {}",
            index.saturating_add(1),
            reviews.join(", ")
        ));
    }
    for task in &detail.tasks {
        lines.push(format!(
            "  {}  {}  {}  {}  {}",
            task.task,
            task.role,
            agent_core::account_view::labelled(task.agent.name(), accounts.get(&task.agent.agent_name())),
            task.worktree.as_path().display(),
            task.branch
        ));
        for run in &task.runs {
            let state = match run.state() {
                RunState::Running => "running".to_owned(),
                RunState::Ended { at, .. } => format!("ended {}", zone.stamp(*at)),
            };
            lines.push(format!(
                "    {}  started {}  {state}",
                run.id(),
                zone.stamp(run.started_at())
            ));
        }
        if let Some(steps) = detail.task_plans.get(&task.task) {
            for step in steps {
                lines.push(render_plan_step(step));
            }
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// A step of a plan as a line of a task: its mark, its ID, its title, and its settled state when judged.
pub(crate) fn render_plan_step(step: &PlanStepSummary) -> String {
    let state_suffix = match step.state {
        Some(JudgementState::Confirmed) => " (confirmed)".to_owned(),
        Some(JudgementState::Declared) => " (declared)".to_owned(),
        Some(JudgementState::Mismatch) => match &step.reason {
            Some(reason) => format!(" (mismatch: {reason})"),
            None => " (mismatch)".to_owned(),
        },
        None => String::new(),
    };
    format!("    [{}] {} {}{}", step.mark.mark(), step.id, step.title, state_suffix)
}

/// What is written when the daemon knows no team.
const NO_TEAMS: &str = "no teams\n";

/// What `agent status --running` writes when no round goes on.
const NO_OPEN_ROUNDS: &str = "no team has an open round\n";

mod accounts;
mod investigators;
mod members;
pub(crate) mod pool;
mod refusals;
pub(crate) mod requests;
mod returned;
mod reviewer;
mod rounds;
mod snapshots;
mod splits;
pub(crate) mod versions;
mod workers;

#[cfg(test)]
#[path = "status/history/tests.rs"]
mod history_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "status/tests/splits.rs"]
mod split_tests;

#[cfg(test)]
#[path = "status/tests/negotiation.rs"]
mod negotiation_tests;

#[cfg(test)]
#[path = "status/tests/short_desc.rs"]
mod short_desc_tests;

#[cfg(test)]
#[path = "status/tests/part_states.rs"]
mod part_state_tests;

#[cfg(test)]
#[path = "status/tests/returned.rs"]
mod returned_tests;

#[cfg(test)]
#[path = "status/tests/short.rs"]
mod short_tests;

#[cfg(test)]
#[path = "status/tests/short_members.rs"]
mod short_members_tests;

#[cfg(test)]
#[path = "status/tests/versions.rs"]
mod versions_tests;

#[cfg(test)]
#[path = "status/tests/account_waits.rs"]
mod account_waits_tests;

#[cfg(test)]
#[path = "status/tests/split_tier.rs"]
mod split_tier_tests;

#[cfg(test)]
#[path = "status/tests/account_labels.rs"]
mod account_label_tests;

#[cfg(test)]
#[path = "status/tests/account_status.rs"]
mod account_status_tests;

#[cfg(test)]
#[path = "status/tests/reading_status.rs"]
mod reading_status_tests;
