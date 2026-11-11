//! Persisted daemon state (`state.json`): teams, member tasks, plan-item progress, runs, clarification questions,
//! code reviews, and gate results.
//!
//! `agent-core` defines the schema, invariants, and JSON conversion without performing file I/O. Every state record
//! carries [`SCHEMA_VERSION`]; [`State::from_json_str`] (and [`serde::Deserialize`]) refuses any other schema version
//! with [`StateError::UnsupportedVersion`] before interpreting the rest of the payload.

use std::collections::BTreeMap;
use std::fmt;
use std::num::{NonZeroU32, NonZeroU64};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use time::{Date, Month, PrimitiveDateTime, Time, UtcDateTime};

use crate::config::{AbsolutePath, ModelName, RepoName};
use crate::ids::{AgentName, BranchName, InvestigatorName, ItemId, RunId, TaskId, TeamId};
use crate::kinds::{CliKind, Role, Tier};
use crate::questions::{Question as QuestionItem, Verdict};
use crate::status::{ItemStatus, ReviewVerdict, RunOutcome, TaskStatus};

/// Current schema version of `state.json`.
pub const SCHEMA_VERSION: u32 = 1;

/// Maximum size of a serialized `state.json` document, in bytes (16 MiB).
pub const MAX_STATE_BYTES: usize = 16 << 20;

/// Shortest allowed git commit SHA (abbreviated SHA).
const MIN_COMMIT_SHA_LEN: usize = 7;

/// Longest allowed git commit SHA (SHA-256 hex length).
const MAX_COMMIT_SHA_LEN: usize = 64;

/// Length of an RFC 3339 whole-second UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`).
const TIMESTAMP_LEN: usize = 20;

/// Why a timestamp string could not be parsed as a [`Timestamp`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidTimestamp {
    /// The string does not match the `YYYY-MM-DDTHH:MM:SSZ` format.
    #[error("invalid UTC timestamp {0:?}: expected YYYY-MM-DDTHH:MM:SSZ")]
    Format(String),
    /// The calendar date or clock time is out of range (for example `2026-02-30T00:00:00Z`).
    #[error("invalid UTC calendar date or time in {value:?}: {source}")]
    OutOfRange {
        /// The rejected timestamp text.
        value: String,
        /// Which date or time component was out of range.
        #[source]
        source: time::error::ComponentRange,
    },
    /// A Unix timestamp in seconds is outside the representable date range.
    #[error("unix timestamp {seconds} is out of range: {source}")]
    UnixTimestampOutOfRange {
        /// The rejected Unix timestamp in seconds.
        seconds: i64,
        /// Range error from `time`.
        #[source]
        source: time::error::ComponentRange,
    },
}

/// A whole-second UTC timestamp formatted as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// ```
/// # fn main() -> Result<(), agent_core::state::InvalidTimestamp> {
/// use agent_core::state::Timestamp;
///
/// let ts: Timestamp = "2026-04-06T17:19:45Z".parse()?;
/// assert_eq!(ts.to_string(), "2026-04-06T17:19:45Z");
/// # Ok(())
/// # }
/// ```
///
/// A [`Timestamp`] cannot be built from an unvalidated string outside this module:
///
/// ```compile_fail,E0423
/// let _ = agent_core::state::Timestamp("2026-04-06".to_owned());
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Timestamp(UtcDateTime);

impl Timestamp {
    /// Creates a [`Timestamp`] from a [`UtcDateTime`], truncating any subsecond nanoseconds.
    #[must_use]
    pub fn from_utc(dt: UtcDateTime) -> Self {
        let time = Time::from_hms(dt.hour(), dt.minute(), dt.second()).unwrap_or(Time::MIDNIGHT);
        Self(PrimitiveDateTime::new(dt.date(), time).assume_utc().into())
    }

    /// Creates a [`Timestamp`] from whole seconds since the Unix epoch (`1970-01-01T00:00:00Z`).
    ///
    /// # Errors
    ///
    /// Returns [`InvalidTimestamp::UnixTimestampOutOfRange`] if `seconds` is outside [`UtcDateTime`]'s range.
    pub fn from_unix_timestamp(seconds: i64) -> Result<Self, InvalidTimestamp> {
        UtcDateTime::from_unix_timestamp(seconds)
            .map(Self::from_utc)
            .map_err(|source| InvalidTimestamp::UnixTimestampOutOfRange { seconds, source })
    }

    /// Whole seconds since the Unix epoch (`1970-01-01T00:00:00Z`).
    #[must_use]
    pub const fn unix_timestamp(self) -> i64 {
        self.0.unix_timestamp()
    }

    /// The underlying [`UtcDateTime`].
    #[must_use]
    pub const fn as_utc_date_time(self) -> UtcDateTime {
        self.0
    }
}

impl From<UtcDateTime> for Timestamp {
    fn from(dt: UtcDateTime) -> Self {
        Self::from_utc(dt)
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Timestamp({self})")
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dt = self.0;
        let year = dt.year();
        let month = u8::from(dt.month());
        let day = dt.day();
        let hour = dt.hour();
        let minute = dt.minute();
        let second = dt.second();
        write!(f, "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
    }
}

impl FromStr for Timestamp {
    type Err = InvalidTimestamp;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_timestamp(s)
    }
}

impl TryFrom<String> for Timestamp {
    type Error = InvalidTimestamp;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        parse_timestamp(&value)
    }
}

impl From<Timestamp> for String {
    fn from(value: Timestamp) -> Self {
        value.to_string()
    }
}

/// Parses an `YYYY-MM-DDTHH:MM:SSZ` string into a [`Timestamp`].
fn parse_timestamp(s: &str) -> Result<Timestamp, InvalidTimestamp> {
    let Some((year, month_num, day, hour, minute, second)) = read_timestamp_parts(s) else {
        return Err(InvalidTimestamp::Format(s.to_owned()));
    };
    let out_of_range = |source| InvalidTimestamp::OutOfRange {
        value: s.to_owned(),
        source,
    };
    let month = Month::try_from(month_num).map_err(out_of_range)?;
    let date = Date::from_calendar_date(i32::from(year), month, day).map_err(out_of_range)?;
    let time = Time::from_hms(hour, minute, second).map_err(out_of_range)?;
    Ok(Timestamp(PrimitiveDateTime::new(date, time).assume_utc().into()))
}

