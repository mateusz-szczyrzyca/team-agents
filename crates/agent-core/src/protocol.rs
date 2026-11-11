//! JSON-line Unix-socket protocol between the `agent` CLI / `agent-web` dashboard and the daemon (`agentd`).
//!
//! Every [`Request`], [`Response`], and [`Event`] frame is a single UTF-8 JSON object terminated by `\n` and carrying
//! `"version": 1` ([`PROTOCOL_VERSION`]). Frames with any other protocol version are refused with
//! [`ProtocolError::UnsupportedVersion`] before interpreting the command, response, or event payload.

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU32;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::config::RepoName;
use crate::ids::{AgentName, InvalidId, ItemId, RunId, TaskId, TeamId};
use crate::questions::Verdict;
use crate::state::{
    GateResult, Member, MergeMode, PauseState, PullRequest, Question, RecordedAnswer, Review, Run, RunFinish, State,
    Team, TeamStatus, Timestamp,
};
use crate::status::{ItemStatus, TaskStatus};

/// Current wire protocol version for daemon socket messages.
pub const PROTOCOL_VERSION: u32 = 1;

/// Maximum length of a single JSON-line socket frame, in bytes (4 MiB).
pub const MAX_MESSAGE_BYTES: usize = 4 << 20;

/// Declares a versioned protocol enum (`Request`, `Response`, or `Event`) whose wire JSON representation flattens a
/// tagged inner enum beside `"version": 1`.
macro_rules! versioned_protocol_enum {
    (
        $(#[$enum_meta:meta])*
        pub enum $name:ident (raw = $raw:ident, raw_str = $raw_str:literal, body = $body:ident, tag = $tag:literal, check = $check:path) {
            $(
                $(#[$variant_meta:meta])*
                $variant:ident {
                    $(
                        $(#[$field_meta:meta])*
                        $field:ident : $fty:ty,
                    )*
                } => $wire_name:literal,
            )+
        }
    ) => {
        $(#[$enum_meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(try_from = $raw_str, into = $raw_str)]
        pub enum $name {
            $(
                $(#[$variant_meta])*
                $variant {
                    $(
                        $(#[$field_meta])*
                        $field : $fty,
                    )*
                },
            )+
        }

        /// Tagged payload inside [`$raw`].
        #[derive(Serialize, Deserialize)]
        #[serde(tag = $tag, rename_all = "snake_case")]
        enum $body {
            $(
                $variant {
                    $( $field : $fty, )*
                },
            )+
        }

        /// Versioned wire envelope for [`$name`].
        #[derive(Serialize, Deserialize)]
        struct $raw {
            /// Wire protocol version.
            version: u32,
            /// Tagged payload body.
            #[serde(flatten)]
            body: $body,
        }

        impl From<$name> for $raw {
            fn from(value: $name) -> Self {
                let body = match value {
                    $(
                        $name::$variant { $( $field ),* } => $body::$variant { $( $field ),* },
                    )+
                };
                Self {
                    version: PROTOCOL_VERSION,
                    body,
                }
            }
        }

        impl TryFrom<$raw> for $name {
            type Error = ProtocolError;

            fn try_from(raw: $raw) -> Result<Self, ProtocolError> {
                let value = match raw.body {
                    $(
                        $body::$variant { $( $field ),* } => Self::$variant { $( $field ),* },
                    )+
                };
                $check(raw.version, value)
            }
        }
    };
}

/// Why a string is neither a [`TeamId`] nor a [`TaskId`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid target ID {value:?}: expected a team-... or task-... identifier ({source})")]
pub struct InvalidTargetId {
    /// The rejected string.
    pub value: String,
    /// Underlying identifier parse error.
    #[source]
    pub source: InvalidId,
}

/// Identifies either a [`TeamId`] (`team-...`) or a [`TaskId`] (`task-...`) in control commands.
///
/// ```
/// # fn main() -> Result<(), agent_core::protocol::InvalidTargetId> {
/// use agent_core::protocol::TargetId;
///
/// let team: TargetId = "team-20260406-170900-a1b2".parse()?;
/// assert!(matches!(team, TargetId::Team(_)));
///
/// let task: TargetId = "task-20260406-171000-k3x9".parse()?;
/// assert!(matches!(task, TargetId::Task(_)));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum TargetId {
    /// Targets an entire team (`team-YYYYMMDD-HHMMSS-xxxx`).
    Team(TeamId),
    /// Targets a single task (`task-YYYYMMDD-HHMMSS-xxxx`).
    Task(TaskId),
}

impl TargetId {
    /// String slice of the underlying team or task identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Team(id) => id.as_str(),
            Self::Task(id) => id.as_str(),
        }
    }
}

impl From<TeamId> for TargetId {
    fn from(id: TeamId) -> Self {
        Self::Team(id)
    }
}

impl From<TaskId> for TargetId {
    fn from(id: TaskId) -> Self {
        Self::Task(id)
    }
}

impl fmt::Display for TargetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TargetId {
    type Err = InvalidTargetId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with("team-") {
            s.parse::<TeamId>().map(Self::Team).map_err(|source| InvalidTargetId {
                value: s.to_owned(),
                source,
            })
        } else {
            s.parse::<TaskId>().map(Self::Task).map_err(|source| InvalidTargetId {
                value: s.to_owned(),
                source,
            })
        }
    }
}

impl TryFrom<String> for TargetId {
    type Error = InvalidTargetId;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<TargetId> for String {
    fn from(target: TargetId) -> Self {
        target.to_string()
    }
}

/// What a [`Request::Run`] command starts: a solo agent task or a coordinated team working on a task list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum RunTarget {
    /// Run a single normal-tier agent on a prompt.
    Solo {
        /// Normal-tier agent to run.
        agent: AgentName,
        /// Prompt text for the task (must not be blank).
        prompt: String,
    },
    /// Run a coordinated team across a Markdown task list.
    Team {
        /// Optional explicit coordinator agent; if `None`, the daemon picks the first normal agent.
        coordinator: Option<AgentName>,
        /// Team member agents (must not be empty).
        agents: Vec<AgentName>,
        /// Whether approved pull requests merge automatically or wait for human merge.
        merge_mode: MergeMode,
        /// Markdown task-list document (must not be blank).
        task_list: String,
    },
}

/// How [`Request::Answer`] supplies answers for a task's pending clarification round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum AnswerPayload {
    /// Complete Markdown text of `questions-<round>.md`, marked with `**ANSWERED**`.
    FileContent(String),
    /// Per-question verdicts keyed by 1-based question number.
    Verdicts(BTreeMap<NonZeroU32, Verdict>),
}

/// Machine-readable error code returned inside [`Response::Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Request arguments failed validation.
    InvalidRequest,
    /// Target team, task, or question round does not exist.
    NotFound,
    /// Requested lifecycle state transition is not permitted from the current status.
    InvalidTransition,
    /// Operation conflicts with existing state (for example duplicate ID or already answered round).
    Conflict,
    /// Target or gate lock is currently busy.
    Busy,
    /// `make gate` failed.
    GateFailed,
    /// Unexpected internal daemon failure.
    Internal,
}

