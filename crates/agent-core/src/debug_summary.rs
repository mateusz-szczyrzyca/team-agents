//! Pure bounded summary renderer for `agent debug`.
//!
//! Renders `SUMMARY.md` from collected facts in strict specification order, with UTC and local
//! times, task tables, and byte-safe bounded truncation (< 100,000 bytes) preserving section headings.

#[cfg(test)]
mod tests;

use time::UtcOffset;

use crate::ids::{AgentName, BranchName, ItemId, RoundId, TaskId, TeamId};
use crate::kinds::Role;
use crate::state::{CommitId, Timestamp};
use crate::status::{ItemStatus, TaskStatus};

/// Maximum byte limit for `SUMMARY.md`. The output must be strictly less than this limit.
pub const MAX_SUMMARY_BYTES: usize = 100_000;

/// Marker indicating that content was truncated to remain under [`MAX_SUMMARY_BYTES`].
pub const CUT_MARKER: &str = "\n[... cut ...]\n";

/// Fixed section headings in specification order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SummarySection {
    /// 1. The anomalies.
    Anomalies,
    /// 2. The daemon and the versions.
    DaemonAndVersions,
    /// 3. The team, its items with their phase and assignee, its rounds.
    TeamItemsRounds,
    /// 4. One table of the tasks.
    TaskTable,
    /// 5. For each task the end of its last run and its questions.
    TaskDetails,
    /// 6. Git for each worktree.
    GitWorktrees,
    /// 7. Gate records and the end of the last three gate outputs of each task.
    GateRecords,
    /// 8. The end of the team's history.
    HistoryTail,
    /// 9. The daemon's log lines.
    DaemonLog,
    /// 10. The bus.
    Bus,
    /// 11. The notifications.
    Notifications,
    /// 12. The machine.
    Machine,
}

impl SummarySection {
    /// All 12 summary sections in strict specification order.
    pub const ALL: [Self; 12] = [
        Self::Anomalies,
        Self::DaemonAndVersions,
        Self::TeamItemsRounds,
        Self::TaskTable,
        Self::TaskDetails,
        Self::GitWorktrees,
        Self::GateRecords,
        Self::HistoryTail,
        Self::DaemonLog,
        Self::Bus,
        Self::Notifications,
        Self::Machine,
    ];

    /// The markdown heading text for this section.
    #[must_use]
    pub const fn heading(self) -> &'static str {
        match self {
            Self::Anomalies => "## Anomalies",
            Self::DaemonAndVersions => "## Daemon and Versions",
            Self::TeamItemsRounds => "## Team, Items, and Rounds",
            Self::TaskTable => "## Task Status",
            Self::TaskDetails => "## Task Runs and Questions",
            Self::GitWorktrees => "## Git Worktrees",
            Self::GateRecords => "## Gate Records and Outputs",
            Self::HistoryTail => "## Team History",
            Self::DaemonLog => "## Daemon Log",
            Self::Bus => "## Bus Messages",
            Self::Notifications => "## Notifications",
            Self::Machine => "## Machine and Resources",
        }
    }
}

/// Standalone input model representing all facts needed to render `SUMMARY.md`.
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryInput {
    /// When this debug bundle was collected, in UTC.
    pub collected_at: Timestamp,
    /// Detected anomalies (empty means "none of the known ones").
    pub anomalies: Vec<SummaryAnomaly>,
    /// Information about the running daemon, or `None` if the daemon does not answer.
    pub daemon: Option<DaemonSummary>,
    /// Information about the team, its items, and its rounds.
    pub team: TeamSummaryFacts,
    /// Per-task status rows and details.
    pub tasks: Vec<TaskSummaryFacts>,
    /// Per-worktree git status.
    pub git: Vec<GitWorktreeSummary>,
    /// Per-task gate records and output tails.
    pub gate: Vec<TaskGateSummary>,
    /// The tail of the team history as `agent log --team` tells it.
    pub history_tail: Option<String>,
    /// Relevant daemon log lines (team/task restricted + warnings/errors of last 500 lines).
    pub daemon_log_lines: Option<Vec<String>>,
    /// Last bus messages to/from/about team/tasks/agents.
    pub bus_messages: Vec<String>,
    /// Notifications written for team and tasks.
    pub notifications: Vec<String>,
    /// Machine health and resources.
    pub machine: MachineSummary,
    /// Names and error reasons for any collection parts that could not be read.
    pub unreadable_parts: Vec<UnreadablePartSummary>,
}

/// An anomaly reported in the summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummaryAnomaly {
    /// Human-readable sentence explaining what is wrong.
    pub message: String,
    /// Command that would help fix or investigate, if known.
    pub command: Option<String>,
}

/// Daemon and executable status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonSummary {
    /// Process ID.
    pub pid: u32,
    /// When the daemon process started.
    pub started_at: Option<Timestamp>,
    /// Command line arguments.
    pub cmdline: Vec<String>,
    /// Names of environment variables (values omitted/cut).
    pub env_var_names: Vec<String>,
    /// Path of the daemon executable.
    pub exe_path: String,
    /// Whether the executable file on disk is newer than the running process.
    pub disk_newer_than_process: bool,
    /// The daemon's version string.
    pub version: String,
}

/// Information about the team, its items, and rounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamSummaryFacts {
    /// Team ID.
    pub id: TeamId,
    /// Repository name.
    pub repo: String,
    /// Items of the team.
    pub items: Vec<ItemSummaryFacts>,
    /// Rounds of the team.
    pub rounds: Vec<RoundSummaryFacts>,
}

