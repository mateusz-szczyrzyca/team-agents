//! The records of the history: what happened to a team, a round and a run, as typed values.
//!
//! The history is an append-only database that the daemon writes and `agent log` reads (`docs/history.md`). This module
//! is its vocabulary and does no I/O: a [`Record`] is what the writer is given, and the reading API gives the same
//! types back. A record is made of values that are valid already (IDs, paths, statuses), so the database never holds a
//! state the types refuse.
//!
//! Every free text of a record is cut at [`MaxTextBytes`] by [`cut_text`] before it is written; the record says which of
//! its fields were cut ([`Cut`]).

use std::collections::BTreeSet;
use std::fmt;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use crate::cli::ActivityEvent;
use crate::config::{AbsolutePath, Config, EnvName, ModelName};
use crate::decide::checks::FileChange;
use crate::decide::team::TeamRequest;
use crate::ids::{AgentName, BranchName, ItemId, RoundId, RunId, TaskId, TeamId};
use crate::kinds::{CliKind, Keyword, Role, Tier, UnknownKeyword};
use crate::state::{CommitId, MergeMode, PrNumber, RunEnd, SessionId, Timestamp};

/// The longest text of one field the history keeps, in bytes, unless `[history] max_text_bytes` says otherwise: a prompt
/// of a long task list fits.
pub const DEFAULT_MAX_TEXT_BYTES: u32 = 1 << 20;

/// The largest value `[history] max_text_bytes` takes: far below the 1 GB that SQLite refuses.
pub const LARGEST_MAX_TEXT_BYTES: u32 = 256 << 20;

/// The longest text of one field the history keeps, in bytes: `1` to [`LARGEST_MAX_TEXT_BYTES`].
///
/// A longer text is cut at a character boundary at or below this many bytes and the record says it was cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct MaxTextBytes(NonZeroU32);

/// A number that is not a [`MaxTextBytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{value} bytes is not a limit of text: it is 1 to {LARGEST_MAX_TEXT_BYTES}")]
pub struct InvalidMaxTextBytes {
    /// The refused number.
    pub value: u32,
}

impl MaxTextBytes {
    /// The limit of `bytes`.
    ///
    /// # Errors
    /// [`InvalidMaxTextBytes`] for `0` and for more than [`LARGEST_MAX_TEXT_BYTES`].
    pub fn new(bytes: u32) -> Result<Self, InvalidMaxTextBytes> {
        match NonZeroU32::new(bytes) {
            Some(bytes) if bytes.get() <= LARGEST_MAX_TEXT_BYTES => Ok(Self(bytes)),
            Some(_) | None => Err(InvalidMaxTextBytes { value: bytes }),
        }
    }

    /// The limit in bytes.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0.get()
    }
}

impl Default for MaxTextBytes {
    fn default() -> Self {
        Self(NonZeroU32::new(DEFAULT_MAX_TEXT_BYTES).unwrap_or(NonZeroU32::MIN))
    }
}

impl TryFrom<u32> for MaxTextBytes {
    type Error = InvalidMaxTextBytes;

    fn try_from(bytes: u32) -> Result<Self, Self::Error> {
        Self::new(bytes)
    }
}

impl From<MaxTextBytes> for u32 {
    fn from(limit: MaxTextBytes) -> Self {
        limit.get()
    }
}

/// A text as the history keeps it: whole, or cut at the limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    /// The text kept: a prefix of the original that ends at a character boundary.
    pub text: String,
    /// Whether the original was longer and the rest is gone.
    pub was_cut: bool,
}

/// Keeps at most `limit` bytes of `text`: all of it when it fits, else the longest prefix that ends at a character
/// boundary within the limit.
#[must_use]
pub fn cut_text(_text: &str, _limit: MaxTextBytes) -> Kept {
    Kept {
        text: String::new(),
        was_cut: false,
    }
}

/// The fields of one record whose text was cut, by the name of the field.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cut(BTreeSet<&'static str>);

