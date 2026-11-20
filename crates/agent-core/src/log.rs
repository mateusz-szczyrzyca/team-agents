//! The daemon's own structured log: its levels ([`LogLevel`]), its fixed event names ([`LogEvent`]), and the records it
//! writes to `history.db` ([`LogRecord`], [`LogEntry`]); see `docs/log.md`.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::num::NonZeroU64;

use serde::{Deserialize, Serialize};

use crate::history::{Cut, MaxTextBytes, UnknownCutField};
use crate::ids::{RoundId, RunId, TaskId, TeamId};
use crate::kinds::{Keyword, UnknownKeyword};
use crate::state::Timestamp;

/// The fields of a [`LogEntry`] that [`MaxTextBytes`] may cut.
pub const KNOWN_LOG_CUT_FIELDS: &[&str] = &["fields", "message"];

/// The severity of a daemon log event, in order from highest (`Error`) to lowest (`Debug`).
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantArray,
    strum::VariantNames,
)]
#[serde(try_from = "String", into = "&'static str")]
#[strum(
    serialize_all = "snake_case",
    parse_err_ty = UnknownKeyword,
    parse_err_fn = UnknownKeyword::of::<LogLevel>,
)]
pub enum LogLevel {
    /// Something broke or could not be done.
    Error,
    /// A failure the daemon recovers from or retries.
    Warn,
    /// Every change of a team, round, item, task, or run.
    #[default]
    Info,
    /// Every decision that takes action and every outside call.
    Debug,
}

impl Keyword for LogLevel {
    const WHAT: &'static str = "log level";
}

impl LogLevel {
    /// Every [`LogLevel`] in severity order.
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    /// The keyword of this level (`error`, `warn`, `info`, `debug`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// Whether a configured threshold of `self` keeps an event of `event_level`.
    #[must_use]
    pub const fn keeps(self, event_level: Self) -> bool {
        (event_level as u8) <= (self as u8)
    }
}

impl TryFrom<String> for LogLevel {
    type Error = UnknownKeyword;

    fn try_from(text: String) -> Result<Self, UnknownKeyword> {
        text.parse()
    }
}