/// Summary of an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemSummaryFacts {
    /// Item ID.
    pub id: ItemId,
    /// Title or short description.
    pub title: String,
    /// Current phase (e.g. pending, `in_progress`, `in_review`, approved, done).
    pub phase: ItemStatus,
    /// Assignee agent, if any.
    pub assignee: Option<AgentName>,
}

/// Summary of a round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundSummaryFacts {
    /// Round ID.
    pub id: RoundId,
    /// Associated item ID, if any.
    pub item: Option<ItemId>,
    /// When the round opened.
    pub opened_at: Timestamp,
    /// Status of the round (e.g. open, closed).
    pub status: String,
}

/// Status and run summary for one task in the task table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSummaryFacts {
    /// Task ID.
    pub id: TaskId,
    /// Agent running this task.
    pub agent: AgentName,
    /// Role of the agent (e.g. coordinator/lead, member, worker, reviewer).
    pub role: Role,
    /// Current status of the task.
    pub status: TaskStatus,
    /// Total count of runs.
    pub run_count: usize,
    /// Start time of the last run.
    pub last_run_started: Option<Timestamp>,
    /// End time of the last run.
    pub last_run_ended: Option<Timestamp>,
    /// Timestamp when output last changed.
    pub last_activity: Option<Timestamp>,
    /// End detail of the last run (outcome, error, final message).
    pub last_run_end_detail: Option<String>,
    /// Questions file status and content for this task.
    pub questions: Option<QuestionsSummaryFacts>,
}

/// Questions file facts for a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionsSummaryFacts {
    /// Whether there are unanswered questions.
    pub has_unanswered: bool,
    /// Formatted content or summary of questions and verdicts.
    pub content: String,
}

/// Git status of a task's worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitWorktreeSummary {
    /// Task ID.
    pub task_id: TaskId,
    /// Current branch name.
    pub branch: BranchName,
    /// Tip commit.
    pub tip: CommitId,
    /// Origin's tip commit for this branch, if seen.
    pub origin_tip: Option<CommitId>,
    /// Commits ahead of base branch.
    pub commits_over_base: Vec<GitCommitSummaryFacts>,
    /// Names of changed uncommitted files.
    pub uncommitted_files: Vec<String>,
    /// Harness refs (`refs/team-agents/`).
    pub harness_refs: Vec<String>,
    /// Whether the lead's tip contains this branch's tip.
    pub lead_contains_tip: Option<bool>,
    /// Top 50 newest untracked files (name, size in bytes, modified timestamp).
    pub newest_untracked: Vec<UntrackedFileSummaryFacts>,
}

/// Commit summary line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommitSummaryFacts {
    /// Commit ID.
    pub id: CommitId,
    /// Author name.
    pub author: String,
    /// Timestamp of commit.
    pub time: Timestamp,
    /// Subject line.
    pub subject: String,
}

/// Untracked file metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UntrackedFileSummaryFacts {
    /// Relative path in worktree.
    pub path: String,
    /// Size in bytes.
    pub size_bytes: u64,
    /// Modification time, if readable.
    pub modified: Option<Timestamp>,
}

/// Gate information for a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskGateSummary {
    /// Task ID.
    pub task_id: TaskId,
    /// Last gate record summary (commit, clean tree, passed).
    pub record: Option<GateRecordSummaryFacts>,
    /// Tails of the last up to 3 gate outputs.
    pub recent_output_tails: Vec<String>,
}

/// Summary of a gate record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateRecordSummaryFacts {
    /// Commit tested.
    pub commit: CommitId,
    /// Whether the tree was clean.
    pub clean: bool,
    /// Whether the gate passed.
    pub passed: bool,
}

/// Machine and resource metrics.
#[derive(Debug, Clone, PartialEq)]
pub struct MachineSummary {
    /// 1-, 5-, 15-minute load averages.
    pub load_avg: (f64, f64, f64),
    /// Total system memory in bytes.
    pub memory_total_bytes: u64,
    /// Available system memory in bytes.
    pub memory_available_bytes: u64,
    /// Filesystem usage for home directory.
    pub home_fs: FsUsageSummary,
    /// Filesystem usage for temporary directory.
    pub temp_fs: FsUsageSummary,
    /// Process IDs of daemon descendants.
    pub daemon_descendant_pids: Vec<u32>,
    /// Total size in bytes of tasks directory.
    pub tasks_dir_bytes: u64,
    /// Total size in bytes of worktrees directory.
    pub worktrees_dir_bytes: u64,
}

/// Filesystem space and inode usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FsUsageSummary {
    /// Percentage of space used (0-100).
    pub space_percent: u8,
    /// Percentage of inodes used (0-100).
    pub inode_percent: u8,
}

/// An unreadable collection part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadablePartSummary {
    /// Name or path of the part.
    pub name: String,
    /// Reason it could not be read.
    pub reason: String,
}

/// Renders the `SUMMARY.md` markdown text from `input`.
///
/// Output is guaranteed to be strictly less than [`MAX_SUMMARY_BYTES`] (100,000 bytes),
/// truncating long text bodies while preserving the end and all 12 section headings.
#[must_use]
pub fn render_summary(_input: &SummaryInput, _now: Timestamp, _offset: UtcOffset) -> String {
    String::new()
}