/// Reads the six numeric components of `YYYY-MM-DDTHH:MM:SSZ`, returning `None` if the shape differs.
fn read_timestamp_parts(s: &str) -> Option<(u16, u8, u8, u8, u8, u8)> {
    if s.len() != TIMESTAMP_LEN {
        return None;
    }
    let mut bytes = s.bytes();
    let century = two_digits(&mut bytes)?;
    let year = u16::from(century) * 100 + u16::from(two_digits(&mut bytes)?);
    expect_byte(&mut bytes, b'-')?;
    let month = two_digits(&mut bytes)?;
    expect_byte(&mut bytes, b'-')?;
    let day = two_digits(&mut bytes)?;
    expect_byte(&mut bytes, b'T')?;
    let hour = two_digits(&mut bytes)?;
    expect_byte(&mut bytes, b':')?;
    let minute = two_digits(&mut bytes)?;
    expect_byte(&mut bytes, b':')?;
    let second = two_digits(&mut bytes)?;
    expect_byte(&mut bytes, b'Z')?;
    bytes
        .next()
        .is_none()
        .then_some((year, month, day, hour, minute, second))
}

/// Reads a single decimal digit from `bytes`.
fn digit(bytes: &mut impl Iterator<Item = u8>) -> Option<u8> {
    bytes.next().filter(u8::is_ascii_digit).map(|b| b - b'0')
}

/// Reads a two-digit decimal number (`00..=99`) from `bytes`.
fn two_digits(bytes: &mut impl Iterator<Item = u8>) -> Option<u8> {
    Some(digit(bytes)? * 10 + digit(bytes)?)
}

/// Consumes the next byte if it equals `expected`.
fn expect_byte(bytes: &mut impl Iterator<Item = u8>, expected: u8) -> Option<()> {
    bytes.next().filter(|&b| b == expected).map(|_| ())
}

/// Why a string is not a valid [`CommitSha`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidCommitSha {
    /// The string is empty.
    #[error("commit SHA must not be empty")]
    Empty,
    /// The string is shorter than 7 hex characters.
    #[error("commit SHA {value:?} is shorter than {MIN_COMMIT_SHA_LEN} hex characters")]
    TooShort {
        /// The rejected string.
        value: String,
    },
    /// The string is longer than 64 hex characters.
    #[error("commit SHA {value:?} is longer than {MAX_COMMIT_SHA_LEN} hex characters")]
    TooLong {
        /// The rejected string.
        value: String,
    },
    /// The string contains a character outside `0..=9` and `a..=f`.
    #[error("commit SHA {value:?} contains non-lowercase-hex character {ch:?}")]
    NotLowercaseHex {
        /// The rejected string.
        value: String,
        /// The first offending character.
        ch: char,
    },
}

/// A git commit SHA: 7 to 64 lowercase ASCII hexadecimal characters (`0-9`, `a-f`).
///
/// ```
/// # fn main() -> Result<(), agent_core::state::InvalidCommitSha> {
/// use agent_core::state::CommitSha;
///
/// let sha: CommitSha = "26b65de7e4076596768ca96b48a7f94799c93874".parse()?;
/// assert_eq!(sha.as_str(), "26b65de7e4076596768ca96b48a7f94799c93874");
/// # Ok(())
/// # }
/// ```
///
/// The inner field is private so invalid SHAs cannot be constructed:
///
/// ```compile_fail,E0423
/// let _ = agent_core::state::CommitSha(String::from("not-a-sha"));
/// ```
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CommitSha(String);

impl CommitSha {
    /// The validated lowercase hexadecimal SHA string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CommitSha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CommitSha({:?})", self.0)
    }
}

impl fmt::Display for CommitSha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for CommitSha {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for CommitSha {
    type Err = InvalidCommitSha;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        validate_commit_sha(s)?;
        Ok(Self(s.to_owned()))
    }
}

impl TryFrom<String> for CommitSha {
    type Error = InvalidCommitSha;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_commit_sha(&value)?;
        Ok(Self(value))
    }
}

impl From<CommitSha> for String {
    fn from(value: CommitSha) -> Self {
        value.0
    }
}

/// Checks that `s` is 7..=64 lowercase ASCII hex digits.
fn validate_commit_sha(s: &str) -> Result<(), InvalidCommitSha> {
    if s.is_empty() {
        return Err(InvalidCommitSha::Empty);
    }
    if s.len() < MIN_COMMIT_SHA_LEN {
        return Err(InvalidCommitSha::TooShort { value: s.to_owned() });
    }
    if s.len() > MAX_COMMIT_SHA_LEN {
        return Err(InvalidCommitSha::TooLong { value: s.to_owned() });
    }
    if let Some(ch) = s.chars().find(|c| !matches!(c, '0'..='9' | 'a'..='f')) {
        return Err(InvalidCommitSha::NotLowercaseHex {
            value: s.to_owned(),
            ch,
        });
    }
    Ok(())
}

/// Whether a team merges approved pull requests automatically (`--autopilot`) or stops for the human (`--no-merge`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMode {
    /// Merge automatically once the gate and reviews pass (`--autopilot`).
    Autopilot,
    /// Leave approved pull requests open for human merge (`--no-merge`).
    NoMerge,
}

/// Lifecycle status of a [`Team`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamStatus {
    /// The team is working through its plan items.
    Working,
    /// The team is paused by the operator or due to provider limits.
    Paused,
    /// The team was stopped by the operator before finishing all items.
    Stopped,
    /// An unrecoverable error stopped the team.
    Failed,
    /// Every item in the team's plan is done.
    Done,
}