/// Every event name the daemon logs (`docs/log.md`).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantArray,
    strum::VariantNames,
)]
#[serde(try_from = "String", into = "&'static str")]
#[strum(
    serialize_all = "snake_case",
    parse_err_ty = UnknownKeyword,
    parse_err_fn = UnknownKeyword::of::<LogEvent>,
)]
pub enum LogEvent {
    /// A line of a model CLI's output cannot be read as its event format.
    CliOutputUnreadable,
    /// A model CLI run cannot start, ends without reporting a status or a reviewer's verdict, or fails.
    RunEndedWithoutStatus,
    /// The hang watchdog stops a silent run.
    RunHung,
    /// A run stops because its model quota is exhausted.
    RunQuotaExhausted,
    /// A run stops because of a rate limit.
    RunRateLimited,
    /// An `agentbus` operation fails and what waits for it is retried or dropped.
    BusDown,
    /// A `git` call, worktree operation or branch operation fails or is retried.
    GitCallFailed,
    /// A GitHub API call, a publication or a merge fails or is retried, or the home has no GitHub token.
    GithubCallFailed,
    /// A gate command cannot start or be recorded.
    GateCannotStart,
    /// A gate command exceeds its timeout and is killed.
    GateTimedOut,
    /// A gate runs on or leaves a dirty worktree or moves `HEAD`.
    GateDirtyTree,
    /// The daemon's state file or the activity log of a task cannot be saved.
    StateSaveFailed,
    /// A questions file is missing or not valid, or cannot be prepared, watched or opened.
    QuestionsFileInvalid,
    /// An agent, investigator, or repository named in the state or a request is no longer in `agents.toml`.
    ConfigurationNameMissing,
    /// A start or restart of the daemon found an interrupted run or a host fallback and corrected it.
    RestartCorrected,
    /// A snapshot of a task's worktree cannot be taken; the daemon goes on without it.
    SnapshotFailed,
    /// A snapshot of a task's worktree is bigger than `[snapshot] warn_mb`; it is kept, and its size is logged.
    SnapshotLarge,
    /// A client connection, subscription or request cannot be accepted or is refused.
    RequestRefused,
    /// A task or thread in the daemon panicked.
    DaemonPanicked,
    /// A task of the daemon (a side task or the task of a client connection) ends with an error it does not handle.
    DaemonTaskFailed,
    /// A team that was formed is not started because of a fault of the daemon, not of the request.
    TeamNotStarted,
    /// A notification for the human cannot be made, timed, queued or delivered.
    NotificationFailed,
    /// A record of the history cannot be made as it should: it is left out, counted as lost or made with a fallback.
    HistoryRecordIncomplete,
    /// A team is started or closed.
    TeamChanged,
    /// A round is opened or closed.
    RoundChanged,
    /// An item's phase changes.
    ItemPhaseChanged,
    /// A task's status changes.
    TaskStatusChanged,
    /// A model CLI run starts.
    RunStarted,
    /// A model CLI run ends.
    RunEnded,
    /// The daemon has opened its state and its history and serves its socket.
    DaemonStarted,
    /// A signal (`SIGTERM` or `SIGINT`) asks the daemon to stop.
    DaemonStopRequested,
    /// Everything of the daemon but its history has ended: the last record of a daemon that ended cleanly.
    DaemonStopped,
    /// A request for a worker is given to a worker.
    RequestDelegated,
    /// The open questions of a task are answered, with the channel of the answer.
    QuestionsAnswered,
    /// The human used a control (stop, pause, unpause, resume) and it reached its tasks.
    ControlUsed,
    /// The checks before a review send a round back to its team, ask the human, or take the human's answer.
    ChecksDecided,
    /// The reviews of a pull request are asked for.
    ReviewsRequested,
    /// A reviewer's report comes when no review waits for it: it is kept or left out, and changes nothing.
    ReviewReportLate,
    /// A notification for the human is delivered.
    NotificationSent,
    /// A new questions file is opened in the editor, or is not because the editor is not there.
    QuestionsFileOpened,
    /// A build directory was removed to release disk space.
    BuildDirectoryRemoved,
    /// A pure decision of `agent_core::decide` takes action, with its input and result.
    DecisionTaken,
    /// An outside call (`git`, `agentbus`, GitHub API, model CLI) completes, with its duration and outcome.
    OutsideCallCompleted,
    /// The other end of an answer (a client of the socket, or a task of the daemon) was gone before the answer.
    PeerGone,
    /// A step of the owner that calls GitHub or git failed and waits to be tried again.
    GithubStepWaiting,
    /// A step of the owner that waited for GitHub or git goes through.
    GithubStepResumed,
}

impl Keyword for LogEvent {
    const WHAT: &'static str = "log event";
}

impl LogEvent {
    /// Every [`LogEvent`] in declaration order.
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    /// The `snake_case` name of this event.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        self.into()
    }

    /// The severity level of this event.
    #[must_use]
    pub const fn level(self) -> LogLevel {
        match self {
            Self::StateSaveFailed
            | Self::ConfigurationNameMissing
            | Self::DaemonPanicked
            | Self::DaemonTaskFailed
            | Self::TeamNotStarted
            | Self::NotificationFailed => LogLevel::Error,
            Self::CliOutputUnreadable
            | Self::RunEndedWithoutStatus
            | Self::RunHung
            | Self::RunQuotaExhausted
            | Self::RunRateLimited
            | Self::BusDown
            | Self::GitCallFailed
            | Self::GithubCallFailed
            | Self::GithubStepWaiting
            | Self::GateCannotStart
            | Self::GateTimedOut
            | Self::GateDirtyTree
            | Self::QuestionsFileInvalid
            | Self::RestartCorrected
            | Self::SnapshotFailed
            | Self::SnapshotLarge
            | Self::RequestRefused
            | Self::HistoryRecordIncomplete => LogLevel::Warn,
            Self::TeamChanged
            | Self::RoundChanged
            | Self::ItemPhaseChanged
            | Self::TaskStatusChanged
            | Self::RunStarted
            | Self::RunEnded
            | Self::DaemonStarted
            | Self::DaemonStopRequested
            | Self::DaemonStopped
            | Self::RequestDelegated
            | Self::QuestionsAnswered
            | Self::ControlUsed
            | Self::ChecksDecided
            | Self::GithubStepResumed
            | Self::ReviewsRequested
            | Self::ReviewReportLate
            | Self::NotificationSent
            | Self::QuestionsFileOpened
            | Self::BuildDirectoryRemoved => LogLevel::Info,
            Self::DecisionTaken | Self::OutsideCallCompleted | Self::PeerGone => LogLevel::Debug,
        }
    }
}