impl Cut {
    /// Keeps `text` within `limit` and notes that `field` was cut when it had to be.
    pub fn keep(&mut self, field: &'static str, text: &str, limit: MaxTextBytes) -> String {
        let kept = cut_text(text, limit);
        if kept.was_cut {
            self.0.insert(field);
        }
        kept.text
    }

    /// Whether `field` was cut.
    #[must_use]
    pub fn has(&self, field: &str) -> bool {
        self.0.contains(field)
    }

    /// Whether nothing was cut.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The names of the fields cut, in alphabetical order.
    pub fn fields(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.0.iter().copied()
    }
}

/// How a participant of a team got its role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HowChosen {
    /// Named on the command line, and nothing else decided its role: a solo agent, a member.
    Named,
    /// Named as the coordinator by `--coordinator`.
    NamedByFlag,
    /// Named as the coordinator by the `Coordinator:` line of the task list.
    NamedByLine,
    /// Drawn at random as the coordinator among these, in the order named.
    Drawn {
        /// The candidates: the investigators of the team, or when it has none, every agent that takes items.
        among: Vec<AgentName>,
    },
    /// A worker: it serves requests in the order of the configuration.
    ConfiguredOrder,
}

/// The keyword of a [`HowChosen`], as the database keeps it.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantArray,
    strum::VariantNames,
)]
#[strum(serialize_all = "snake_case", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<HowKind>)]
pub enum HowKind {
    /// [`HowChosen::Named`].
    Named,
    /// [`HowChosen::NamedByFlag`].
    NamedByFlag,
    /// [`HowChosen::NamedByLine`].
    NamedByLine,
    /// [`HowChosen::Drawn`].
    Drawn,
    /// [`HowChosen::ConfiguredOrder`].
    ConfiguredOrder,
}

impl Keyword for HowKind {
    const WHAT: &'static str = "way a role was got";
}

impl HowChosen {
    /// The keyword of the way.
    #[must_use]
    pub const fn kind(&self) -> HowKind {
        match self {
            Self::Named => HowKind::Named,
            Self::NamedByFlag => HowKind::NamedByFlag,
            Self::NamedByLine => HowKind::NamedByLine,
            Self::Drawn { .. } => HowKind::Drawn,
            Self::ConfiguredOrder => HowKind::ConfiguredOrder,
        }
    }
}

/// One participant of a team's plan: who, in which role, and how it got it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEntry {
    /// The agent.
    pub agent: AgentName,
    /// Its place on the command line, counted from 1.
    pub position: NonZeroU32,
    /// Its role in the team.
    pub role: Role,
    /// How it got the role.
    pub how: HowChosen,
}

/// A team starting: what was asked and how it was formed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamStart {
    /// The team.
    pub team: TeamId,
    /// When it started.
    pub at: Timestamp,
    /// The repository, as `[repos.NAME]` of the configuration.
    pub repo: String,
    /// The branch the team's branches start from.
    pub base_branch: BranchName,
    /// What happens to an approved pull request.
    pub merge: MergeMode,
    /// The task list as it was given.
    pub task_list: String,
    /// Every participant with its role and how it got it, in the order of the command line.
    pub plan: Vec<PlanEntry>,
}

/// An item of a task list in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemText {
    /// The item's number.
    pub item: ItemId,
    /// The rest of the item's line.
    pub title: String,
    /// What to do.
    pub description: String,
    /// The reviewers' standard, when the item has one.
    pub expected_result: Option<String>,
    /// What anyone can run to see that the item works, when the item has it.
    pub checks: Option<String>,
}

/// What a round is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoundSubject {
    /// One item of the task list.
    Item(ItemText),
    /// The whole work of a list without items: its brief.
    Brief(String),
}

/// A round opening.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundOpening {
    /// The round.
    pub round: RoundId,
    /// Its team.
    pub team: TeamId,
    /// When it opened.
    pub at: Timestamp,
    /// What it is about.
    pub subject: RoundSubject,
}

/// Who runs: the agent of a run with the tier, CLI and model it had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentIdentity {
    /// The agent's name.
    pub name: String,
    /// The tier of the configuration it is in.
    pub tier: Tier,
    /// The model CLI it runs.
    pub cli: CliKind,
    /// The model, or `None` for the CLI's default.
    pub model: Option<ModelName>,
}