/// Validates a deserialized [`Request`] envelope (stubbed in RED).
fn check_raw_request(_version: u32, _req: Request) -> Result<Request, ProtocolError> {
    Err(ProtocolError::UnsupportedVersion {
        found: 0,
        supported: PROTOCOL_VERSION,
    })
}

/// Validates a deserialized [`Response`] envelope (stubbed in RED).
fn check_raw_response(_version: u32, _resp: Response) -> Result<Response, ProtocolError> {
    Err(ProtocolError::UnsupportedVersion {
        found: 0,
        supported: PROTOCOL_VERSION,
    })
}

/// Validates a deserialized [`Event`] envelope (stubbed in RED).
fn check_raw_event(_version: u32, _event: Event) -> Result<Event, ProtocolError> {
    Err(ProtocolError::UnsupportedVersion {
        found: 0,
        supported: PROTOCOL_VERSION,
    })
}

versioned_protocol_enum! {
    /// Client-to-daemon request sent as a single JSON line over the control Unix socket.
    ///
    /// Covers the 11 daemon commands: `run`, `status`, `show`, `answer`, `stop`, `pause`, `unpause`, `resume`, `say`,
    /// `gate`, and `subscribe`.
    pub enum Request (raw = RawRequest, raw_str = "RawRequest", body = RequestBody, tag = "command", check = check_raw_request) {
        /// Start a new solo task or team (`agent run`).
        Run {
            /// Repository name in `agents.toml`.
            repo: RepoName,
            /// Short human-readable topic (must not be blank).
            topic: String,
            /// Solo or team run specification.
            target: RunTarget,
        } => "run",
        /// Query the current daemon state (`agent status`).
        Status {
            /// Optional team or task filter.
            filter: Option<TargetId>,
        } => "status",
        /// Inspect a single team or task in detail (`agent show`).
        Show {
            /// Team or task to inspect.
            target: TargetId,
        } => "show",
        /// Record answers to a task's clarification questions (`agent answer`).
        Answer {
            /// Task (or team) whose pending questions are being answered.
            target: TargetId,
            /// Optional explicit clarification round number; defaults to the latest pending round.
            round: Option<NonZeroU32>,
            /// Answered Markdown file content or structured verdicts.
            payload: AnswerPayload,
        } => "answer",
        /// Stop a running team or task (`agent stop`).
        Stop {
            /// Team or task to stop.
            target: TargetId,
            /// Optional operator reason recorded in the log.
            reason: Option<String>,
        } => "stop",
        /// Pause a team or task (`agent pause`).
        Pause {
            /// Team or task to pause.
            target: TargetId,
            /// Optional operator reason recorded in [`PauseState`].
            reason: Option<String>,
        } => "pause",
        /// Unpause a paused team or task (`agent unpause`).
        Unpause {
            /// Team or task to unpause.
            target: TargetId,
        } => "unpause",
        /// Resume a stopped or failed team or task (`agent resume`).
        Resume {
            /// Team or task to resume.
            target: TargetId,
        } => "resume",
        /// Deliver a message from the operator to a team or task (`agent say`).
        Say {
            /// Destination team or task.
            target: TargetId,
            /// Message text (must not be blank).
            message: String,
        } => "say",
        /// Run `make gate` on a task's worktree (`agent gate`).
        Gate {
            /// Task whose worktree gate should be executed.
            task_id: TaskId,
        } => "gate",
        /// Subscribe to a live stream of [`Event`] frames after an initial state snapshot (`agent-web`).
        Subscribe {
            /// Optional team or task filter for the event stream.
            filter: Option<TargetId>,
        } => "subscribe",
    }
}

impl Request {
    /// Wire protocol version of this request (always [`PROTOCOL_VERSION`]).
    #[must_use]
    pub const fn version(&self) -> u32 {
        PROTOCOL_VERSION
    }