impl TryFrom<String> for LogEvent {
    type Error = UnknownKeyword;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

/// The team and open rounds a daemon log entry belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogTeamScope {
    /// The team.
    team: TeamId,
    /// The open rounds of the team in strictly ascending order, without duplicates.
    rounds: Vec<RoundId>,
}

impl LogTeamScope {
    /// Builds a team scope, sorting and deduplicating `rounds`.
    #[must_use]
    pub fn new(team: TeamId, rounds: impl IntoIterator<Item = RoundId>) -> Self {
        let mut rounds: Vec<RoundId> = rounds.into_iter().collect();
        rounds.sort();
        rounds.dedup();
        Self { team, rounds }
    }

    /// Builds a team scope from `rounds` that must already be strictly ascending.
    ///
    /// # Errors
    /// [`InvalidLogEntry::UnsortedRounds`] if `rounds` is not strictly ascending.
    pub fn from_sorted(team: TeamId, rounds: Vec<RoundId>) -> Result<Self, InvalidLogEntry> {
        if !rounds.windows(2).all(|pair| {
            matches!(
                (pair.first(), pair.get(1)),
                (Some(left), Some(right)) if left < right
            )
        }) {
            return Err(InvalidLogEntry::UnsortedRounds);
        }
        Ok(Self { team, rounds })
    }

    getter!("The team.", pub const fn team(&self) -> &TeamId = &self.team);
    getter!("The open rounds of the team in strictly ascending order.", pub fn rounds(&self) -> &[RoundId] = &self.rounds);
}

/// The task and optional run a daemon log entry belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogTaskScope {
    /// The task.
    task: TaskId,
    /// The run of the task, when the entry occurred inside a run.
    run: Option<RunId>,
}

impl LogTaskScope {
    /// Builds a task scope for `task` and optional `run`.
    #[must_use]
    pub const fn new(task: TaskId, run: Option<RunId>) -> Self {
        Self { task, run }
    }

    getter!("The task.", pub const fn task(&self) -> &TaskId = &self.task);
    getter!("The run of the task, if any.", pub const fn run(&self) -> Option<&RunId> = self.run.as_ref());
}

/// One daemon log event as captured by the tracing layer before it is written to `history.db`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogRecord {
    /// When the event occurred.
    pub at: Timestamp,
    /// The Rust module path (`target`) that emitted the event.
    pub module: String,
    /// The fixed event kind.
    pub event: LogEvent,
    /// The team and open rounds from enclosing spans, if any.
    pub team: Option<LogTeamScope>,
    /// The task and run from enclosing spans, if any.
    pub task: Option<LogTaskScope>,
    /// The human-readable message of the event.
    pub message: String,
    /// The structured fields of the event, sorted by key.
    pub fields: BTreeMap<String, serde_json::Value>,
    /// Which of `message` or `fields` were cut at [`MaxTextBytes`].
    pub cut: Cut,
}

impl LogRecord {
    /// The severity level of this event.
    #[must_use]
    pub const fn level(&self) -> LogLevel {
        self.event.level()
    }

    /// The team of this record, if any.
    #[must_use]
    pub fn team_id(&self) -> Option<&TeamId> {
        self.team.as_ref().map(LogTeamScope::team)
    }

    /// The open rounds of this record's team, or empty when not in a round.
    #[must_use]
    pub fn rounds(&self) -> &[RoundId] {
        self.team.as_ref().map_or(&[], LogTeamScope::rounds)
    }

    /// The task of this record, if any.
    #[must_use]
    pub fn task_id(&self) -> Option<&TaskId> {
        self.task.as_ref().map(LogTaskScope::task)
    }

    /// The run of this record, if any.
    #[must_use]
    pub fn run_id(&self) -> Option<&RunId> {
        self.task.as_ref().and_then(LogTaskScope::run)
    }

    /// Cuts `message` and `fields` to fit within `limit` and marks any truncated field in `self.cut`.
    #[must_use]
    pub fn keep(mut self, limit: MaxTextBytes) -> Self {
        self.message = self.cut.keep("message", &self.message, limit);
        let max = usize::try_from(limit.get()).unwrap_or(usize::MAX);
        let serialized_len = serde_json::to_string(&self.fields).map_or(usize::MAX, |text| text.len());
        if serialized_len > max {
            let mut replacement = BTreeMap::new();
            replacement.insert("truncated".to_owned(), serde_json::Value::Bool(true));
            self.fields = replacement;
            self.cut.mark("fields");
        }
        self
    }