/// The agent or investigator identity assigned to a [`Member`] task.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "tier", content = "name", rename_all = "snake_case")]
pub enum Participant {
    /// A normal-tier agent (`Tier::Normal`).
    Normal(AgentName),
    /// A worker-tier helper agent (`Tier::Worker`).
    Worker(AgentName),
    /// An independent investigator or reviewer (`Tier::Investigator`).
    Investigator(InvestigatorName),
}

impl Participant {
    /// The [`Tier`] of this participant variant.
    #[must_use]
    pub const fn tier(&self) -> Tier {
        match self {
            Self::Normal(_) => Tier::Normal,
            Self::Worker(_) => Tier::Worker,
            Self::Investigator(_) => Tier::Investigator,
        }
    }

    /// Whether this participant may take `role` in a task or team.
    #[must_use]
    pub const fn allows_role(&self, role: Role) -> bool {
        match self {
            Self::Normal(_) => matches!(role, Role::Solo | Role::Coordinator | Role::Member),
            Self::Worker(_) => matches!(role, Role::Worker),
            Self::Investigator(_) => matches!(role, Role::Coordinator | Role::Reviewer),
        }
    }

    /// The participant's [`AgentName`], if it is a normal or worker agent.
    #[must_use]
    pub const fn agent_name(&self) -> Option<&AgentName> {
        match self {
            Self::Normal(name) | Self::Worker(name) => Some(name),
            Self::Investigator(_) => None,
        }
    }

    /// The participant's [`InvestigatorName`], if it is an investigator.
    #[must_use]
    pub const fn investigator_name(&self) -> Option<&InvestigatorName> {
        match self {
            Self::Investigator(name) => Some(name),
            Self::Normal(_) | Self::Worker(_) => None,
        }
    }

    /// The participant's name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Normal(name) | Self::Worker(name) => name.as_str(),
            Self::Investigator(name) => name.as_str(),
        }
    }
}

/// Why a team or member task is paused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseKind {
    /// Paused explicitly by the operator (`agent pause`).
    Manual,
    /// Paused automatically because the provider reported exhausted quota.
    QuotaExhausted,
    /// Paused automatically because the provider rate-limited the agent CLI.
    RateLimited,
}

/// Active pause metadata on a [`Team`] or [`Member`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PauseState {
    /// Why the target was paused.
    pub kind: PauseKind,
    /// Human-readable reason or provider error excerpt.
    pub reason: Option<String>,
    /// When the pause took effect.
    pub paused_at: Timestamp,
    /// When an automatic pause is scheduled to retry, if known.
    pub resume_at: Option<Timestamp>,
}

/// Cleanliness of a git working tree when a gate check runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeState {
    /// No uncommitted changes or untracked files.
    Clean,
    /// Modified or untracked files are present.
    Dirty,
    /// A git merge or rebase is still in progress.
    MergeInProgress,
}

/// Outcome of running the repository gate command (`make gate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateOutcome {
    /// All gate checks exited with code `0` on a clean tree.
    Passed,
    /// One or more gate checks exited non-zero or the tree was not clean.
    Failed,
    /// The gate command exceeded its configured timeout.
    TimedOut,
}

/// Recorded result of running `make gate` on a task worktree or item branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateResult {
    /// Whether the gate passed, failed, or timed out.
    pub outcome: GateOutcome,
    /// Process exit code, when the command exited normally.
    pub exit_code: Option<i32>,
    /// Commit SHA at `HEAD` when the gate ran.
    pub commit: CommitSha,
    /// Working-tree state observed for the gate run.
    pub tree_state: TreeState,
    /// When the gate command started.
    pub started_at: Timestamp,
    /// When the gate command finished.
    pub finished_at: Timestamp,
    /// Name of the first failing gate step, if any.
    pub failed_step: Option<String>,
    /// Short failure or pass summary line.
    pub summary: Option<String>,
}

impl GateResult {
    /// Whether the gate passed on a clean working tree.
    #[must_use]
    pub const fn passed(&self) -> bool {
        matches!(self.outcome, GateOutcome::Passed) && matches!(self.tree_state, TreeState::Clean)
    }
}

/// State of a GitHub pull request tracked for a task or plan item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestState {
    /// Open as a draft pull request.
    Draft,
    /// Open and ready for review.
    Open,
    /// Merged into the target branch.
    Merged,
    /// Closed without merging.
    Closed,
}

/// GitHub pull request metadata recorded on an [`ItemProgress`] or [`Member`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    /// GitHub pull request number (`#1`, `#81`, ...).
    pub number: NonZeroU64,
    /// Head branch of the pull request.
    pub head_branch: BranchName,
    /// Commit SHA at the tip of `head_branch`.
    pub head_commit: CommitSha,
    /// Current state of the pull request.
    pub state: PullRequestState,
}

/// One reviewer's recorded verdict on a pull request or round commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    /// Investigator who performed the review.
    pub reviewer: InvestigatorName,
    /// Task ID of the review run.
    pub task_id: TaskId,
    /// Review outcome (`Approve` or `RequestChanges`).
    pub verdict: ReviewVerdict,
    /// Commit SHA that was reviewed.
    pub commit: CommitSha,
    /// When the review verdict was recorded.
    pub recorded_at: Timestamp,
    /// Optional summary of findings.
    pub summary: Option<String>,
}

/// Progress of a single plan item ([`ItemId`]) inside a [`Team`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemProgress {
    /// Item identifier (`1.1`, `2.2`, ...).
    pub item_id: ItemId,
    /// Item title from the task list.
    pub title: String,
    /// Feature number this item belongs to.
    pub feature_number: u32,
    /// Current lifecycle status of the item.
    pub status: ItemStatus,
    /// Current round number (`1` for the first attempt, incremented on retry).
    pub round: u32,
    /// Lead or implementer task currently assigned to this item, if any.
    pub assigned_task: Option<TaskId>,
    /// Pull request opened for this item, if any.
    pub pull_request: Option<PullRequest>,
    /// Most recent gate result for this item, if any.
    pub gate_result: Option<GateResult>,
    /// Code reviews recorded for this item's pull request.
    pub reviews: Vec<Review>,
}