/// Why a run starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunWhy {
    /// The first run of a task.
    FirstRun,
    /// The task is given an item.
    Item {
        /// The item.
        item: ItemId,
    },
    /// A message or a board item woke the task.
    WakeUp,
    /// A worker serves a request.
    Request {
        /// The run that asked, when the state knows it.
        asked_by: Option<RunId>,
    },
    /// The reviewers asked for changes: the lead fixes.
    FixPass {
        /// Which fix pass of the item it is, counted from 1.
        number: NonZeroU32,
    },
    /// A reviewer reviews a pull request.
    Review {
        /// The pull request.
        pr: PrNumber,
        /// How many fix passes the pull request had when the review was asked: `0` for the first review.
        pass: u32,
    },
    /// The fix passes are used up: the lead puts the question to the human.
    AskHuman,
    /// A reviewer's report had no verdict: it is asked again.
    VerdictMissing,
    /// The checks before a review sent the round back, or put a question to the human.
    Checks,
    /// The human answered the task's questions.
    Answers,
    /// The human stopped the task and let it go on.
    ResumedByHuman,
    /// The task goes on after a restart of the daemon, a hang or a limit.
    Continuation,
}

/// The keyword of a [`RunWhy`], as the database keeps it.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    strum::Display,
    strum::EnumString,
    strum::IntoStaticStr,
    strum::VariantArray,
    strum::VariantNames,
)]
#[strum(serialize_all = "snake_case", parse_err_ty = UnknownKeyword, parse_err_fn = UnknownKeyword::of::<WhyKind>)]
pub enum WhyKind {
    /// [`RunWhy::FirstRun`].
    FirstRun,
    /// [`RunWhy::Item`].
    Item,
    /// [`RunWhy::WakeUp`].
    WakeUp,
    /// [`RunWhy::Request`].
    Request,
    /// [`RunWhy::FixPass`].
    FixPass,
    /// [`RunWhy::Review`].
    Review,
    /// [`RunWhy::AskHuman`].
    AskHuman,
    /// [`RunWhy::VerdictMissing`].
    VerdictMissing,
    /// [`RunWhy::Checks`].
    Checks,
    /// [`RunWhy::Answers`].
    Answers,
    /// [`RunWhy::ResumedByHuman`].
    ResumedByHuman,
    /// [`RunWhy::Continuation`].
    Continuation,
}

impl Keyword for WhyKind {
    const WHAT: &'static str = "reason a run starts";
}

impl RunWhy {
    /// The keyword of the reason.
    #[must_use]
    pub const fn kind(&self) -> WhyKind {
        match self {
            Self::FirstRun => WhyKind::FirstRun,
            Self::Item { .. } => WhyKind::Item,
            Self::WakeUp => WhyKind::WakeUp,
            Self::Request { .. } => WhyKind::Request,
            Self::FixPass { .. } => WhyKind::FixPass,
            Self::Review { .. } => WhyKind::Review,
            Self::AskHuman => WhyKind::AskHuman,
            Self::VerdictMissing => WhyKind::VerdictMissing,
            Self::Checks => WhyKind::Checks,
            Self::Answers => WhyKind::Answers,
            Self::ResumedByHuman => WhyKind::ResumedByHuman,
            Self::Continuation => WhyKind::Continuation,
        }
    }
}

/// A run starting, as the owner decided it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStart {
    /// The run.
    pub run: RunId,
    /// The team of its task.
    pub team: TeamId,
    /// Its task.
    pub task: TaskId,
    /// When it started.
    pub at: Timestamp,
    /// Who runs.
    pub agent: AgentIdentity,
    /// The role of the task.
    pub role: Role,
    /// The rounds the run works for, oldest first.
    pub rounds: Vec<RoundId>,
    /// Why it starts.
    pub why: RunWhy,
    /// The whole prompt.
    pub prompt: String,
    /// The worktree the CLI runs in.
    pub worktree: AbsolutePath,
    /// The branch the worktree is on.
    pub branch: BranchName,
    /// The session the run begins with, when it is known before the CLI starts (a resumed one, or a Claude Code
    /// session the harness names).
    pub session: Option<SessionId>,
}

