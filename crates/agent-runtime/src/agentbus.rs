//! `agentbus` as a library: the commands the daemon uses, with typed arguments, typed results and typed errors.
//!
//! `agentbus` (Go, `agentbus/`) owns the shared bus: the teams, their board, the queue of requests for workers, the
//! checkpoints and the messages, all in one directory under a lock. The daemon does not read those files; it runs
//! `agentbus` subcommands — one process per call, with an argument vector and never a shell — and reads what they
//! print. An exit status says why a command failed (see [`FailureClass`]), and the daemon acts on the class, not on
//! the message.
//!
//! One [`AgentBus`] is one bus: the `AGENTS_HOME` it runs under, whose `bus/` directory holds the state. Nothing
//! else of the environment decides where the bus is.
//!
//! What a command prints is a format of Go code. The readers here accept exactly what the commands of this
//! repository print; an output they do not recognise is [`AgentBusError::Unexpected`], never a guess.

use std::collections::BTreeMap;
use std::fmt;
use std::num::{NonZeroU32, NonZeroU64};
use std::path::PathBuf;
use std::str::FromStr;
use std::string::FromUtf8Error;
use std::time::Duration;

use agent_core::config::{AbsolutePath, Program, RepoName};
use agent_core::ids::{AgentName, BranchName, TaskId, TeamId};
use agent_core::kinds::{Keyword, UnknownKeyword};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

use crate::process::{CaptureError, Captured};

/// How long one command may run before its process group is killed. A command waits for the bus lock, which others
/// hold for the length of one operation.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// How many bytes of the output of a command are read; a team with a long board is far below it.
const MAX_OUTPUT_BYTES: u64 = 8 << 20;

/// How many characters of what a command printed an error keeps.
const MAX_DIAGNOSTIC_CHARS: usize = 2048;

/// The first exit status `agentbus` uses for a class of failure; the classes follow in the order of
/// [`FailureClass`] up to [`LAST_CLASS_EXIT_CODE`].
const FIRST_CLASS_EXIT_CODE: i32 = 2;

/// The last exit status that is a class of failure.
const LAST_CLASS_EXIT_CODE: i32 = 10;

/// Which `agentbus` subcommand was run, for an error. The text is the subcommand's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "kebab-case")]
pub enum BusCommand {
    /// `team-create`.
    TeamCreate,
    /// `team-get`.
    TeamGet,
    /// `team-set`.
    TeamSet,
    /// `team-done`.
    TeamDone,
    /// `board-add`.
    BoardAdd,
    /// `board-dump`.
    BoardDump,
    /// `board-claim`.
    BoardClaim,
    /// `board-release-agent`.
    BoardReleaseAgent,
    /// `delegate`.
    Delegate,
    /// `helper-next`.
    HelperNext,
    /// `helper-set`.
    HelperSet,
    /// `helper-get`.
    HelperGet,
    /// `checkpoint-get`.
    CheckpointGet,
    /// `send`.
    Send,
    /// `status`.
    Status,
}

/// Why `agentbus` failed, by its exit status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub enum FailureClass {
    /// 2: an argument is missing or has an invalid value.
    InvalidArgument,
    /// 3: the team, member, board item or request does not exist.
    NotFound,
    /// 4: the operation is for a member of a team and the caller is in none.
    NotInTeam,
    /// 5: the caller's role or tier does not allow it.
    Permission,
    /// 6: it was done already, or the object is taken by someone else.
    Conflict,
    /// 7: the object exists but its state does not allow the operation.
    InvalidState,
    /// 8: reading or writing the bus directory failed.
    Storage,
    /// 9: a tool that does not exist.
    UnknownTool,
    /// 10: a process `agentbus` started failed.
    Process,
    /// Any other status, `1` included: not a class `agentbus` gives.
    Unclassified,
}

impl FailureClass {
    /// The class of the exit status `code`.
    #[must_use]
    pub fn from_exit_code(code: i32) -> Self {
        let _ = (code, FIRST_CLASS_EXIT_CODE, LAST_CLASS_EXIT_CODE);
        Self::Unclassified
    }
}

/// Why a command of `agentbus` failed.
#[derive(Debug, thiserror::Error)]
pub enum AgentBusError {
    /// The process could not be started, could not be read or waited for, or ran over its time and was killed.
    #[error("cannot run agentbus {command}")]
    Run {
        /// The command.
        command: BusCommand,
        /// What went wrong; [`CaptureError::Timeout`] for a command over its time.
        source: CaptureError,
    },
    /// The command ended with a failure status.
    #[error("agentbus {command} failed with status {exit_code} ({class}): {message}")]
    Failed {
        /// The command.
        command: BusCommand,
        /// The exit status.
        exit_code: i32,
        /// What the status means.
        class: FailureClass,
        /// What `agentbus` said on its standard error, bounded.
        message: String,
    },
    /// A signal ended the process.
    #[error("agentbus {command} was ended by the signal {signal}")]
    Killed {
        /// The command.
        command: BusCommand,
        /// The signal number.
        signal: i32,
    },
    /// The command printed more than the limit.
    #[error("agentbus {command} printed more than {limit} bytes")]
    OutputTooLong {
        /// The command.
        command: BusCommand,
        /// The limit, in bytes.
        limit: u64,
    },
    /// The output is not UTF-8.
    #[error("agentbus {command} printed text that is not UTF-8")]
    NotUtf8 {
        /// The command.
        command: BusCommand,
        /// Where the text goes wrong.
        source: FromUtf8Error,
    },
    /// The output is not the JSON that was expected.
    #[error("agentbus {command} printed JSON that is not what was expected")]
    Decode {
        /// The command.
        command: BusCommand,
        /// What the decoder said.
        source: serde_json::Error,
    },
    /// The output is not in the format of the command.
    #[error("agentbus {command} printed something unexpected: {output:?}")]
    Unexpected {
        /// The command.
        command: BusCommand,
        /// What it printed, bounded.
        output: String,
    },
}