/// How a finished agent CLI run terminated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum RunEnd {
    /// The agent printed a final `STATUS: ...` line.
    StatusLine(RunOutcome),
    /// The process was terminated by a Unix signal.
    Signal(i32),
    /// The watchdog stopped the process after exceeding the run timeout.
    WatchdogTimeout,
    /// The process could not be spawned or failed before producing a status line.
    SpawnFailure(String),
}

/// Completion details of a finished [`Run`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunFinish {
    /// How the run terminated.
    pub ended: RunEnd,
    /// Process exit code, if the process exited normally.
    pub exit_code: Option<i32>,
    /// When the process exited.
    pub finished_at: Timestamp,
    /// Worktree `HEAD` commit when the run finished, if recorded.
    pub commit: Option<CommitSha>,
}

/// Lifecycle state of an agent CLI [`Run`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum RunState {
    /// The agent CLI process is currently running.
    Running,
    /// The agent CLI finished its turn and the runner is executing `make gate`.
    WaitingOnGate,
    /// The invocation has finished.
    Finished(RunFinish),
}

/// One invocation (attempt) of an agent CLI inside a [`Member`] task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    /// Unique run identifier (`run-YYYYMMDD-HHMMSS-xxxx`).
    pub run_id: RunId,
    /// 1-based attempt number within the task (`1`, `2`, ...).
    pub run_number: NonZeroU32,
    /// When this invocation started.
    pub started_at: Timestamp,
    /// Current state of this invocation.
    pub state: RunState,
}

impl Run {
    /// Whether this invocation is still active (`Running` or `WaitingOnGate`).
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self.state, RunState::Running | RunState::WaitingOnGate)
    }

    /// Completion details if the invocation has finished.
    #[must_use]
    pub const fn finish(&self) -> Option<&RunFinish> {
        match &self.state {
            RunState::Finished(finish) => Some(finish),
            RunState::Running | RunState::WaitingOnGate => None,
        }
    }
}

/// How an answer to a clarification question round reached the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerSource {
    /// Submitted through `agent answer`.
    Cli,
    /// Detected by watching `questions-<round>.md` for `**ANSWERED**`.
    FileWatch,
    /// Submitted through the web dashboard (`agent-web`).
    Web,
}

/// Recorded human answers for a [`Question`] round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedAnswer {
    /// Map from question number (`1`, `2`, ...) to its [`Verdict`].
    pub verdicts: BTreeMap<NonZeroU32, Verdict>,
    /// Channel through which the answers arrived.
    pub source: AnswerSource,
    /// When the answers were recorded.
    pub answered_at: Timestamp,
}

/// A clarification round opened by a [`Member`] task when it reports `STATUS: QUESTIONS`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// 1-based clarification round number (`questions-1.md`, `questions-2.md`, ...).
    pub round: NonZeroU32,
    /// When the task entered `Questions` status for this round.
    pub asked_at: Timestamp,
    /// Parsed question blocks from `questions-<round>.md`.
    pub questions: Vec<QuestionItem>,
    /// Recorded answers once the human has answered this round.
    pub answer: Option<RecordedAnswer>,
}

impl Question {
    /// Whether this clarification round has been answered.
    #[must_use]
    pub const fn is_answered(&self) -> bool {
        self.answer.is_some()
    }

    /// Whether this clarification round is still waiting for answers.
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        self.answer.is_none()
    }
}

/// One agent task (alone or inside a [`Team`]) and its worktree, runs, questions, reviews, and gate result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    /// Unique task identifier.
    pub task_id: TaskId,
    /// Team this task belongs to, or `None` for a standalone task.
    pub team_id: Option<TeamId>,
    /// Repository this task works on.
    pub repo: RepoName,
    /// Short topic or assignment title.
    pub topic: String,
    /// Agent or investigator identity assigned to this task.
    pub participant: Participant,
    /// Agent CLI kind (`Claude`, `Codex`, `Gemini`, `Opencode`).
    pub cli_kind: CliKind,
    /// Optional model override for the agent CLI.
    pub model: Option<ModelName>,
    /// Role of this task (`Solo`, `Coordinator`, `Member`, `Worker`, `Reviewer`).
    pub role: Role,
    /// Current lifecycle status of the task.
    pub status: TaskStatus,
    /// Git branch checked out in the task's worktree.
    pub branch: BranchName,
    /// Immutable base commit from which `branch` was created.
    pub base_commit: CommitSha,
    /// Latest commit observed at `HEAD` in the task's worktree.
    pub head_commit: Option<CommitSha>,
    /// Absolute path to the task's git worktree.
    pub worktree_path: AbsolutePath,
    /// Plan item this task is working on, if part of a feature plan.
    pub item_id: Option<ItemId>,
    /// When the task was created.
    pub created_at: Timestamp,
    /// When the task record was last updated.
    pub updated_at: Timestamp,
    /// Active pause metadata when `status` is `Paused`.
    pub pause: Option<PauseState>,
    /// Pull request associated with this task, if any.
    pub pull_request: Option<PullRequest>,
    /// Chronological list of CLI runs for this task.
    pub runs: Vec<Run>,
    /// Clarification question rounds asked by this task.
    pub questions: Vec<Question>,
    /// Reviews performed by or recorded on this task.
    pub reviews: Vec<Review>,
    /// Most recent gate result recorded for this task's worktree.
    pub last_gate: Option<GateResult>,
}

impl Member {
    /// Validates that `participant` is allowed to take `role`.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::InvalidMemberRole`] if `participant`'s tier does not permit `role`.
    pub fn validate(&self) -> Result<(), StateError> {
        if !self.participant.allows_role(self.role) {
            return Err(StateError::InvalidMemberRole {
                task_id: self.task_id.clone(),
                tier: self.participant.tier(),
                role: self.role,
            });
        }
        Ok(())
    }
}

/// Summary of a feature section from a [`crate::task_list::TaskList`] stored in [`Team`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureSpec {
    /// Feature number (`1`, `2`, ...).
    pub number: u32,
    /// Feature title.
    pub title: String,
    /// Feature numbers this feature depends on.
    pub depends_on: Vec<u32>,
}