    /// Lowercase command name (`run`, `status`, `show`, `answer`, `stop`, `pause`, `unpause`, `resume`, `say`,
    /// `gate`, or `subscribe`).
    #[must_use]
    pub const fn command_name(&self) -> &'static str {
        match self {
            Self::Run { .. } => "run",
            Self::Status { .. } => "status",
            Self::Show { .. } => "show",
            Self::Answer { .. } => "answer",
            Self::Stop { .. } => "stop",
            Self::Pause { .. } => "pause",
            Self::Unpause { .. } => "unpause",
            Self::Resume { .. } => "resume",
            Self::Say { .. } => "say",
            Self::Gate { .. } => "gate",
            Self::Subscribe { .. } => "subscribe",
        }
    }

    /// Checks command-level invariants (non-empty topic, prompt, agent list, task list, message, and answers).
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if any required text or collection is empty.
    pub fn validate(&self) -> Result<(), ProtocolError> {
        Err(ProtocolError::EmptyLine)
    }

    /// Parses a single JSON line into a [`Request`], checking frame bounds and [`PROTOCOL_VERSION`] first.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if the frame is empty, contains multiple lines, exceeds [`MAX_MESSAGE_BYTES`],
    /// carries an unsupported `version`, or fails JSON/invariant validation.
    pub fn from_json_line(_line: &str) -> Result<Self, ProtocolError> {
        Err(ProtocolError::EmptyLine)
    }

    /// Serializes this [`Request`] as a single `\n`-terminated JSON line after validating its invariants.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if [`Self::validate`] fails or the serialized frame exceeds [`MAX_MESSAGE_BYTES`].
    pub fn to_json_line(&self) -> Result<String, ProtocolError> {
        Err(ProtocolError::EmptyLine)
    }
}

versioned_protocol_enum! {
    /// Daemon-to-client response sent as a single JSON line over the control Unix socket.
    pub enum Response (raw = RawResponse, raw_str = "RawResponse", body = ResponseBody, tag = "response", check = check_raw_response) {
        /// Acknowledgement of [`Request::Run`].
        RunStarted {
            /// Created team ID when `RunTarget::Team` was started.
            team_id: Option<TeamId>,
            /// Created task IDs (one for `Solo`, or the member tasks for `Team`).
            tasks: Vec<TaskId>,
        } => "run_started",
        /// Current state returned for [`Request::Status`].
        Status {
            /// Snapshot of matching teams and tasks.
            state: State,
        } => "status",
        /// Detailed team view returned for [`Request::Show`] when targeting a [`TeamId`].
        ShowTeam {
            /// Team metadata and item progress.
            team: Box<Team>,
            /// Member task records belonging to the team.
            members: Vec<Member>,
        } => "show_team",
        /// Detailed task view returned for [`Request::Show`] when targeting a [`TaskId`].
        ShowTask {
            /// Member task record.
            member: Box<Member>,
        } => "show_task",
        /// Acknowledgement of [`Request::Answer`].
        Answered {
            /// Task whose questions were answered.
            task_id: TaskId,
            /// Clarification round number that was answered.
            round: NonZeroU32,
            /// New task status after recording the answer (`Queued`).
            task_status: TaskStatus,
        } => "answered",
        /// Acknowledgement of [`Request::Stop`].
        Stopped {
            /// Stopped team or task.
            target: TargetId,
        } => "stopped",
        /// Acknowledgement of [`Request::Pause`].
        Paused {
            /// Paused team or task.
            target: TargetId,
            /// Recorded pause metadata.
            pause: PauseState,
        } => "paused",
        /// Acknowledgement of [`Request::Unpause`].
        Unpaused {
            /// Unpaused team or task.
            target: TargetId,
        } => "unpaused",
        /// Acknowledgement of [`Request::Resume`].
        Resumed {
            /// Resumed team or task.
            target: TargetId,
        } => "resumed",
        /// Acknowledgement of [`Request::Say`].
        Said {
            /// Target team or task that received the operator message.
            target: TargetId,
        } => "said",
        /// Result of [`Request::Gate`].
        Gate {
            /// Task whose worktree gate ran.
            task_id: TaskId,
            /// Recorded gate result.
            result: GateResult,
        } => "gate",
        /// Initial snapshot sent in response to [`Request::Subscribe`], followed by [`Event`] lines.
        Subscribed {
            /// Initial state snapshot at subscription time.
            snapshot: State,
        } => "subscribed",
        /// Error response when a command could not be fulfilled.
        Error {
            /// Machine-readable error category.
            code: ErrorCode,
            /// Human-readable explanation.
            message: String,
        } => "error",
    }
}

impl Response {
    /// Wire protocol version of this response (always [`PROTOCOL_VERSION`]).
    #[must_use]
    pub const fn version(&self) -> u32 {
        PROTOCOL_VERSION
    }

    /// Parses a single JSON line into a [`Response`], checking frame bounds and [`PROTOCOL_VERSION`] first.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if the frame is empty, contains multiple lines, exceeds [`MAX_MESSAGE_BYTES`],
    /// carries an unsupported `version`, or fails JSON validation.
    pub fn from_json_line(_line: &str) -> Result<Self, ProtocolError> {
        Err(ProtocolError::EmptyLine)
    }

    /// Serializes this [`Response`] as a single `\n`-terminated JSON line.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::TooLarge`] if the serialized frame exceeds [`MAX_MESSAGE_BYTES`].
    pub fn to_json_line(&self) -> Result<String, ProtocolError> {
        Err(ProtocolError::EmptyLine)
    }
}