impl AgentBusError {
    /// The class of the failure, when the command ended with a failure status.
    #[must_use]
    pub fn class(&self) -> Option<FailureClass> {
        None
    }
}

/// The model tier of a team member on the bus.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantNames,
)]
#[serde(try_from = "String", into = "&'static str")]
#[strum(serialize_all = "snake_case", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<BusTier>)]
pub enum BusTier {
    /// Does the real work, may coordinate and delegate.
    Normal,
    /// A cheap model: easy board items and delegated requests only.
    Worker,
}

impl Keyword for BusTier {
    const WHAT: &'static str = "bus tier";
}

impl TryFrom<String> for BusTier {
    type Error = UnknownKeyword;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

/// The state of a team member on the bus.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantNames,
)]
#[serde(try_from = "String", into = "&'static str")]
#[strum(serialize_all = "snake_case", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<MemberState>)]
pub enum MemberState {
    /// Its run is in progress.
    Active,
    /// It waits for messages or board items: no run, no tokens.
    Sleeping,
    /// A wake-up was triggered and its run is starting.
    Waking,
    /// Finished.
    Done,
    /// Out of the team.
    Excluded,
    /// Marked hung by the watchdog.
    Hung,
    /// Out of quota or stopped by the wake-loop guard: messages do not wake it.
    Paused,
    /// Stopped by the human.
    Stopped,
}

impl Keyword for MemberState {
    const WHAT: &'static str = "member state";
}

impl TryFrom<String> for MemberState {
    type Error = UnknownKeyword;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

/// Who should take a board item.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantNames,
)]
#[serde(try_from = "String", into = "&'static str")]
#[strum(serialize_all = "snake_case", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<BoardLevel>)]
pub enum BoardLevel {
    /// Simple, mechanical work, for workers.
    Easy,
    /// Work that needs reasoning, for normal models.
    Normal,
}

impl Keyword for BoardLevel {
    const WHAT: &'static str = "board level";
}

impl TryFrom<String> for BoardLevel {
    type Error = UnknownKeyword;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

/// Where a board item is in its life.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantNames,
)]
#[serde(try_from = "String", into = "&'static str")]
#[strum(serialize_all = "snake_case", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<BoardStatus>)]
pub enum BoardStatus {
    /// Waiting for someone to claim it.
    Todo,
    /// Someone is working on it.
    Claimed,
    /// Its result was submitted.
    Done,
}

impl Keyword for BoardStatus {
    const WHAT: &'static str = "board status";
}

impl TryFrom<String> for BoardStatus {
    type Error = UnknownKeyword;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

/// Where a request for a worker is in its life.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantNames,
)]
#[serde(try_from = "String", into = "&'static str")]
#[strum(serialize_all = "snake_case", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<DelegationStatus>)]
pub enum DelegationStatus {
    /// Waiting for a free worker.
    Queued,
    /// Given to a worker, whose task is being created.
    Starting,
    /// The worker is working on it.
    Running,
    /// The result went to the requester.
    Done,
    /// The worker failed or hung, or the request was cancelled.
    Failed,
    /// The worker could not do it without answers, which went to the requester.
    Unclear,
}

impl Keyword for DelegationStatus {
    const WHAT: &'static str = "delegation status";
}

impl TryFrom<String> for DelegationStatus {
    type Error = UnknownKeyword;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

/// The kind of an event the harness publishes to the team on the bus.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantNames,
)]
#[serde(try_from = "String", into = "&'static str")]
#[strum(serialize_all = "snake_case", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<NoticeKind>)]
pub enum NoticeKind {
    /// A member finished.
    MemberFinished,
    /// A member's run failed.
    MemberFailed,
    /// The human stopped a member.
    MemberStopped,
    /// A member's questions went to the coordinator.
    MemberQuestions,
    /// A member ended its turn without a status.
    MemberNoStatus,
    /// All members are asleep.
    TeamStuck,
    /// A worker was stopped by the watchdog.
    WorkerHung,
    /// A normal member was marked hung.
    AgentHung,
    /// The coordinator was marked hung.
    CoordinatorHung,
    /// A request for a worker finished.
    HelperResult,
    /// A request for a worker could not be started.
    HelperStartFailed,
    /// A member is paused until a given time.
    MemberPaused,
    /// A member is back after an absence.
    MemberBack,
    /// A member is out: its items are back on the board.
    MemberOut,
}

impl Keyword for NoticeKind {
    const WHAT: &'static str = "notice kind";
}