/// A coordinated team working through a feature plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawTeam", into = "RawTeam")]
pub struct Team {
    /// Unique team identifier.
    pub team_id: TeamId,
    /// Repository the team works on.
    pub repo: RepoName,
    /// Team topic or plan title.
    pub topic: String,
    /// Current lifecycle status of the team.
    pub status: TeamStatus,
    /// Whether approved pull requests merge automatically or wait for human merge.
    pub merge_mode: MergeMode,
    /// Team coordinator (`Normal` or `Investigator`).
    pub coordinator: Participant,
    /// When the team was created.
    pub created_at: Timestamp,
    /// When the team was last updated.
    pub updated_at: Timestamp,
    /// Active pause state when the team is paused.
    pub pause: Option<PauseState>,
    /// Features in the team's plan, in plan order.
    pub features: Vec<FeatureSpec>,
    /// Progress of each plan item, keyed by [`ItemId`].
    pub items: BTreeMap<ItemId, ItemProgress>,
    /// Member task IDs belonging to this team.
    pub member_tasks: Vec<TaskId>,
}

/// Wire representation of [`Team`] with `items` as a sequence so duplicate item IDs are detected.
#[derive(Serialize, Deserialize)]
struct RawTeam {
    /// Unique team identifier.
    team_id: TeamId,
    /// Repository the team works on.
    repo: RepoName,
    /// Team topic or plan title.
    topic: String,
    /// Current lifecycle status of the team.
    status: TeamStatus,
    /// Merge mode (`autopilot` or `no_merge`).
    merge_mode: MergeMode,
    /// Team coordinator.
    coordinator: Participant,
    /// When the team was created.
    created_at: Timestamp,
    /// When the team was last updated.
    updated_at: Timestamp,
    /// Active pause state.
    pause: Option<PauseState>,
    /// Features in the team's plan.
    features: Vec<FeatureSpec>,
    /// Plan items in order.
    items: Vec<ItemProgress>,
    /// Member task IDs belonging to this team.
    member_tasks: Vec<TaskId>,
}

impl TryFrom<RawTeam> for Team {
    type Error = StateError;

    fn try_from(raw: RawTeam) -> Result<Self, Self::Error> {
        let mut items = BTreeMap::new();
        for item in raw.items {
            let item_id = item.item_id;
            if items.insert(item_id, item).is_some() {
                return Err(StateError::DuplicateItem {
                    team_id: raw.team_id,
                    item_id,
                });
            }
        }
        Ok(Self {
            team_id: raw.team_id,
            repo: raw.repo,
            topic: raw.topic,
            status: raw.status,
            merge_mode: raw.merge_mode,
            coordinator: raw.coordinator,
            created_at: raw.created_at,
            updated_at: raw.updated_at,
            pause: raw.pause,
            features: raw.features,
            items,
            member_tasks: raw.member_tasks,
        })
    }
}

impl From<Team> for RawTeam {
    fn from(team: Team) -> Self {
        Self {
            team_id: team.team_id,
            repo: team.repo,
            topic: team.topic,
            status: team.status,
            merge_mode: team.merge_mode,
            coordinator: team.coordinator,
            created_at: team.created_at,
            updated_at: team.updated_at,
            pause: team.pause,
            features: team.features,
            items: team.items.into_values().collect(),
            member_tasks: team.member_tasks,
        }
    }
}

/// Why loading or validating a [`State`] failed.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// The JSON input exceeds [`MAX_STATE_BYTES`].
    #[error("state JSON is {size} bytes, exceeding the {max}-byte limit")]
    TooLarge {
        /// Actual byte length.
        size: usize,
        /// Maximum allowed byte length.
        max: usize,
    },
    /// The document's `schema_version` is not [`SCHEMA_VERSION`].
    #[error("unsupported state schema version {found}: this binary supports version {supported}")]
    UnsupportedVersion {
        /// Schema version found in the document.
        found: u32,
        /// Schema version supported by this binary.
        supported: u32,
    },
    /// The JSON document is syntactically invalid or fails field validation.
    #[error("invalid state JSON: {0}")]
    Parse(#[source] serde_json::Error),
    /// Two teams in the state share the same [`TeamId`].
    #[error("duplicate team {0} in state")]
    DuplicateTeam(TeamId),
    /// Two member tasks in the state share the same [`TaskId`].
    #[error("duplicate task {0} in state")]
    DuplicateTask(TaskId),
    /// Two items in a team share the same [`ItemId`].
    #[error("team {team_id} has duplicate item {item_id}")]
    DuplicateItem {
        /// Team containing the duplicate item.
        team_id: TeamId,
        /// Duplicate item identifier.
        item_id: ItemId,
    },
    /// A team lists a member task ID that does not exist in `tasks`.
    #[error("team {team_id} references unknown member task {task_id}")]
    UnknownMemberTask {
        /// Team referencing the missing task.
        team_id: TeamId,
        /// Missing task identifier.
        task_id: TaskId,
    },
    /// A [`Member`] has a [`Participant`] tier that does not permit its [`Role`].
    #[error("task {task_id} has participant tier {tier} which cannot take role {role}")]
    InvalidMemberRole {
        /// Task with mismatched participant tier and role.
        task_id: TaskId,
        /// Tier from [`Participant`].
        tier: Tier,
        /// Role stored on the [`Member`].
        role: Role,
    },
}

/// Top-level persisted daemon state (`state.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawState", into = "RawState")]
pub struct State {
    /// Schema version of this state snapshot (always [`SCHEMA_VERSION`]).
    schema_version: u32,
    /// Teams keyed by [`TeamId`].
    teams: BTreeMap<TeamId, Team>,
    /// Member tasks keyed by [`TaskId`].
    tasks: BTreeMap<TaskId, Member>,
}