    /// Whether `self` and `other` are the same warning in the same scope and may be coalesced into one row.
    #[must_use]
    pub fn same_warning(&self, other: &Self) -> bool {
        self.level() == LogLevel::Warn
            && other.level() == LogLevel::Warn
            && self.event == other.event
            && self.team == other.team
            && self.task == other.task
            && self.message == other.message
            && self.fields == other.fields
    }
}

/// Why a stored row of `daemon_logs` / `daemon_log_rounds` is not a valid [`LogEntry`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidLogEntry {
    /// The stored `level` column does not match [`LogEvent::level`].
    #[error("log event {event} has level {}, not {level}", event.level())]
    LevelMismatch {
        /// The stored event.
        event: LogEvent,
        /// The stored level that disagrees with `event`.
        level: LogLevel,
    },
    /// A non-warning row has `count > 1`.
    #[error("log event {event} has count {count}: only warn entries may repeat")]
    NonWarnRepeated {
        /// The stored event.
        event: LogEvent,
        /// The stored count.
        count: NonZeroU64,
    },
    /// `first_at` is later than `at`.
    #[error("log entry has first_at {first_at} after at {at}")]
    FirstAfterLast {
        /// When the entry first occurred.
        first_at: Timestamp,
        /// When the entry last occurred.
        at: Timestamp,
    },
    /// A `run` number is present without a `task`.
    #[error("log entry has run {run} without a task")]
    RunWithoutTask {
        /// The run number without a task.
        run: RunId,
    },
    /// One or more `rounds` are present without a `team`.
    #[error("log entry has rounds without a team")]
    RoundsWithoutTeam,
    /// The rounds linked to the entry are not strictly ascending.
    #[error("log entry rounds are not strictly ascending")]
    UnsortedRounds,
    /// The `cut` column names a field that log entries never cut.
    #[error(transparent)]
    Cut(#[from] UnknownCutField),
}

/// The raw columns of a stored `daemon_logs` row and its `daemon_log_rounds` rows, before validation into [`LogEntry`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntryParts {
    /// The row's primary key (`id`).
    pub id: i64,
    /// When the entry was first recorded.
    pub first_at: Timestamp,
    /// When the entry was most recently recorded.
    pub at: Timestamp,
    /// The stored `level` column.
    pub level: LogLevel,
    /// The stored `module` column.
    pub module: String,
    /// The stored `event` column.
    pub event: LogEvent,
    /// The stored `team` column, if any.
    pub team: Option<TeamId>,
    /// The rounds linked in `daemon_log_rounds`, in row order.
    pub rounds: Vec<RoundId>,
    /// The stored `task` column, if any.
    pub task: Option<TaskId>,
    /// The stored `run` column, if any.
    pub run: Option<RunId>,
    /// The stored `message` column.
    pub message: String,
    /// The parsed JSON object from the `fields` column.
    pub fields: BTreeMap<String, serde_json::Value>,
    /// The stored repetition `count`.
    pub count: NonZeroU64,
    /// The raw `cut` column text.
    pub cut: String,
}

/// A validated row of `daemon_logs` and its `daemon_log_rounds` links read from `history.db`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    /// The row's primary key (`id`).
    id: i64,
    /// When the entry was first recorded.
    first_at: Timestamp,
    /// When the entry was most recently recorded.
    at: Timestamp,
    /// The Rust module path that emitted the event.
    module: String,
    /// The fixed event kind.
    event: LogEvent,
    /// The team and open rounds, if any.
    team: Option<LogTeamScope>,
    /// The task and run, if any.
    task: Option<LogTaskScope>,
    /// The human-readable message.
    message: String,
    /// The structured fields of the event.
    fields: BTreeMap<String, serde_json::Value>,
    /// How many times this entry was recorded (`1` for non-warnings, `>= 1` for coalesced warnings).
    count: NonZeroU64,
    /// Which of `message` or `fields` were cut.
    cut: Cut,
}