impl TryFrom<String> for NoticeKind {
    type Error = UnknownKeyword;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

/// Why a text is not a [`BoardItemId`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{value:?} is not a board item ID: `B` and a number above 0, such as B12")]
pub struct InvalidBoardItemId {
    /// The refused text.
    pub value: String,
}

/// The ID of a board item: `B` and its number, such as `B12`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct BoardItemId(NonZeroU32);

impl BoardItemId {
    /// The item with this number.
    #[must_use]
    pub const fn new(number: NonZeroU32) -> Self {
        Self(number)
    }

    /// The number.
    #[must_use]
    pub const fn number(self) -> NonZeroU32 {
        self.0
    }
}

impl fmt::Display for BoardItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "B{}", self.0)
    }
}

impl FromStr for BoardItemId {
    type Err = InvalidBoardItemId;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Err(InvalidBoardItemId { value: text.to_owned() })
    }
}

impl TryFrom<String> for BoardItemId {
    type Error = InvalidBoardItemId;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<BoardItemId> for String {
    fn from(id: BoardItemId) -> Self {
        id.to_string()
    }
}

/// The number of a request for a worker, shown as `H12`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct RequestId(NonZeroU64);

impl RequestId {
    /// The request with this number.
    #[must_use]
    pub const fn new(number: NonZeroU64) -> Self {
        Self(number)
    }

    /// The number.
    #[must_use]
    pub const fn number(self) -> NonZeroU64 {
        self.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "H{}", self.0)
    }
}

/// A member of a team to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMember {
    /// The task of its run.
    pub task: TaskId,
    /// Its branch.
    pub branch: BranchName,
    /// Its worktree.
    pub worktree: AbsolutePath,
    /// Normal or worker.
    pub tier: BusTier,
    /// Its state when the team is created.
    pub state: MemberState,
    /// The provider of its model, for balancing the load; none when it is not known.
    pub provider: Option<String>,
}

/// A team to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewTeam {
    /// Its ID.
    pub id: TeamId,
    /// The repository it works on.
    pub repo: RepoName,
    /// Its coordinator; none until one is chosen.
    pub lead: Option<AgentName>,
    /// Its members by name. A team without members is refused by the bus.
    pub members: BTreeMap<AgentName, NewMember>,
}

/// A member of a team as the bus has it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct BusMember {
    /// The task of its run.
    pub task: TaskId,
    /// Its branch.
    pub branch: BranchName,
    /// Its worktree.
    pub worktree: PathBuf,
    /// Its state.
    pub state: MemberState,
    /// Normal or worker.
    pub tier: BusTier,
    /// The provider of its model; none when it is not known.
    #[serde(default)]
    pub provider: Option<String>,
}

/// An item of a team's board.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct BoardItem {
    /// Its short description.
    pub title: String,
    /// Exactly what to do and how to return the result.
    pub details: String,
    /// Who should take it.
    pub level: BoardLevel,
    /// Where it is in its life.
    pub status: BoardStatus,
    /// Who added it; the result goes to them.
    pub creator: AgentName,
    /// Who claimed it.
    #[serde(default)]
    pub assignee: Option<AgentName>,
    /// The only member that may claim it; none when anyone at its level may.
    #[serde(default, rename = "for")]
    pub designated_for: Option<AgentName>,
    /// The result submitted when it was done; empty before.
    #[serde(default)]
    pub result: String,
    /// The latest progress note of whoever holds it.
    #[serde(default)]
    pub progress: String,
    /// Who held it before it went back to the board.
    #[serde(default)]
    pub previous: Option<AgentName>,
}

/// A team as the bus has it. Only what the daemon uses is read; the rest of what the bus keeps is left alone.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct BusTeam {
    /// Its ID.
    pub id: TeamId,
    /// The repository it works on.
    pub repo: RepoName,
    /// Its coordinator; none until one is chosen.
    #[serde(default, deserialize_with = "empty_as_none")]
    pub lead: Option<AgentName>,
    /// Whether its work is finished.
    pub done: bool,
    /// Its members by name.
    pub members: BTreeMap<AgentName, BusMember>,
    /// Its board.
    #[serde(default, deserialize_with = "null_as_default")]
    pub board: BTreeMap<BoardItemId, BoardItem>,
}

/// The board of a team.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct BoardDump {
    /// The items by ID, in the order of their numbers.
    #[serde(default, deserialize_with = "null_as_default")]
    pub board: BTreeMap<BoardItemId, BoardItem>,
}

/// Reads a JSON `null` as the default of the type.
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// Reads an empty string as none.
fn empty_as_none<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: fmt::Display,
{
    let text = String::deserialize(deserializer)?;
    if text.is_empty() {
        return Ok(None);
    }
    text.parse().map(Some).map_err(D::Error::custom)
}

/// Why `team-set` woke the member.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Woken {
    /// It did not: nothing waits for the member.
    No,
    /// A message waits for it.
    Message,
    /// New work for it is on the board.
    Board,
}

/// An item to put on a team's board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBoardItem {
    /// Its short description.
    pub title: String,
    /// Exactly what to do and how to return the result.
    pub details: String,
    /// Who should take it.
    pub level: BoardLevel,
    /// The only member that may claim it; none lets anyone at its level.
    pub designated_for: Option<AgentName>,
}