versioned_protocol_enum! {
    /// State-change event streamed over a [`Request::Subscribe`] connection to the dashboard (`agent-web`).
    pub enum Event (raw = RawEvent, raw_str = "RawEvent", body = EventBody, tag = "event", check = check_raw_event) {
        /// A new team was created.
        TeamCreated {
            /// Newly created team record.
            team: Box<Team>,
            /// When the event occurred.
            recorded_at: Timestamp,
        } => "team_created",
        /// A team changed lifecycle status.
        TeamStatusChanged {
            /// Team identifier.
            team_id: TeamId,
            /// Previous team status.
            from: TeamStatus,
            /// New team status.
            to: TeamStatus,
            /// Active pause details if `to` is `Paused`.
            pause: Option<PauseState>,
            /// When the transition occurred.
            recorded_at: Timestamp,
        } => "team_status_changed",
        /// A new member or standalone task was created.
        TaskCreated {
            /// Newly created task record.
            member: Box<Member>,
            /// When the event occurred.
            recorded_at: Timestamp,
        } => "task_created",
        /// A task changed lifecycle status.
        TaskStatusChanged {
            /// Task identifier.
            task_id: TaskId,
            /// Enclosing team ID, if any.
            team_id: Option<TeamId>,
            /// Previous task status.
            from: TaskStatus,
            /// New task status.
            to: TaskStatus,
            /// Active pause details if `to` is `Paused`.
            pause: Option<PauseState>,
            /// When the transition occurred.
            recorded_at: Timestamp,
        } => "task_status_changed",
        /// A plan item inside a team changed status.
        ItemStatusChanged {
            /// Team identifier.
            team_id: TeamId,
            /// Plan item identifier (`1.1`, `2.2`, ...).
            item_id: ItemId,
            /// Previous item status.
            from: ItemStatus,
            /// New item status.
            to: ItemStatus,
            /// Current round number for the item.
            round: u32,
            /// Assigned member task, if any.
            assigned_task: Option<TaskId>,
            /// When the transition occurred.
            recorded_at: Timestamp,
        } => "item_status_changed",
        /// A new agent CLI invocation started inside a task.
        RunStarted {
            /// Task identifier.
            task_id: TaskId,
            /// Started run record.
            run: Run,
        } => "run_started",
        /// An agent CLI invocation finished.
        RunFinished {
            /// Task identifier.
            task_id: TaskId,
            /// Run identifier.
            run_id: RunId,
            /// 1-based attempt number.
            run_number: NonZeroU32,
            /// Completion details.
            finish: RunFinish,
        } => "run_finished",
        /// A task opened a new clarification question round (`STATUS: QUESTIONS`).
        QuestionsAsked {
            /// Task identifier.
            task_id: TaskId,
            /// Parsed question round.
            question: Question,
        } => "questions_asked",
        /// A task's clarification question round was answered.
        QuestionsAnswered {
            /// Task identifier.
            task_id: TaskId,
            /// Answered clarification round number.
            round: NonZeroU32,
            /// Recorded answers and source.
            answer: RecordedAnswer,
        } => "questions_answered",
        /// A `make gate` execution finished and was recorded.
        GateRecorded {
            /// Task identifier.
            task_id: TaskId,
            /// Plan item identifier, if associated with an item.
            item_id: Option<ItemId>,
            /// Gate result.
            result: GateResult,
        } => "gate_recorded",
        /// A pull request was opened or updated for a task or item.
        PullRequestUpdated {
            /// Task identifier.
            task_id: TaskId,
            /// Plan item identifier, if associated with an item.
            item_id: Option<ItemId>,
            /// Updated pull request metadata.
            pull_request: PullRequest,
            /// When the update was recorded.
            recorded_at: Timestamp,
        } => "pull_request_updated",
        /// A reviewer recorded a verdict on a pull request.
        ReviewRecorded {
            /// Enclosing team ID, if any.
            team_id: Option<TeamId>,
            /// Plan item ID, if any.
            item_id: Option<ItemId>,
            /// Recorded review verdict.
            review: Review,
        } => "review_recorded",
        /// An operator or bus message was delivered to a team or task.
        MessageDelivered {
            /// Destination team or task.
            target: TargetId,
            /// Sender name (`operator`, agent name, ...).
            sender: String,
            /// Delivered message text.
            message: String,
            /// When the message was delivered.
            recorded_at: Timestamp,
        } => "message_delivered",
    }
}

impl Event {
    /// Wire protocol version of this event (always [`PROTOCOL_VERSION`]).
    #[must_use]
    pub const fn version(&self) -> u32 {
        PROTOCOL_VERSION
    }