impl LogEntry {
    /// Validates `parts` into a [`LogEntry`].
    ///
    /// # Errors
    /// [`InvalidLogEntry`] if any invariant of `daemon_logs` or `daemon_log_rounds` is violated.
    pub fn try_new(parts: LogEntryParts) -> Result<Self, InvalidLogEntry> {
        let LogEntryParts {
            id,
            first_at,
            at,
            level,
            module,
            event,
            team,
            rounds,
            task,
            run,
            message,
            fields,
            count,
            cut,
        } = parts;
        if level != event.level() {
            return Err(InvalidLogEntry::LevelMismatch { event, level });
        }
        if count.get() > 1 && level != LogLevel::Warn {
            return Err(InvalidLogEntry::NonWarnRepeated { event, count });
        }
        if first_at > at {
            return Err(InvalidLogEntry::FirstAfterLast { first_at, at });
        }
        let team = match (team, rounds.is_empty()) {
            (Some(team), _) => Some(LogTeamScope::from_sorted(team, rounds)?),
            (None, true) => None,
            (None, false) => return Err(InvalidLogEntry::RoundsWithoutTeam),
        };
        let task = match (task, run) {
            (Some(task), run) => Some(LogTaskScope::new(task, run)),
            (None, None) => None,
            (None, Some(run)) => return Err(InvalidLogEntry::RunWithoutTask { run }),
        };
        let cut = Cut::parse(&cut, KNOWN_LOG_CUT_FIELDS)?;
        Ok(Self {
            id,
            first_at,
            at,
            module,
            event,
            team,
            task,
            message,
            fields,
            count,
            cut,
        })
    }

    getter!("The row's primary key (`id`).", pub const fn id(&self) -> i64 = self.id);
    getter!("When the entry was first recorded.", pub const fn first_at(&self) -> Timestamp = self.first_at);
    getter!("When the entry was most recently recorded.", pub const fn at(&self) -> Timestamp = self.at);
    getter!("The severity level of this entry.", pub const fn level(&self) -> LogLevel = self.event.level());
    getter!("The Rust module path that emitted the event.", pub fn module(&self) -> &str = &self.module);
    getter!("The fixed event kind.", pub const fn event(&self) -> LogEvent = self.event);
    getter!("The team and open rounds, if any.", pub const fn team_scope(&self) -> Option<&LogTeamScope> = self.team.as_ref());
    getter!("The team, if any.", pub fn team(&self) -> Option<&TeamId> = self.team.as_ref().map(LogTeamScope::team));
    getter!("The open rounds of the team, or empty when not in a round.", pub fn rounds(&self) -> &[RoundId] = self.team.as_ref().map_or(&[], LogTeamScope::rounds));
    getter!("The task and run, if any.", pub const fn task_scope(&self) -> Option<&LogTaskScope> = self.task.as_ref());
    getter!("The task, if any.", pub fn task(&self) -> Option<&TaskId> = self.task.as_ref().map(LogTaskScope::task));
    getter!("The run, if any.", pub fn run(&self) -> Option<&RunId> = self.task.as_ref().and_then(LogTaskScope::run));
    getter!("The human-readable message.", pub fn message(&self) -> &str = &self.message);
    getter!("The structured fields of the event.", pub const fn fields(&self) -> &BTreeMap<String, serde_json::Value> = &self.fields);
    getter!("How many times this entry was recorded.", pub const fn count(&self) -> NonZeroU64 = self.count);
    getter!("Which of `message` or `fields` were cut.", pub const fn cut(&self) -> &Cut = &self.cut);
}

/// The single subject printed by a scoped history query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumString, strum::VariantNames)]
#[strum(parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<LogView>)]
pub enum LogView {
    /// Tasks with the latest recorded run status.
    #[strum(serialize = "tasks")]
    Tasks,
    /// Participants and how they were chosen.
    #[strum(serialize = "agents")]
    Agents,
    /// Worktree paths, branches and recorded heads.
    #[strum(serialize = "worktrees")]
    Worktrees,
    /// Run starts.
    #[strum(serialize = "runs")]
    Runs,
    /// Whole recorded prompts.
    #[strum(serialize = "prompts")]
    Prompts,
    /// Recorded changes.
    #[strum(serialize = "changes")]
    Changes,
    /// Review requests and reports.
    #[strum(serialize = "reviews")]
    Reviews,
    /// Availability of token accounting.
    #[strum(serialize = "usage")]
    Usage,
    /// Daemon log rows.
    #[strum(serialize = "log")]
    Log,
}

impl Keyword for LogView {
    const WHAT: &'static str = "history view";
}