/// A request for a worker to queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delegation {
    /// The requester's team.
    pub team: TeamId,
    /// The member that asks.
    pub requester: AgentName,
    /// Its task.
    pub task: TaskId,
    /// Its repository.
    pub repo: RepoName,
    /// Its branch: the worker starts from its tip.
    pub branch: BranchName,
    /// Its worktree, which the worker may read.
    pub worktree: AbsolutePath,
    /// The command the bus runs with `_dispatch-helpers` once the request is queued.
    pub dispatcher: Program,
    /// What the worker is to do.
    pub request: String,
    /// What the worker needs to know.
    pub context: String,
}

/// A request for a worker, as the bus has it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct DelegationRequest {
    /// Its number.
    pub id: RequestId,
    /// The member that asked.
    pub from: AgentName,
    /// Its task.
    pub from_task: TaskId,
    /// Its repository.
    pub repo: RepoName,
    /// Its branch.
    pub branch: BranchName,
    /// Its worktree.
    pub worktree: PathBuf,
    /// What the worker is to do.
    pub request: String,
    /// What the worker needs to know.
    pub context: String,
    /// Where it is in its life.
    pub status: DelegationStatus,
    /// The worker it was given to.
    #[serde(default)]
    pub worker: Option<AgentName>,
    /// The task of the worker's run.
    #[serde(default)]
    pub helper_task: Option<TaskId>,
    /// The result; empty before.
    #[serde(default)]
    pub summary: String,
}

/// Changes to a request for a worker; what is `None` stays as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DelegationUpdate {
    /// A new status.
    pub status: Option<DelegationStatus>,
    /// The task of the worker's run.
    pub task: Option<TaskId>,
    /// The result.
    pub summary: Option<String>,
}

/// Where a checkpoint is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointKey {
    /// A team's, kept by its coordinator.
    Team(TeamId),
    /// A member's own task.
    Task(TaskId),
}

impl fmt::Display for CheckpointKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Team(team) => write!(f, "team-{team}"),
            Self::Task(task) => write!(f, "task-{task}"),
        }
    }
}

/// Who a message is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recipient {
    /// Everyone on the bus.
    All,
    /// One agent.
    Agent(AgentName),
    /// A team.
    Team(TeamId),
}

impl fmt::Display for Recipient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => f.write_str("all"),
            Self::Agent(agent) => write!(f, "{agent}"),
            Self::Team(team) => write!(f, "team:{team}"),
        }
    }
}

/// Why a text is not a [`FieldName`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{value:?} is not a field name: 1 to 64 letters, digits and `_`")]
pub struct InvalidFieldName {
    /// The refused text.
    pub value: String,
}

/// The name of a detail of an event: letters, digits and `_`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FieldName(String);

impl FieldName {
    /// The name's text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for FieldName {
    type Err = InvalidFieldName;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Err(InvalidFieldName { value: text.to_owned() })
    }
}

/// What to put on the bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outgoing {
    /// A message, as from a person.
    Chat {
        /// What it says.
        text: String,
    },
    /// An event of the harness.
    Notice {
        /// Its kind.
        kind: NoticeKind,
        /// Its details.
        fields: BTreeMap<FieldName, String>,
        /// What it says.
        text: String,
    },
}

/// A message that was put on the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SentMessage {
    /// Its number on the bus.
    pub id: NonZeroU64,
}

/// An agent that is running, as `agentbus status` lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveAgent {
    /// Its name.
    pub agent: AgentName,
    /// Its task.
    pub task: TaskId,
    /// Its branch.
    pub branch: BranchName,
    /// The process ID of its `agentbus serve`.
    pub pid: u32,
    /// What it last said it is doing.
    pub status: String,
}

/// What `agentbus status` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusStatus {
    /// The agents that are running.
    pub agents: Vec<ActiveAgent>,
    /// The reservations of shared resources, one line each, as the bus prints them: their text is free.
    pub reservations: Vec<String>,
}

/// The bus of one `AGENTS_HOME`, run through the `agentbus` command.
#[derive(Debug, Clone)]
pub struct AgentBus {
    /// The `agentbus` program.
    program: Program,
    /// The `AGENTS_HOME` it runs under; the bus is `bus/` in it.
    home: AbsolutePath,
    /// How long a command may run.
    timeout: Duration,
}