    /// Snake-case event name (`team_created`, `task_status_changed`, ...).
    #[must_use]
    pub const fn event_name(&self) -> &'static str {
        match self {
            Self::TeamCreated { .. } => "team_created",
            Self::TeamStatusChanged { .. } => "team_status_changed",
            Self::TaskCreated { .. } => "task_created",
            Self::TaskStatusChanged { .. } => "task_status_changed",
            Self::ItemStatusChanged { .. } => "item_status_changed",
            Self::RunStarted { .. } => "run_started",
            Self::RunFinished { .. } => "run_finished",
            Self::QuestionsAsked { .. } => "questions_asked",
            Self::QuestionsAnswered { .. } => "questions_answered",
            Self::GateRecorded { .. } => "gate_recorded",
            Self::PullRequestUpdated { .. } => "pull_request_updated",
            Self::ReviewRecorded { .. } => "review_recorded",
            Self::MessageDelivered { .. } => "message_delivered",
        }
    }

    /// Parses a single JSON line into an [`Event`], checking frame bounds and [`PROTOCOL_VERSION`] first.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] if the frame is empty, contains multiple lines, exceeds [`MAX_MESSAGE_BYTES`],
    /// carries an unsupported `version`, or fails JSON validation.
    pub fn from_json_line(_line: &str) -> Result<Self, ProtocolError> {
        Err(ProtocolError::EmptyLine)
    }

    /// Serializes this [`Event`] as a single `\n`-terminated JSON line.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::TooLarge`] if the serialized frame exceeds [`MAX_MESSAGE_BYTES`].
    pub fn to_json_line(&self) -> Result<String, ProtocolError> {
        Err(ProtocolError::EmptyLine)
    }
}