/// How the engine started the process of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// The run.
    pub run: RunId,
    /// When the process started.
    pub at: Timestamp,
    /// The program, as it was run.
    pub program: String,
    /// Its arguments, in order: what the CLI saw.
    pub args: Vec<String>,
    /// The directory it ran in.
    pub working_dir: AbsolutePath,
    /// The commit the worktree was at when the run started; `None` when git could not say.
    pub head: Option<CommitId>,
    /// The names of the environment variables the daemon set for the process, sorted. Never their values.
    pub env_names: Vec<EnvName>,
    /// Whether the prompt travelled in a file because it was too long for the command line.
    pub prompt_in_file: bool,
}

/// How a run ended, as the owner recorded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunEndRecord {
    /// The run.
    pub run: RunId,
    /// When it ended.
    pub at: Timestamp,
    /// How it ended.
    pub end: RunEnd,
    /// The agent's final message, whole; `None` when the run ended without one.
    pub final_message: Option<String>,
}

/// One step of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The run.
    pub run: RunId,
    /// The step's number in the run, counted from 1.
    pub number: NonZeroU32,
    /// When it was seen.
    pub at: Timestamp,
    /// What it was.
    pub event: ActivityEvent,
}

/// One commit of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRecord {
    /// The commit.
    pub id: CommitId,
    /// The first line of its message.
    pub subject: String,
    /// When it was authored.
    pub authored_at: Timestamp,
}

/// What a run changed, from git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunChanges {
    /// The run.
    pub run: RunId,
    /// The commit the worktree was at when the run started.
    pub head_before: Option<CommitId>,
    /// The commit it was at when the run ended.
    pub head_after: Option<CommitId>,
    /// The commits between the two heads, oldest first.
    pub commits: Vec<CommitRecord>,
    /// The files that differ between the two heads.
    pub files: Vec<FileChange>,
    /// The files the run left changed in the worktree without committing them.
    pub uncommitted: Vec<FileChange>,
}

/// Everything the writer is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    /// A team starting.
    Team(Box<TeamStart>),
    /// A round opening.
    Round(Box<RoundOpening>),
    /// A run starting.
    Run(Box<RunStart>),
    /// The process of a run starting.
    Launch(Box<Launch>),
    /// A step of a run.
    Step(Box<Step>),
    /// A run ending.
    RunEnd(Box<RunEndRecord>),
    /// What a run changed.
    Changes(Box<RunChanges>),
}

/// Whether the history is being written, and what it lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryHealth {
    /// Whether records are being written.
    pub state: HistoryState,
    /// How many records were handed to the history and are not in it: the queue was full, the database could not be
    /// written, or there is no database.
    pub lost: u64,
}

/// Whether the history is being written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum HistoryState {
    /// The database is open and records are written to it.
    Writing,
    /// `[history] enabled = false`: nothing is recorded, and nothing is lost.
    Disabled,
    /// The database cannot be used: the daemon runs without a history.
    Unavailable {
        /// Why, in a line.
        reason: String,
    },
}

impl fmt::Display for HistoryHealth {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Ok(())
    }
}

/// Why a plan cannot be explained: it does not come from the request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExplainError {
    /// A participant of the crew is not among the agents named.
    #[error("{agent} is in the team but was not named on the command line")]
    NotNamed {
        /// The agent.
        agent: String,
    },
    /// A participant has a role a team is not formed with.
    #[error("{agent} has the role {role}, which a team is not formed with")]
    NotFormed {
        /// The agent.
        agent: String,
        /// Its role.
        role: Role,
    },
}

/// How each participant of `crew` — the people of a team in their roles, as the team was formed — got its role, given
/// the `request` and the configuration.
///
/// # Errors
/// [`ExplainError`] when the crew has someone the request did not name, or a reviewer.
pub fn explain_plan(
    _request: &TeamRequest<'_>,
    _config: &Config,
    _crew: &[(Role, String)],
) -> Result<Vec<PlanEntry>, ExplainError> {
    Ok(Vec::new())
}