impl AgentBus {
    /// The bus of `home`, run with `program`.
    #[must_use]
    pub fn new(program: Program, home: AbsolutePath) -> Self {
        Self {
            program,
            home,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// The same, with another time for a command.
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The directory that holds the state of the bus.
    #[must_use]
    pub fn bus_dir(&self) -> PathBuf {
        self.home.as_path().join("bus")
    }

    /// Creates a team.
    ///
    /// # Errors
    /// [`AgentBusError`]; [`FailureClass::InvalidArgument`] for a team without members.
    pub async fn team_create(&self, team: &NewTeam) -> Result<(), AgentBusError> {
        let _ = (
            team,
            &self.program,
            self.timeout,
            MAX_OUTPUT_BYTES,
            MAX_DIAGNOSTIC_CHARS,
        );
        Err(wrong(BusCommand::TeamCreate))
    }

    /// Reads a team.
    ///
    /// # Errors
    /// [`AgentBusError`]; [`FailureClass::NotFound`] for a team that does not exist.
    pub async fn team_get(&self, team: &TeamId) -> Result<BusTeam, AgentBusError> {
        let _ = team;
        Err(wrong(BusCommand::TeamGet))
    }

    /// Sets the state of a member. A member that goes to sleep while something waits for it is woken at once, and
    /// the answer says why.
    ///
    /// # Errors
    /// [`AgentBusError`]; [`FailureClass::NotFound`] for a member that is not in the team.
    pub async fn team_set(
        &self,
        team: &TeamId,
        member: &AgentName,
        state: MemberState,
    ) -> Result<Woken, AgentBusError> {
        let _ = (team, member, state);
        Err(wrong(BusCommand::TeamSet))
    }

    /// Marks a team finished. A team that does not exist or is finished already is left as it is.
    ///
    /// # Errors
    /// [`AgentBusError`].
    pub async fn team_done(&self, team: &TeamId) -> Result<(), AgentBusError> {
        let _ = team;
        Err(wrong(BusCommand::TeamDone))
    }

    /// Adds an item to a team's board as one of its members.
    ///
    /// # Errors
    /// [`AgentBusError`]; [`FailureClass::NotFound`] when the member it is for is not in the team,
    /// [`FailureClass::InvalidArgument`] for an item for a worker that is not easy or one without a title.
    pub async fn board_add(
        &self,
        team: &TeamId,
        member: &AgentName,
        item: &NewBoardItem,
    ) -> Result<BoardItemId, AgentBusError> {
        let _ = (team, member, item);
        Err(wrong(BusCommand::BoardAdd))
    }

    /// Reads a team's board.
    ///
    /// # Errors
    /// [`AgentBusError`]; [`FailureClass::NotFound`] for a team that does not exist.
    pub async fn board_dump(&self, team: &TeamId) -> Result<BoardDump, AgentBusError> {
        let _ = team;
        Err(wrong(BusCommand::BoardDump))
    }

    /// Claims an item as a member. What the item asks is in [`AgentBus::board_dump`].
    ///
    /// # Errors
    /// [`AgentBusError`]; [`FailureClass::Conflict`] for an item that is taken, [`FailureClass::Permission`] for an
    /// item that is for someone else or not easy for a worker, [`FailureClass::NotFound`] for no such item.
    pub async fn board_claim(&self, team: &TeamId, member: &AgentName, item: BoardItemId) -> Result<(), AgentBusError> {
        let _ = (team, member, item);
        Err(wrong(BusCommand::BoardClaim))
    }

    /// Puts the items an agent holds, or that are for it, back on the board, and returns their IDs.
    ///
    /// # Errors
    /// [`AgentBusError`]; [`FailureClass::NotFound`] for a team that does not exist.
    pub async fn board_release_agent(
        &self,
        team: &TeamId,
        agent: &AgentName,
    ) -> Result<Vec<BoardItemId>, AgentBusError> {
        let _ = (team, agent);
        Err(wrong(BusCommand::BoardReleaseAgent))
    }

    /// Queues a request for a worker.
    ///
    /// # Errors
    /// [`AgentBusError`].
    pub async fn delegate(&self, delegation: &Delegation) -> Result<RequestId, AgentBusError> {
        let _ = delegation;
        Err(wrong(BusCommand::Delegate))
    }

    /// Gives the oldest queued request to `worker`, which marks it `starting`; none when the queue is empty.
    ///
    /// # Errors
    /// [`AgentBusError`].
    pub async fn helper_next(&self, worker: &AgentName) -> Result<Option<DelegationRequest>, AgentBusError> {
        let _ = worker;
        Err(wrong(BusCommand::HelperNext))
    }

    /// Changes a request for a worker.
    ///
    /// # Errors
    /// [`AgentBusError`]; [`FailureClass::NotFound`] for a request that does not exist.
    pub async fn helper_set(&self, request: RequestId, update: &DelegationUpdate) -> Result<(), AgentBusError> {
        let _ = (request, update);
        Err(wrong(BusCommand::HelperSet))
    }

    /// Reads a request for a worker.
    ///
    /// # Errors
    /// [`AgentBusError`]; [`FailureClass::NotFound`] for a request that does not exist.
    pub async fn helper_get(&self, request: RequestId) -> Result<DelegationRequest, AgentBusError> {
        let _ = request;
        Err(wrong(BusCommand::HelperGet))
    }

    /// The checkpoint kept under `key`; none when there is none.
    ///
    /// # Errors
    /// [`AgentBusError`].
    pub async fn checkpoint_get(&self, key: &CheckpointKey) -> Result<Option<String>, AgentBusError> {
        let _ = key;
        Err(wrong(BusCommand::CheckpointGet))
    }

    /// Puts a message on the bus.
    ///
    /// # Errors
    /// [`AgentBusError`]; [`FailureClass::InvalidArgument`] for an empty text.
    pub async fn send(
        &self,
        from: &AgentName,
        to: &Recipient,
        message: &Outgoing,
    ) -> Result<SentMessage, AgentBusError> {
        let _ = (from, to, message);
        Err(wrong(BusCommand::Send))
    }

    /// The agents that are running and the reservations.
    ///
    /// # Errors
    /// [`AgentBusError`].
    pub async fn status(&self) -> Result<BusStatus, AgentBusError> {
        Err(wrong(BusCommand::Status))
    }
}

/// An error that is no right answer to anything.
fn wrong(command: BusCommand) -> AgentBusError {
    AgentBusError::Unexpected {
        command,
        output: String::new(),
    }
}

/// Decodes the JSON a command printed.
fn decode_json<T: serde::de::DeserializeOwned>(command: BusCommand, output: &str) -> Result<T, AgentBusError> {
    let _ = output;
    Err(unexpected(command, output))
}

/// The error for output that is not in the format of `command`.
fn unexpected(command: BusCommand, output: &str) -> AgentBusError {
    AgentBusError::Unexpected {
        command,
        output: diagnostic(output.as_bytes()),
    }
}

/// A few characters of what a command printed, for an error: invalid UTF-8 is replaced, because the text is for a
/// human to read.
fn diagnostic(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim()
        .chars()
        .take(MAX_DIAGNOSTIC_CHARS)
        .collect()
}

/// What a finished command says: its standard output when it succeeded, an error when it did not.
fn outcome(command: BusCommand, captured: Captured) -> Result<String, AgentBusError> {
    let _ = (command, captured);
    Ok(String::new())
}

/// The ID of the item `board-add` reports: `Added B12 (normal). ...`.
fn parse_added(output: &str) -> Result<BoardItemId, AgentBusError> {
    Err(unexpected(BusCommand::BoardAdd, output))
}

/// Why `team-set` woke a member: nothing, `waking` or `waking (board)`.
fn parse_woken(output: &str) -> Result<Woken, AgentBusError> {
    Err(unexpected(BusCommand::TeamSet, output))
}

/// The IDs `board-release-agent` prints, one a line.
fn parse_released(output: &str) -> Result<Vec<BoardItemId>, AgentBusError> {
    Err(unexpected(BusCommand::BoardReleaseAgent, output))
}

/// The number of the request `delegate` accepted: `Request H12 accepted. ...`.
fn parse_request(output: &str) -> Result<RequestId, AgentBusError> {
    Err(unexpected(BusCommand::Delegate, output))
}

/// The message `send` printed: `[#12 2026-11-12 10:00:00] from → to: text`.
fn parse_sent(output: &str) -> Result<SentMessage, AgentBusError> {
    Err(unexpected(BusCommand::Send, output))
}

/// The report of `status`.
fn parse_status(output: &str) -> Result<BusStatus, AgentBusError> {
    Err(unexpected(BusCommand::Status, output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{ExitReport, Stream};

    fn captured(exit: ExitReport, stdout: &[u8], stderr: &[u8]) -> Captured {
        Captured {
            exit,
            stdout: Stream {
                bytes: stdout.to_vec(),
                dropped: 0,
            },
            stderr: Stream {
                bytes: stderr.to_vec(),
                dropped: 0,
            },
        }
    }

    fn exited(code: i32) -> ExitReport {
        ExitReport::Exited { code }
    }

    // Rule: the exit statuses 2 to 10 are the classes of failure `agentbus` gives; any other status is no class.
    #[test]
    fn an_exit_status_is_a_class_of_failure_when_agentbus_gives_it_one() {
        let classes = [
            (2, FailureClass::InvalidArgument),
            (3, FailureClass::NotFound),
            (4, FailureClass::NotInTeam),
            (5, FailureClass::Permission),
            (6, FailureClass::Conflict),
            (7, FailureClass::InvalidState),
            (8, FailureClass::Storage),
            (9, FailureClass::UnknownTool),
            (10, FailureClass::Process),
        ];
        for (code, class) in classes {
            assert_eq!(FailureClass::from_exit_code(code), class, "{code}");
        }
        for code in [-1, 0, 1, 11, 127, 255] {
            assert_eq!(FailureClass::from_exit_code(code), FailureClass::Unclassified, "{code}");
        }
    }

    #[test]
    fn a_command_that_succeeded_gives_its_output() {
        let done = captured(exited(0), "héllo\n".as_bytes(), b"noise");

        assert_eq!(outcome(BusCommand::Status, done).unwrap(), "héllo\n");
    }

    #[test]
    fn a_command_that_failed_gives_its_class_and_what_it_said_without_its_name() {
        let failed = captured(exited(3), b"", b"agentbus: no team team-x\n");

        let error = outcome(BusCommand::TeamGet, failed).unwrap_err();

        assert!(
            matches!(&error, AgentBusError::Failed { command: BusCommand::TeamGet, exit_code: 3, class: FailureClass::NotFound, message }
                if message == "no team team-x"),
            "{error:?}"
        );
        assert_eq!(error.class(), Some(FailureClass::NotFound));
    }

    #[test]
    fn what_a_failed_command_said_is_bounded_and_read_as_text_even_when_it_is_not_utf8() {
        let mut said = vec![0xff_u8];
        said.extend("ż".repeat(5000).as_bytes());

        let error = outcome(BusCommand::TeamGet, captured(exited(8), b"", &said)).unwrap_err();

        assert!(
            matches!(&error, AgentBusError::Failed { message, .. } if message.chars().count() == 2048),
            "{error:?}"
        );
    }

    #[test]
    fn a_command_ended_by_a_signal_is_killed() {
        let error = outcome(BusCommand::Send, captured(ExitReport::Signaled { signal: 9 }, b"", b"")).unwrap_err();

        assert!(
            matches!(
                error,
                AgentBusError::Killed {
                    command: BusCommand::Send,
                    signal: 9
                }
            ),
            "{error:?}"
        );
        assert_eq!(error.class(), None);
    }

    #[test]
    fn output_over_the_limit_is_an_error_because_it_is_cut() {
        let mut long = captured(exited(0), b"{", b"");
        long.stdout.dropped = 1;

        let error = outcome(BusCommand::TeamGet, long).unwrap_err();

        assert!(
            matches!(error, AgentBusError::OutputTooLong { command: BusCommand::TeamGet, limit } if limit == MAX_OUTPUT_BYTES),
            "{error:?}"
        );
    }

    #[test]
    fn output_that_is_not_utf8_is_an_error() {
        let error = outcome(BusCommand::Status, captured(exited(0), &[0xff, 0xfe], b"")).unwrap_err();

        assert!(
            matches!(
                error,
                AgentBusError::NotUtf8 {
                    command: BusCommand::Status,
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn board_add_reports_the_id_of_the_item() {
        let line = "Added B12 (normal). The result will arrive as a message from whoever does it.\n";

        assert_eq!(parse_added(line).unwrap().to_string(), "B12");
    }

    #[test]
    fn a_board_add_report_that_names_no_item_is_unexpected() {
        for text in [
            "",
            "Added",
            "Added Bx (normal).",
            "Added B0 (easy).",
            "Added 12 (easy).",
            "No team\n",
        ] {
            let error = parse_added(text).unwrap_err();

            assert!(
                matches!(
                    error,
                    AgentBusError::Unexpected {
                        command: BusCommand::BoardAdd,
                        ..
                    }
                ),
                "{text:?}: {error:?}"
            );
        }
    }

    #[test]
    fn team_set_reports_why_it_woke_the_member() {
        assert_eq!(parse_woken("").unwrap(), Woken::No);
        assert_eq!(parse_woken("waking\n").unwrap(), Woken::Message);
        assert_eq!(parse_woken("waking (board)\n").unwrap(), Woken::Board);
    }

    #[test]
    fn a_team_set_report_that_is_none_of_those_is_unexpected() {
        let error = parse_woken("sleeping\n").unwrap_err();

        assert!(
            matches!(
                error,
                AgentBusError::Unexpected {
                    command: BusCommand::TeamSet,
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn released_items_are_read_one_to_a_line() {
        let ids: Vec<String> = parse_released("B1\nB3\n")
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect();

        assert_eq!(ids, ["B1", "B3"]);
        assert_eq!(parse_released("").unwrap(), []);
    }

    #[test]
    fn a_released_line_that_is_no_item_is_unexpected() {
        let error = parse_released("B1\nxyz\n").unwrap_err();

        assert!(
            matches!(
                error,
                AgentBusError::Unexpected {
                    command: BusCommand::BoardReleaseAgent,
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn delegate_reports_the_number_of_the_request() {
        let text = "Request H7 accepted. The worker gets the state of your branch's last commit.\n";

        assert_eq!(parse_request(text).unwrap().to_string(), "H7");
    }

    #[test]
    fn a_delegate_report_without_a_request_number_is_unexpected() {
        for text in ["", "Request H0 accepted.", "Request accepted.", "accepted H7"] {
            let error = parse_request(text).unwrap_err();

            assert!(
                matches!(
                    error,
                    AgentBusError::Unexpected {
                        command: BusCommand::Delegate,
                        ..
                    }
                ),
                "{text:?}: {error:?}"
            );
        }
    }

    #[test]
    fn send_reports_the_number_of_the_message() {
        let line = "[#12 2026-11-12 10:00:00] claude-1 → team:team-x: hello [#99 not this]\n";

        assert_eq!(parse_sent(line).unwrap().id.get(), 12);
    }

    #[test]
    fn a_send_report_without_a_message_number_is_unexpected() {
        for text in ["", "[#0 2026-11-12 10:00:00] a → b: x", "[# ] x", "12", "[#x 1] a"] {
            let error = parse_sent(text).unwrap_err();

            assert!(
                matches!(
                    error,
                    AgentBusError::Unexpected {
                        command: BusCommand::Send,
                        ..
                    }
                ),
                "{text:?}: {error:?}"
            );
        }
    }

    #[test]
    fn status_without_agents_or_reservations_is_empty() {
        let report = parse_status("Active agents:\n  (none)\nReservations:\nNo reservations.\n").unwrap();

        assert_eq!(
            report,
            BusStatus {
                agents: Vec::new(),
                reservations: Vec::new()
            }
        );
    }

    #[test]
    fn status_lists_the_agents_with_their_free_text_status_and_the_reservations_as_lines() {
        let text = "Active agents:\n\
            \x20 claude-1 — task-20261112-134008-ab12 (agents/claude-1/item-1) pid 4242, status: reviewing (the diff), 2 left\n\
            \x20 codex-1 — task-20261112-134009-cd34 (agents/codex-1/item-2) pid 77, status: \n\
            Reservations:\n\
            - serial-port: codex-1 (task task-20261112-134009-cd34) since 2026-11-12 10:00:00, purpose: flashing\n\
            - lab: claude-1 (task task-20261112-134008-ab12) since 2026-11-12 10:05:00\n";

        let report = parse_status(text).unwrap();

        let agents: Vec<_> = report
            .agents
            .iter()
            .map(|agent| {
                (
                    agent.agent.as_str(),
                    agent.task.as_str(),
                    agent.branch.as_str(),
                    agent.pid,
                    agent.status.as_str(),
                )
            })
            .collect();
        assert_eq!(
            agents,
            [
                (
                    "claude-1",
                    "task-20261112-134008-ab12",
                    "agents/claude-1/item-1",
                    4242,
                    "reviewing (the diff), 2 left"
                ),
                ("codex-1", "task-20261112-134009-cd34", "agents/codex-1/item-2", 77, ""),
            ]
        );
        assert_eq!(
            report.reservations,
            [
                "- serial-port: codex-1 (task task-20261112-134009-cd34) since 2026-11-12 10:00:00, purpose: flashing",
                "- lab: claude-1 (task task-20261112-134008-ab12) since 2026-11-12 10:05:00",
            ]
        );
    }

    #[test]
    fn a_status_report_that_is_not_in_the_format_is_unexpected() {
        let cases = [
            "",
            "Reservations:\nNo reservations.\n",
            "Active agents:\n  (none)\n",
            "Active agents:\n  claude-1 — nonsense\nReservations:\nNo reservations.\n",
            "Active agents:\n  Claude-1 — task-20261112-134008-ab12 (agents/x) pid 1, status: s\nReservations:\nNo reservations.\n",
            "Active agents:\n  claude-1 — task-20261112-134008-ab12 (agents/x) pid 0x, status: s\nReservations:\nNo reservations.\n",
        ];
        for text in cases {
            let error = parse_status(text).unwrap_err();

            assert!(
                matches!(
                    error,
                    AgentBusError::Unexpected {
                        command: BusCommand::Status,
                        ..
                    }
                ),
                "{text:?}: {error:?}"
            );
        }
    }

    #[test]
    fn a_checkpoint_key_is_the_kind_and_the_id() {
        let team: TeamId = "team-20261112-134008-ab12".parse().unwrap();
        let task: TaskId = "task-20261112-134008-ab12".parse().unwrap();

        assert_eq!(CheckpointKey::Team(team).to_string(), "team-team-20261112-134008-ab12");
        assert_eq!(CheckpointKey::Task(task).to_string(), "task-task-20261112-134008-ab12");
    }

    #[test]
    fn a_recipient_is_written_the_way_the_bus_reads_it() {
        let team: TeamId = "team-20261112-134008-ab12".parse().unwrap();
        let agent: AgentName = "codex-1".parse().unwrap();

        assert_eq!(Recipient::All.to_string(), "all");
        assert_eq!(Recipient::Agent(agent).to_string(), "codex-1");
        assert_eq!(Recipient::Team(team).to_string(), "team:team-20261112-134008-ab12");
    }

    #[test]
    fn a_field_name_is_letters_digits_and_underscores() {
        for good in ["a", "member", "pause_until", "x9", "A_B"] {
            assert_eq!(good.parse::<FieldName>().unwrap().as_str(), good);
        }
        let long = "a".repeat(65);
        for bad in ["", "a=b", "a b", "żółw", "a-b", long.as_str()] {
            assert_eq!(
                bad.parse::<FieldName>().unwrap_err(),
                InvalidFieldName { value: bad.to_owned() },
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_board_item_id_is_b_and_a_number_above_zero() {
        for (text, number) in [("B1", 1), ("B12", 12), ("B4294967295", u32::MAX)] {
            let id: BoardItemId = text.parse().unwrap();
            assert_eq!((id.number().get(), id.to_string()), (number, text.to_owned()));
        }
        for bad in ["", "B", "B0", "b1", "1", "B-1", "B1 ", "B01", "B4294967296", "BB1"] {
            assert_eq!(
                bad.parse::<BoardItemId>().unwrap_err(),
                InvalidBoardItemId { value: bad.to_owned() },
                "{bad:?}"
            );
        }
    }

    #[test]
    fn board_item_ids_order_by_number() {
        let mut ids: Vec<BoardItemId> = ["B10", "B2", "B1"].iter().map(|text| text.parse().unwrap()).collect();

        ids.sort();

        let texts: Vec<_> = ids.iter().map(ToString::to_string).collect();
        assert_eq!(texts, ["B1", "B2", "B10"]);
    }

    #[test]
    fn a_decode_error_names_the_command() {
        let error = decode_json::<BusTeam>(BusCommand::TeamGet, "{\"id\": 5}").unwrap_err();

        assert!(
            matches!(
                error,
                AgentBusError::Decode {
                    command: BusCommand::TeamGet,
                    ..
                }
            ),
            "{error:?}"
        );
    }
}