/// Why parsing, validating, or framing a protocol message failed.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// The JSON line exceeds [`MAX_MESSAGE_BYTES`].
    #[error("protocol frame is {size} bytes, exceeding the {max}-byte limit")]
    TooLarge {
        /// Actual byte length.
        size: usize,
        /// Maximum allowed byte length.
        max: usize,
    },
    /// The input line is empty or whitespace-only.
    #[error("protocol frame must not be empty")]
    EmptyLine,
    /// The input contains an embedded newline (frames are single JSON lines).
    #[error("protocol frame must be a single line without embedded newlines")]
    EmbeddedNewline,
    /// The frame's `version` is not [`PROTOCOL_VERSION`].
    #[error("unsupported protocol version {found}: this binary supports version {supported}")]
    UnsupportedVersion {
        /// Protocol version found in the frame.
        found: u32,
        /// Protocol version supported by this binary.
        supported: u32,
    },
    /// The JSON frame is syntactically invalid or fails field validation.
    #[error("invalid protocol JSON: {0}")]
    Parse(#[source] serde_json::Error),
    /// `Request::Run` has an empty or whitespace-only `topic`.
    #[error("run request topic must not be empty")]
    EmptyTopic,
    /// `RunTarget::Solo` has an empty or whitespace-only `prompt`.
    #[error("solo run prompt must not be empty")]
    EmptyPrompt,
    /// `RunTarget::Team` has an empty `agents` list.
    #[error("team run requires at least one agent")]
    EmptyAgents,
    /// `RunTarget::Team` has an empty or whitespace-only `task_list`.
    #[error("team run task list must not be empty")]
    EmptyTaskList,
    /// `Request::Say` has an empty or whitespace-only `message`.
    #[error("say message must not be empty")]
    EmptyMessage,
    /// `AnswerPayload::FileContent` is empty or whitespace-only.
    #[error("answer file content must not be empty")]
    EmptyAnswerFile,
    /// `AnswerPayload::Verdicts` has no question verdicts.
    #[error("answer verdicts map must not be empty")]
    EmptyVerdicts,
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::num::NonZeroU64;

    use super::*;
    use crate::kinds::{CliKind, Role};
    use crate::questions::Question as QuestionItem;
    use crate::state::{
        AnswerSource, CommitSha, FeatureSpec, GateOutcome, ItemProgress, Participant, PauseKind, PullRequestState,
        RunEnd, RunState, TreeState,
    };
    use crate::status::{ReviewVerdict, RunOutcome};

    fn ts(text: &str) -> Timestamp {
        text.parse().expect("valid timestamp")
    }

    fn sha(text: &str) -> CommitSha {
        text.parse().expect("valid sha")
    }

    fn nz32(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).expect("non-zero u32")
    }

    fn nz64(n: u64) -> NonZeroU64 {
        NonZeroU64::new(n).expect("non-zero u64")
    }

    fn sample_team_id() -> TeamId {
        "team-20260406-170900-a1b2".parse().expect("valid team id")
    }

    fn sample_task_id() -> TaskId {
        "task-20260406-171000-k3x9".parse().expect("valid task id")
    }

    fn sample_gate() -> GateResult {
        GateResult {
            outcome: GateOutcome::Passed,
            exit_code: Some(0),
            commit: sha("26b65de7e4076596768ca96b48a7f94799c93874"),
            tree_state: TreeState::Clean,
            started_at: ts("2026-04-06T17:15:00Z"),
            finished_at: ts("2026-04-06T17:16:30Z"),
            failed_step: None,
            summary: Some("ok".to_owned()),
        }
    }

    fn sample_member() -> Member {
        Member {
            task_id: sample_task_id(),
            team_id: Some(sample_team_id()),
            repo: "team-agents".parse().expect("valid repo"),
            topic: "Round 2.2".to_owned(),
            participant: Participant::Normal("codex".parse().expect("valid agent")),
            cli_kind: CliKind::Codex,
            model: None,
            role: Role::Member,
            status: TaskStatus::Running,
            branch: "agents/codex/task-20260406-171000-k3x9".parse().expect("valid branch"),
            base_commit: sha("1111111111111111111111111111111111111111"),
            head_commit: Some(sha("26b65de7e4076596768ca96b48a7f94799c93874")),
            worktree_path: "/home/dev/agents/worktrees/team-agents/task-20260406-171000-k3x9"
                .parse()
                .expect("valid path"),
            item_id: Some("2.2".parse().expect("valid item id")),
            created_at: ts("2026-04-06T17:10:00Z"),
            updated_at: ts("2026-04-06T17:15:00Z"),
            pause: None,
            pull_request: None,
            runs: vec![],
            questions: vec![],
            reviews: vec![],
            last_gate: Some(sample_gate()),
        }
    }

    fn sample_team() -> Team {
        let item_id: ItemId = "2.2".parse().expect("valid item id");
        let mut items = BTreeMap::new();
        items.insert(
            item_id,
            ItemProgress {
                item_id,
                title: "State and protocol".to_owned(),
                feature_number: 2,
                status: ItemStatus::InProgress,
                round: 1,
                assigned_task: Some(sample_task_id()),
                pull_request: None,
                gate_result: None,
                reviews: vec![],
            },
        );
        Team {
            team_id: sample_team_id(),
            repo: "team-agents".parse().expect("valid repo"),
            topic: "Rewrite bin/agent in Rust".to_owned(),
            status: TeamStatus::Working,
            merge_mode: MergeMode::Autopilot,
            coordinator: Participant::Normal("codex".parse().expect("valid agent")),
            created_at: ts("2026-04-06T17:09:00Z"),
            updated_at: ts("2026-04-06T17:15:00Z"),
            pause: None,
            features: vec![FeatureSpec {
                number: 2,
                title: "State".to_owned(),
                depends_on: vec![1],
            }],
            items,
            member_tasks: vec![sample_task_id()],
        }
    }

    #[test]
    fn all_eleven_request_commands_round_trip_as_json_lines() {
        let mut verdicts = BTreeMap::new();
        verdicts.insert(nz32(1), Verdict::AcceptRecommendation);

        let requests = vec![
            (
                "run",
                Request::Run {
                    repo: "team-agents".parse().expect("repo"),
                    topic: "Fix bug".to_owned(),
                    target: RunTarget::Solo {
                        agent: "codex".parse().expect("agent"),
                        prompt: "Fix the bug in state.rs".to_owned(),
                    },
                },
            ),
            (
                "run",
                Request::Run {
                    repo: "team-agents".parse().expect("repo"),
                    topic: "Rust rewrite".to_owned(),
                    target: RunTarget::Team {
                        coordinator: Some("claude".parse().expect("agent")),
                        agents: vec!["claude".parse().expect("agent"), "codex".parse().expect("agent")],
                        merge_mode: MergeMode::NoMerge,
                        task_list: "# Plan\n\n## Feature 1: Base\n\n- [ ] **1.1** Init\n".to_owned(),
                    },
                },
            ),
            (
                "status",
                Request::Status {
                    filter: Some(TargetId::from(sample_team_id())),
                },
            ),
            (
                "show",
                Request::Show {
                    target: TargetId::from(sample_task_id()),
                },
            ),
            (
                "answer",
                Request::Answer {
                    target: TargetId::from(sample_task_id()),
                    round: Some(nz32(1)),
                    payload: AnswerPayload::Verdicts(verdicts),
                },
            ),
            (
                "stop",
                Request::Stop {
                    target: TargetId::from(sample_team_id()),
                    reason: Some("operator stop".to_owned()),
                },
            ),
            (
                "pause",
                Request::Pause {
                    target: TargetId::from(sample_task_id()),
                    reason: Some("maintenance".to_owned()),
                },
            ),
            (
                "unpause",
                Request::Unpause {
                    target: TargetId::from(sample_task_id()),
                },
            ),
            (
                "resume",
                Request::Resume {
                    target: TargetId::from(sample_task_id()),
                },
            ),
            (
                "say",
                Request::Say {
                    target: TargetId::from(sample_team_id()),
                    message: "Please rebase on main".to_owned(),
                },
            ),
            (
                "gate",
                Request::Gate {
                    task_id: sample_task_id(),
                },
            ),
            ("subscribe", Request::Subscribe { filter: None }),
        ];

        for (expected_name, request) in requests {
            assert_eq!(request.version(), PROTOCOL_VERSION);
            assert_eq!(request.command_name(), expected_name);
            let line = request.to_json_line().expect("serialize request line");
            assert!(line.ends_with('\n'));
            assert_eq!(line.matches('\n').count(), 1);
            let parsed = Request::from_json_line(&line).expect("parse request line");
            assert_eq!(parsed, request);
            let via_serde: Request = serde_json::from_str(line.trim_end()).expect("serde request");
            assert_eq!(via_serde, request);
        }
    }

    #[test]
    fn request_response_and_event_with_unknown_protocol_version_are_refused_with_typed_error() {
        for bad_version in [0_u32, 2, 42] {
            let req_line = format!("{{\"version\":{bad_version},\"command\":\"future_cmd\",\"unexpected\":true}}\n");
            let req_err = Request::from_json_line(&req_line).unwrap_err();
            assert!(
                matches!(
                    req_err,
                    ProtocolError::UnsupportedVersion {
                        found,
                        supported: PROTOCOL_VERSION,
                    } if found == bad_version
                ),
                "unexpected request error for version {bad_version}: {req_err:?}"
            );

            let resp_line = format!("{{\"version\":{bad_version},\"response\":\"future_resp\"}}\n");
            let resp_err = Response::from_json_line(&resp_line).unwrap_err();
            assert!(matches!(
                resp_err,
                ProtocolError::UnsupportedVersion {
                    found,
                    supported: PROTOCOL_VERSION,
                } if found == bad_version
            ));

            let ev_line = format!("{{\"version\":{bad_version},\"event\":\"future_event\"}}\n");
            let ev_err = Event::from_json_line(&ev_line).unwrap_err();
            assert!(matches!(
                ev_err,
                ProtocolError::UnsupportedVersion {
                    found,
                    supported: PROTOCOL_VERSION,
                } if found == bad_version
            ));
        }
    }

    #[test]
    fn all_response_variants_round_trip_as_json_lines() {
        let member = sample_member();
        let team = sample_team();
        let state = State::from_teams_and_tasks(vec![team.clone()], vec![member.clone()]).expect("valid state");
        let pause = PauseState {
            kind: PauseKind::Manual,
            reason: Some("break".to_owned()),
            paused_at: ts("2026-04-06T17:20:00Z"),
            resume_at: None,
        };

        let responses = vec![
            Response::RunStarted {
                team_id: Some(sample_team_id()),
                tasks: vec![sample_task_id()],
            },
            Response::Status { state: state.clone() },
            Response::ShowTeam {
                team: Box::new(team),
                members: vec![member.clone()],
            },
            Response::ShowTask {
                member: Box::new(member),
            },
            Response::Answered {
                task_id: sample_task_id(),
                round: nz32(1),
                task_status: TaskStatus::Queued,
            },
            Response::Stopped {
                target: TargetId::from(sample_task_id()),
            },
            Response::Paused {
                target: TargetId::from(sample_task_id()),
                pause,
            },
            Response::Unpaused {
                target: TargetId::from(sample_task_id()),
            },
            Response::Resumed {
                target: TargetId::from(sample_task_id()),
            },
            Response::Said {
                target: TargetId::from(sample_team_id()),
            },
            Response::Gate {
                task_id: sample_task_id(),
                result: sample_gate(),
            },
            Response::Subscribed { snapshot: state },
            Response::Error {
                code: ErrorCode::InvalidTransition,
                message: "cannot resume running task".to_owned(),
            },
        ];

        for response in responses {
            assert_eq!(response.version(), PROTOCOL_VERSION);
            let line = response.to_json_line().expect("serialize response");
            let parsed = Response::from_json_line(&line).expect("parse response");
            assert_eq!(parsed, response);
        }
    }

    #[test]
    fn lifecycle_event_variants_round_trip_as_json_lines() {
        let member = sample_member();
        let team = sample_team();
        let pause = PauseState {
            kind: PauseKind::Manual,
            reason: Some("break".to_owned()),
            paused_at: ts("2026-04-06T17:20:00Z"),
            resume_at: None,
        };
        let finish = RunFinish {
            ended: RunEnd::StatusLine(RunOutcome::Done),
            exit_code: Some(0),
            finished_at: ts("2026-04-06T17:21:00Z"),
            commit: Some(sha("26b65de7e4076596768ca96b48a7f94799c93874")),
        };
        let run_id: RunId = "run-20260406-172000-r001".parse().expect("valid run id");

        let events = vec![
            Event::TeamCreated {
                team: Box::new(team),
                recorded_at: ts("2026-04-06T17:09:00Z"),
            },
            Event::TeamStatusChanged {
                team_id: sample_team_id(),
                from: TeamStatus::Working,
                to: TeamStatus::Paused,
                pause: Some(pause.clone()),
                recorded_at: ts("2026-04-06T17:20:00Z"),
            },
            Event::TaskCreated {
                member: Box::new(member),
                recorded_at: ts("2026-04-06T17:10:00Z"),
            },
            Event::TaskStatusChanged {
                task_id: sample_task_id(),
                team_id: Some(sample_team_id()),
                from: TaskStatus::Running,
                to: TaskStatus::Paused,
                pause: Some(pause),
                recorded_at: ts("2026-04-06T17:20:00Z"),
            },
            Event::ItemStatusChanged {
                team_id: sample_team_id(),
                item_id: "2.2".parse().expect("item id"),
                from: ItemStatus::Pending,
                to: ItemStatus::InProgress,
                round: 1,
                assigned_task: Some(sample_task_id()),
                recorded_at: ts("2026-04-06T17:10:00Z"),
            },
            Event::RunStarted {
                task_id: sample_task_id(),
                run: Run {
                    run_id: run_id.clone(),
                    run_number: nz32(1),
                    started_at: ts("2026-04-06T17:20:00Z"),
                    state: RunState::Running,
                },
            },
            Event::RunFinished {
                task_id: sample_task_id(),
                run_id,
                run_number: nz32(1),
                finish,
            },
        ];

        for event in events {
            assert_eq!(event.version(), PROTOCOL_VERSION);
            assert_ne!(event.event_name(), "");
            let line = event.to_json_line().expect("serialize event");
            let parsed = Event::from_json_line(&line).expect("parse event");
            assert_eq!(parsed, event);
        }
    }

    #[test]
    fn artifact_event_variants_round_trip_as_json_lines() {
        let mut verdicts = BTreeMap::new();
        verdicts.insert(nz32(1), Verdict::AcceptRecommendation);

        let events = vec![
            Event::QuestionsAsked {
                task_id: sample_task_id(),
                question: Question {
                    round: nz32(1),
                    asked_at: ts("2026-04-06T17:22:00Z"),
                    questions: vec![
                        QuestionItem::new(
                            nz32(1),
                            "Socket path?",
                            "a) XDG_RUNTIME_DIR\nb) AGENTS_HOME",
                            "a) — standard runtime dir",
                            None,
                        )
                        .expect("valid question"),
                    ],
                    answer: None,
                },
            },
            Event::QuestionsAnswered {
                task_id: sample_task_id(),
                round: nz32(1),
                answer: RecordedAnswer {
                    verdicts,
                    source: AnswerSource::Web,
                    answered_at: ts("2026-04-06T17:23:00Z"),
                },
            },
            Event::GateRecorded {
                task_id: sample_task_id(),
                item_id: Some("2.2".parse().expect("item id")),
                result: sample_gate(),
            },
            Event::PullRequestUpdated {
                task_id: sample_task_id(),
                item_id: Some("2.2".parse().expect("item id")),
                pull_request: PullRequest {
                    number: nz64(82),
                    head_branch: "agents/codex/task-20260406-171000-k3x9".parse().expect("branch"),
                    head_commit: sha("26b65de7e4076596768ca96b48a7f94799c93874"),
                    state: PullRequestState::Open,
                },
                recorded_at: ts("2026-04-06T17:24:00Z"),
            },
            Event::ReviewRecorded {
                team_id: Some(sample_team_id()),
                item_id: Some("2.2".parse().expect("item id")),
                review: Review {
                    reviewer: "inv-opus".parse().expect("investigator"),
                    task_id: sample_task_id(),
                    verdict: ReviewVerdict::Approve,
                    commit: sha("26b65de7e4076596768ca96b48a7f94799c93874"),
                    recorded_at: ts("2026-04-06T17:25:00Z"),
                    summary: None,
                },
            },
            Event::MessageDelivered {
                target: TargetId::from(sample_task_id()),
                sender: "operator".to_owned(),
                message: "Check gate logs".to_owned(),
                recorded_at: ts("2026-04-06T17:26:00Z"),
            },
        ];

        for event in events {
            assert_eq!(event.version(), PROTOCOL_VERSION);
            assert_ne!(event.event_name(), "");
            let line = event.to_json_line().expect("serialize event");
            let parsed = Event::from_json_line(&line).expect("parse event");
            assert_eq!(parsed, event);
        }
    }

    #[test]
    fn framing_and_payload_validation_errors_are_refused() {
        assert!(matches!(
            Request::from_json_line("   \n"),
            Err(ProtocolError::EmptyLine)
        ));
        assert!(matches!(
            Request::from_json_line("{\"version\":1}\n{\"version\":1}\n"),
            Err(ProtocolError::EmbeddedNewline)
        ));
        let oversized = " ".repeat(MAX_MESSAGE_BYTES + 1);
        assert!(matches!(
            Request::from_json_line(&oversized),
            Err(ProtocolError::TooLarge {
                size,
                max: MAX_MESSAGE_BYTES,
            }) if size == MAX_MESSAGE_BYTES + 1
        ));

        let empty_topic = Request::Run {
            repo: "team-agents".parse().expect("repo"),
            topic: "   ".to_owned(),
            target: RunTarget::Solo {
                agent: "codex".parse().expect("agent"),
                prompt: "do work".to_owned(),
            },
        };
        assert!(matches!(empty_topic.to_json_line(), Err(ProtocolError::EmptyTopic)));

        let empty_prompt = Request::Run {
            repo: "team-agents".parse().expect("repo"),
            topic: "Topic".to_owned(),
            target: RunTarget::Solo {
                agent: "codex".parse().expect("agent"),
                prompt: " ".to_owned(),
            },
        };
        assert!(matches!(empty_prompt.to_json_line(), Err(ProtocolError::EmptyPrompt)));

        let empty_agents = Request::Run {
            repo: "team-agents".parse().expect("repo"),
            topic: "Topic".to_owned(),
            target: RunTarget::Team {
                coordinator: None,
                agents: vec![],
                merge_mode: MergeMode::Autopilot,
                task_list: "# Plan".to_owned(),
            },
        };
        assert!(matches!(empty_agents.to_json_line(), Err(ProtocolError::EmptyAgents)));

        let empty_task_list = Request::Run {
            repo: "team-agents".parse().expect("repo"),
            topic: "Topic".to_owned(),
            target: RunTarget::Team {
                coordinator: None,
                agents: vec!["codex".parse().expect("agent")],
                merge_mode: MergeMode::Autopilot,
                task_list: "\n\t ".to_owned(),
            },
        };
        assert!(matches!(
            empty_task_list.to_json_line(),
            Err(ProtocolError::EmptyTaskList)
        ));

        let empty_say = Request::Say {
            target: TargetId::from(sample_task_id()),
            message: "  ".to_owned(),
        };
        assert!(matches!(empty_say.to_json_line(), Err(ProtocolError::EmptyMessage)));

        let empty_answer_file = Request::Answer {
            target: TargetId::from(sample_task_id()),
            round: None,
            payload: AnswerPayload::FileContent(" ".to_owned()),
        };
        assert!(matches!(
            empty_answer_file.to_json_line(),
            Err(ProtocolError::EmptyAnswerFile)
        ));

        let empty_verdicts = Request::Answer {
            target: TargetId::from(sample_task_id()),
            round: None,
            payload: AnswerPayload::Verdicts(BTreeMap::new()),
        };
        assert!(matches!(
            empty_verdicts.to_json_line(),
            Err(ProtocolError::EmptyVerdicts)
        ));
    }

    #[test]
    fn target_id_and_protocol_parse_errors_preserve_error_sources() {
        let bad_target = "run-20260406-171000-r001".parse::<TargetId>().unwrap_err();
        assert!(bad_target.source().is_some());

        let bad_json = Request::from_json_line("{bad-json\n").unwrap_err();
        assert!(matches!(bad_json, ProtocolError::Parse(_)));
        assert!(bad_json.source().is_some());
    }
}