/// Wire representation of [`State`].
#[derive(Serialize, Deserialize)]
struct RawState {
    /// Schema version field in JSON.
    schema_version: u32,
    /// Teams in sorted [`TeamId`] order.
    teams: Vec<Team>,
    /// Member tasks in sorted [`TaskId`] order.
    tasks: Vec<Member>,
}

/// Minimal JSON header inspected before full [`State`] deserialization so future schema versions with altered
/// inner fields are still rejected with [`StateError::UnsupportedVersion`].
#[derive(Deserialize)]
struct StateVersionHeader {
    /// Schema version field in JSON.
    schema_version: u32,
}

impl TryFrom<RawState> for State {
    type Error = StateError;

    fn try_from(raw: RawState) -> Result<Self, Self::Error> {
        if raw.schema_version != SCHEMA_VERSION {
            return Err(StateError::UnsupportedVersion {
                found: raw.schema_version,
                supported: SCHEMA_VERSION,
            });
        }
        Self::from_teams_and_tasks(raw.teams, raw.tasks)
    }
}

impl From<State> for RawState {
    fn from(state: State) -> Self {
        Self {
            schema_version: state.schema_version,
            teams: state.teams.into_values().collect(),
            tasks: state.tasks.into_values().collect(),
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl State {
    /// Creates an empty [`State`] at [`SCHEMA_VERSION`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            teams: BTreeMap::new(),
            tasks: BTreeMap::new(),
        }
    }

    /// Builds a [`State`] from collections of teams and tasks, checking all cross-record invariants.
    ///
    /// # Errors
    ///
    /// Returns [`StateError`] if any team or task ID is duplicated, a member's participant tier and role disagree,
    /// or a team references an unknown member task.
    pub fn from_teams_and_tasks(teams: Vec<Team>, tasks: Vec<Member>) -> Result<Self, StateError> {
        let mut state = Self::new();
        for member in tasks {
            state.insert_task(member)?;
        }
        for team in teams {
            state.insert_team(team)?;
        }
        Ok(state)
    }

    /// Schema version of this state snapshot (always [`SCHEMA_VERSION`]).
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// All teams, keyed by [`TeamId`].
    #[must_use]
    pub const fn teams(&self) -> &BTreeMap<TeamId, Team> {
        &self.teams
    }

    /// All member tasks, keyed by [`TaskId`].
    #[must_use]
    pub const fn tasks(&self) -> &BTreeMap<TaskId, Member> {
        &self.tasks
    }

    /// Looks up a team by its [`TeamId`].
    #[must_use]
    pub fn team(&self, team_id: &TeamId) -> Option<&Team> {
        self.teams.get(team_id)
    }

    /// Looks up a member task by its [`TaskId`].
    #[must_use]
    pub fn task(&self, task_id: &TaskId) -> Option<&Member> {
        self.tasks.get(task_id)
    }

    /// Inserts a new [`Member`] task after validating its participant tier and role.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::DuplicateTask`] if `member.task_id` already exists, or
    /// [`StateError::InvalidMemberRole`] if its participant tier does not permit its role.
    pub fn insert_task(&mut self, member: Member) -> Result<(), StateError> {
        member.validate()?;
        if self.tasks.contains_key(&member.task_id) {
            return Err(StateError::DuplicateTask(member.task_id));
        }
        self.tasks.insert(member.task_id.clone(), member);
        Ok(())
    }

    /// Inserts a new [`Team`] after checking that all its `member_tasks` exist in [`Self::tasks`].
    ///
    /// # Errors
    ///
    /// Returns [`StateError::DuplicateTeam`] if `team.team_id` already exists, or
    /// [`StateError::UnknownMemberTask`] if any referenced member task is missing.
    pub fn insert_team(&mut self, team: Team) -> Result<(), StateError> {
        if self.teams.contains_key(&team.team_id) {
            return Err(StateError::DuplicateTeam(team.team_id));
        }
        for task_id in &team.member_tasks {
            if !self.tasks.contains_key(task_id) {
                return Err(StateError::UnknownMemberTask {
                    team_id: team.team_id,
                    task_id: task_id.clone(),
                });
            }
        }
        self.teams.insert(team.team_id.clone(), team);
        Ok(())
    }

    /// Deserializes a [`State`] from a JSON string, checking size and `schema_version` first.
    ///
    /// # Errors
    ///
    /// Returns [`StateError::TooLarge`] if `json` exceeds [`MAX_STATE_BYTES`],
    /// [`StateError::UnsupportedVersion`] if `schema_version` is not [`SCHEMA_VERSION`],
    /// [`StateError::Parse`] if the JSON is malformed, or invariant errors if records conflict.
    pub fn from_json_str(json: &str) -> Result<Self, StateError> {
        if json.len() > MAX_STATE_BYTES {
            return Err(StateError::TooLarge {
                size: json.len(),
                max: MAX_STATE_BYTES,
            });
        }
        let header: StateVersionHeader = serde_json::from_str(json).map_err(StateError::Parse)?;
        if header.schema_version != SCHEMA_VERSION {
            return Err(StateError::UnsupportedVersion {
                found: header.schema_version,
                supported: SCHEMA_VERSION,
            });
        }
        let raw: RawState = serde_json::from_str(json).map_err(StateError::Parse)?;
        Self::try_from(raw)
    }

    /// Serializes this [`State`] to compact JSON.
    #[must_use]
    pub fn to_json_string(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Serializes this [`State`] to pretty-printed JSON.
    #[must_use]
    pub fn to_json_string_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    fn ts(text: &str) -> Timestamp {
        text.parse().expect("valid test timestamp")
    }

    fn sha(text: &str) -> CommitSha {
        text.parse().expect("valid test commit sha")
    }

    fn nz32(n: u32) -> NonZeroU32 {
        NonZeroU32::new(n).expect("non-zero u32")
    }

    fn nz64(n: u64) -> NonZeroU64 {
        NonZeroU64::new(n).expect("non-zero u64")
    }

    fn sample_gate_result() -> GateResult {
        GateResult {
            outcome: GateOutcome::Passed,
            exit_code: Some(0),
            commit: sha("26b65de7e4076596768ca96b48a7f94799c93874"),
            tree_state: TreeState::Clean,
            started_at: ts("2026-04-06T17:15:00Z"),
            finished_at: ts("2026-04-06T17:16:30Z"),
            failed_step: None,
            summary: Some("all 9 gate steps passed".to_owned()),
        }
    }

    fn sample_review() -> Review {
        Review {
            reviewer: "inv-opus".parse().expect("valid investigator"),
            task_id: "task-20260406-171500-r3v1".parse().expect("valid task id"),
            verdict: ReviewVerdict::Approve,
            commit: sha("26b65de7e4076596768ca96b48a7f94799c93874"),
            recorded_at: ts("2026-04-06T17:18:00Z"),
            summary: Some("LGTM".to_owned()),
        }
    }

    fn sample_member() -> Member {
        let mut verdicts = BTreeMap::new();
        verdicts.insert(nz32(1), Verdict::AcceptRecommendation);

        Member {
            task_id: "task-20260406-171000-k3x9".parse().expect("valid task id"),
            team_id: Some("team-20260406-170900-a1b2".parse().expect("valid team id")),
            repo: "team-agents".parse().expect("valid repo"),
            topic: "Round rust-agent/2.2".to_owned(),
            participant: Participant::Normal("codex".parse().expect("valid agent")),
            cli_kind: CliKind::Codex,
            model: Some("gpt-5".parse().expect("valid model")),
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
            updated_at: ts("2026-04-06T17:18:00Z"),
            pause: Some(PauseState {
                kind: PauseKind::RateLimited,
                reason: Some("HTTP 429 from upstream".to_owned()),
                paused_at: ts("2026-04-06T17:12:00Z"),
                resume_at: Some(ts("2026-04-06T17:17:00Z")),
            }),
            pull_request: Some(PullRequest {
                number: nz64(82),
                head_branch: "agents/codex/task-20260406-171000-k3x9".parse().expect("valid branch"),
                head_commit: sha("26b65de7e4076596768ca96b48a7f94799c93874"),
                state: PullRequestState::Open,
            }),
            runs: vec![
                Run {
                    run_id: "run-20260406-171005-r001".parse().expect("valid run id"),
                    run_number: nz32(1),
                    started_at: ts("2026-04-06T17:10:05Z"),
                    state: RunState::Finished(RunFinish {
                        ended: RunEnd::StatusLine(RunOutcome::Questions),
                        exit_code: Some(0),
                        finished_at: ts("2026-04-06T17:11:00Z"),
                        commit: Some(sha("1111111111111111111111111111111111111111")),
                    }),
                },
                Run {
                    run_id: "run-20260406-171400-r002".parse().expect("valid run id"),
                    run_number: nz32(2),
                    started_at: ts("2026-04-06T17:14:00Z"),
                    state: RunState::Running,
                },
            ],
            questions: vec![Question {
                round: nz32(1),
                asked_at: ts("2026-04-06T17:11:00Z"),
                questions: vec![
                    QuestionItem::new(
                        nz32(1),
                        "Wire format?",
                        "a) JSON lines\nb) HTTP",
                        "a) — local unix socket",
                        Some(Verdict::AcceptRecommendation),
                    )
                    .expect("valid question item"),
                ],
                answer: Some(RecordedAnswer {
                    verdicts,
                    source: AnswerSource::Cli,
                    answered_at: ts("2026-04-06T17:13:30Z"),
                }),
            }],
            reviews: vec![sample_review()],
            last_gate: Some(sample_gate_result()),
        }
    }

    fn sample_team(member_task_id: TaskId) -> Team {
        let item_id: ItemId = "2.2".parse().expect("valid item id");
        let mut items = BTreeMap::new();
        items.insert(
            item_id,
            ItemProgress {
                item_id,
                title: "State, socket protocol and questions files".to_owned(),
                feature_number: 2,
                status: ItemStatus::InProgress,
                round: 1,
                assigned_task: Some(member_task_id.clone()),
                pull_request: Some(PullRequest {
                    number: nz64(82),
                    head_branch: "agents/codex/task-20260406-171000-k3x9".parse().expect("valid branch"),
                    head_commit: sha("26b65de7e4076596768ca96b48a7f94799c93874"),
                    state: PullRequestState::Open,
                }),
                gate_result: Some(sample_gate_result()),
                reviews: vec![sample_review()],
            },
        );

        Team {
            team_id: "team-20260406-170900-a1b2".parse().expect("valid team id"),
            repo: "team-agents".parse().expect("valid repo"),
            topic: "Rewrite bin/agent in Rust".to_owned(),
            status: TeamStatus::Working,
            merge_mode: MergeMode::Autopilot,
            coordinator: Participant::Normal("codex".parse().expect("valid lead")),
            created_at: ts("2026-04-06T17:09:00Z"),
            updated_at: ts("2026-04-06T17:18:00Z"),
            pause: None,
            features: vec![FeatureSpec {
                number: 2,
                title: "Configuration, task list and state".to_owned(),
                depends_on: vec![1],
            }],
            items,
            member_tasks: vec![member_task_id],
        }
    }

    #[test]
    fn state_saved_and_loaded_back_is_equal() {
        let member = sample_member();
        let team = sample_team(member.task_id.clone());
        let state = State::from_teams_and_tasks(vec![team.clone()], vec![member.clone()]).expect("valid sample state");

        assert_eq!(state.schema_version(), SCHEMA_VERSION);
        assert_eq!(state.team(&team.team_id), Some(&team));
        assert_eq!(state.task(&member.task_id), Some(&member));

        let compact = state.to_json_string();
        let loaded_compact = State::from_json_str(&compact).expect("compact state round trip");
        assert_eq!(loaded_compact, state);

        let pretty = state.to_json_string_pretty();
        let loaded_pretty = State::from_json_str(&pretty).expect("pretty state round trip");
        assert_eq!(loaded_pretty, state);

        let via_serde: State = serde_json::from_str(&pretty).expect("serde_json round trip");
        assert_eq!(via_serde, state);
    }

    #[test]
    fn state_with_wrong_schema_version_is_refused_with_typed_error() {
        for bad_version in [0_u32, 2, 99] {
            let payload = format!("{{\"schema_version\":{bad_version},\"teams\":[],\"tasks\":[]}}");
            let err = State::from_json_str(&payload).unwrap_err();
            assert!(
                matches!(
                    err,
                    StateError::UnsupportedVersion {
                        found,
                        supported: SCHEMA_VERSION,
                    } if found == bad_version
                ),
                "unexpected error for version {bad_version}: {err:?}"
            );

            let serde_err = serde_json::from_str::<State>(&payload).unwrap_err();
            assert_eq!(serde_err.classify(), serde_json::error::Category::Data);
        }
    }

    #[test]
    fn state_with_wrong_schema_version_and_changed_inner_shape_reports_unsupported_version() {
        let future_payload = r#"{"schema_version":2,"teams":"changed-in-v2","tasks":42}"#;
        let err = State::from_json_str(future_payload).unwrap_err();
        assert!(matches!(
            err,
            StateError::UnsupportedVersion {
                found: 2,
                supported: SCHEMA_VERSION,
            }
        ));
    }

    #[test]
    fn state_exceeding_max_size_is_refused() {
        let oversized = " ".repeat(MAX_STATE_BYTES + 1);
        let err = State::from_json_str(&oversized).unwrap_err();
        assert!(matches!(
            err,
            StateError::TooLarge {
                size,
                max: MAX_STATE_BYTES,
            } if size == MAX_STATE_BYTES + 1
        ));
    }

    #[test]
    fn duplicate_team_task_or_item_is_refused() {
        let member = sample_member();
        let team = sample_team(member.task_id.clone());

        let dup_task_err = State::from_teams_and_tasks(vec![], vec![member.clone(), member.clone()]).unwrap_err();
        assert!(matches!(dup_task_err, StateError::DuplicateTask(id) if id == member.task_id));

        let dup_team_err = State::from_teams_and_tasks(vec![team.clone(), team.clone()], vec![member]).unwrap_err();
        assert!(matches!(dup_team_err, StateError::DuplicateTeam(id) if id == team.team_id));

        let dup_item_json = r#"{
            "team_id": "team-20260406-170900-a1b2",
            "repo": "team-agents",
            "topic": "Plan",
            "status": "working",
            "merge_mode": "autopilot",
            "coordinator": { "tier": "normal", "name": "codex" },
            "created_at": "2026-04-06T17:09:00Z",
            "updated_at": "2026-04-06T17:09:00Z",
            "pause": null,
            "features": [],
            "items": [
                {
                    "item_id": "2.2",
                    "title": "First",
                    "feature_number": 2,
                    "status": "pending",
                    "round": 1,
                    "assigned_task": null,
                    "pull_request": null,
                    "gate_result": null,
                    "reviews": []
                },
                {
                    "item_id": "2.2",
                    "title": "Duplicate",
                    "feature_number": 2,
                    "status": "pending",
                    "round": 1,
                    "assigned_task": null,
                    "pull_request": null,
                    "gate_result": null,
                    "reviews": []
                }
            ],
            "member_tasks": []
        }"#;
        let item_err = serde_json::from_str::<Team>(dup_item_json).unwrap_err();
        assert_eq!(item_err.classify(), serde_json::error::Category::Data);
    }

    #[test]
    fn team_referencing_unknown_member_task_is_refused() {
        let member = sample_member();
        let team = sample_team(member.task_id.clone());
        let err = State::from_teams_and_tasks(vec![team.clone()], vec![]).unwrap_err();
        assert!(matches!(
            err,
            StateError::UnknownMemberTask { team_id, task_id }
                if team_id == team.team_id && task_id == member.task_id
        ));
    }

    #[test]
    fn member_with_mismatched_participant_tier_and_role_is_refused() {
        let mut member = sample_member();
        member.role = Role::Reviewer;
        let err = State::from_teams_and_tasks(vec![], vec![member.clone()]).unwrap_err();
        assert!(matches!(
            err,
            StateError::InvalidMemberRole {
                task_id,
                tier: Tier::Normal,
                role: Role::Reviewer,
            } if task_id == member.task_id
        ));
    }

    #[test]
    fn timestamp_and_commit_sha_validate_inputs() {
        let valid_ts = ts("2026-04-06T17:19:45Z");
        let unix = valid_ts.unix_timestamp();
        assert_eq!(Timestamp::from_unix_timestamp(unix), Ok(valid_ts));
        assert!(matches!(
            "2026-04-06 17:19:45Z".parse::<Timestamp>(),
            Err(InvalidTimestamp::Format(_))
        ));
        let bad_cal = "2026-02-30T17:19:45Z".parse::<Timestamp>().unwrap_err();
        assert!(matches!(bad_cal, InvalidTimestamp::OutOfRange { .. }));
        assert!(bad_cal.source().is_some());

        let bad_unix = Timestamp::from_unix_timestamp(i64::MAX).unwrap_err();
        assert!(matches!(bad_unix, InvalidTimestamp::UnixTimestampOutOfRange { .. }));
        assert!(bad_unix.source().is_some());

        assert_eq!("".parse::<CommitSha>(), Err(InvalidCommitSha::Empty));
        assert!(matches!(
            "abc123".parse::<CommitSha>(),
            Err(InvalidCommitSha::TooShort { .. })
        ));
        assert!(matches!(
            "a".repeat(65).parse::<CommitSha>(),
            Err(InvalidCommitSha::TooLong { .. })
        ));
        assert!(matches!(
            "26B65DE".parse::<CommitSha>(),
            Err(InvalidCommitSha::NotLowercaseHex { ch: 'B', .. })
        ));
    }

    #[test]
    fn state_error_parse_preserves_error_source() {
        let err = State::from_json_str("{not-json").unwrap_err();
        assert!(matches!(err, StateError::Parse(_)));
        assert!(err.source().is_some());
    }
}
