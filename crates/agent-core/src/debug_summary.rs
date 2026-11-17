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

/// Internal text buffer avoiding direct `format_push_string` clippy warnings.
#[derive(Debug, Default)]
struct Buffer {
    /// Under construction markdown text.
    text: String,
}

impl Buffer {
    /// Creates a buffer with default capacity.
    fn new() -> Self {
        Self {
            text: String::with_capacity(8192),
        }
    }

    /// Appends a string slice.
    fn push(&mut self, s: &str) {
        self.text.push_str(s);
    }

    /// Appends a line with trailing newline.
    fn line(&mut self, s: &str) {
        self.text.push_str(s);
        self.text.push('\n');
    }

    /// Appends a blank newline.
    fn blank(&mut self) {
        self.text.push('\n');
    }
}

/// Formats a timestamp into human-readable UTC and local time representation.
fn format_time(ts: Timestamp, offset: UtcOffset) -> String {
    let utc = ts.as_utc();
    let local = utc.to_offset(offset);
    let (hours, mins, _) = offset.as_hms();
    let sign = if hours < 0 || mins < 0 { '-' } else { '+' };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC ({:02}:{:02}:{:02} {}{:02}:{:02})",
        utc.year(),
        utc.month() as u8,
        utc.day(),
        utc.hour(),
        utc.minute(),
        utc.second(),
        local.hour(),
        local.minute(),
        local.second(),
        sign,
        hours.unsigned_abs(),
        mins.unsigned_abs(),
    )
}

/// Computes minutes since activity relative to `now`, saturating at 0 if activity is in the future.
fn minutes_since(now: Timestamp, activity: Timestamp) -> i64 {
    let diff = now.as_utc() - activity.as_utc();
    let seconds = diff.whole_seconds();
    if seconds <= 0 { 0 } else { seconds / 60 }
}

/// Truncates `text` so it fits within `max_bytes`, keeping the end and prepending [`CUT_MARKER`].
fn truncate_keep_tail(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let marker_len = CUT_MARKER.len();
    if max_bytes <= marker_len {
        return text
            .chars()
            .rev()
            .take(max_bytes)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
    }
    let keep_bytes = max_bytes.saturating_sub(marker_len);
    let start_candidate = text.len().saturating_sub(keep_bytes);
    let mut boundary = start_candidate;
    while boundary < text.len() && !text.is_char_boundary(boundary) {
        boundary += 1;
    }
    let mut result = String::with_capacity(marker_len + (text.len() - boundary));
    result.push_str(CUT_MARKER);
    if let Some(tail) = text.get(boundary..) {
        result.push_str(tail);
    }
    result
}

fn render_anomalies(anomalies: &[SummaryAnomaly], b: &mut Buffer) {
    if anomalies.is_empty() {
        b.line("none of the known ones");
        b.blank();
    } else {
        for a in anomalies {
            b.line(&format!("- {}", a.message));
            if let Some(cmd) = &a.command {
                b.line(&format!("  Command: `{cmd}`"));
            }
        }
        b.blank();
    }
}

fn render_daemon(daemon: Option<&DaemonSummary>, offset: UtcOffset, b: &mut Buffer) {
    let Some(d) = daemon else {
        b.line("The daemon does not answer (unreachable).");
        b.blank();
        return;
    };
    b.line(&format!("- PID: {}", d.pid));
    if let Some(started) = d.started_at {
        b.line(&format!("- Started: {}", format_time(started, offset)));
    }
    b.line(&format!("- Version: {}", d.version));
    b.line(&format!("- Executable: {}", d.exe_path));
    if d.disk_newer_than_process {
        b.line("  (Warning: binary on disk is newer than running process)");
    }
    b.line(&format!("- Command line: {}", d.cmdline.join(" ")));
    b.line(&format!("- Environment variables: {}", d.env_var_names.join(", ")));
    b.blank();
}

fn render_team(team: &TeamSummaryFacts, offset: UtcOffset, b: &mut Buffer) {
    b.line(&format!("Team `{}` (repository `{}`)", team.id, team.repo));
    b.blank();
    b.line("### Items");
    b.blank();
    b.line("| Item | Title | Phase | Assignee |");
    b.line("|---|---|---|---|");
    for item in &team.items {
        let assignee = item.assignee.as_ref().map_or("-", AgentName::as_str);
        b.line(&format!(
            "| {} | {} | {:?} | {} |",
            item.id, item.title, item.phase, assignee
        ));
    }
    b.blank();
    b.line("### Rounds");
    b.blank();
    b.line("| Round | Item | Opened | Status |");
    b.line("|---|---|---|---|");
    for r in &team.rounds {
        let item_str = r.item.as_ref().map_or_else(|| "-".to_owned(), ToString::to_string);
        b.line(&format!(
            "| {} | {} | {} | {} |",
            r.id,
            item_str,
            format_time(r.opened_at, offset),
            r.status
        ));
    }
    b.blank();
}

fn render_task_table(tasks: &[TaskSummaryFacts], now: Timestamp, offset: UtcOffset, b: &mut Buffer) {
    b.line("| Task | Agent | Role | Status | Runs | Started | Ended | Last Activity | Minutes Since |");
    b.line("|---|---|---|---|---|---|---|---|---|");
    for t in tasks {
        let started = t
            .last_run_started
            .map_or_else(|| "-".to_owned(), |s| format_time(s, offset));
        let ended = t
            .last_run_ended
            .map_or_else(|| "-".to_owned(), |e| format_time(e, offset));
        let (act_str, mins_str) = t.last_activity.map_or_else(
            || ("-".to_owned(), "-".to_owned()),
            |a| (format_time(a, offset), format!("{} min", minutes_since(now, a))),
        );
        b.line(&format!(
            "| {} | {} | {:?} | {:?} | {} | {} | {} | {} | {} |",
            t.id, t.agent, t.role, t.status, t.run_count, started, ended, act_str, mins_str
        ));
    }
    b.blank();
}

fn render_task_details(tasks: &[TaskSummaryFacts], b: &mut Buffer) {
    for t in tasks {
        b.line(&format!("### Task `{}` (`{}`)", t.id, t.agent));
        b.blank();
        let detail = t.last_run_end_detail.as_deref().unwrap_or("-");
        b.line(&format!("- Last run end detail: {detail}"));
        if let Some(q) = &t.questions {
            b.line(&format!("- Questions (unanswered: {}):", q.has_unanswered));
            b.line(&q.content);
        } else {
            b.line("- Questions: none");
        }
        b.blank();
    }
}

fn render_git(worktrees: &[GitWorktreeSummary], offset: UtcOffset, b: &mut Buffer) {
    for w in worktrees {
        b.line(&format!("### Worktree for Task `{}`", w.task_id));
        b.blank();
        let origin = w.origin_tip.as_ref().map_or("-", CommitId::as_str);
        b.line(&format!(
            "- Branch: `{}` (Tip: `{}`, Origin: `{}`)",
            w.branch, w.tip, origin
        ));
        let lead_contains = w.lead_contains_tip.map_or("unknown", |c| if c { "yes" } else { "no" });
        b.line(&format!("- Lead contains tip: {lead_contains}"));
        b.line(&format!(
            "- Uncommitted files ({}): {}",
            w.uncommitted_files.len(),
            w.uncommitted_files.join(", ")
        ));
        b.line(&format!("- Harness refs: {}", w.harness_refs.join(", ")));
        b.line("- Commits over base:");
        for c in &w.commits_over_base {
            b.line(&format!(
                "  * `{}` {} ({})",
                c.id,
                c.subject,
                format_time(c.time, offset)
            ));
        }
        b.line(&format!("- Untracked files (newest {}):", w.newest_untracked.len()));
        for u in &w.newest_untracked {
            let mod_str = u.modified.map_or_else(|| "-".to_owned(), |m| format_time(m, offset));
            b.line(&format!(
                "  * {} ({} bytes, modified: {})",
                u.path, u.size_bytes, mod_str
            ));
        }
        b.blank();
    }
}

fn render_gate(gates: &[TaskGateSummary], b: &mut Buffer) {
    for g in gates {
        b.line(&format!("### Gate for Task `{}`", g.task_id));
        b.blank();
        if let Some(rec) = &g.record {
            let clean = if rec.clean { "clean" } else { "dirty" };
            let passed = if rec.passed { "passed" } else { "failed" };
            b.line(&format!(
                "- Record: Commit `{}`, tree {}, result: {}",
                rec.commit, clean, passed
            ));
        } else {
            b.line("- Record: none");
        }
        b.line("- Recent gate outputs:");
        for tail in &g.recent_output_tails {
            b.line("```");
            b.line(tail);
            b.line("```");
        }
        b.blank();
    }
}

fn render_history(history: Option<&str>, b: &mut Buffer) {
    if let Some(h) = history {
        b.push(h);
    } else {
        b.push("No history recorded.");
    }
    b.blank();
    b.blank();
}

fn render_daemon_log(lines: Option<&[String]>, b: &mut Buffer) {
    if let Some(ls) = lines {
        for line in ls {
            b.push(line);
            if !line.ends_with('\n') {
                b.blank();
            }
        }
    } else {
        b.push("Daemon log not found.");
    }
    b.blank();
    b.blank();
}

fn render_bus(messages: &[String], b: &mut Buffer) {
    if messages.is_empty() {
        b.line("No bus messages.");
    } else {
        for msg in messages {
            b.line(&format!("- {msg}"));
        }
    }
    b.blank();
}

fn render_notifications(notifications: &[String], b: &mut Buffer) {
    if notifications.is_empty() {
        b.line("No notifications.");
    } else {
        for n in notifications {
            b.line(&format!("- {n}"));
        }
    }
    b.blank();
}

fn render_machine(machine: &MachineSummary, unreadable: &[UnreadablePartSummary], b: &mut Buffer) {
    b.line(&format!(
        "- Load average: {:.2}, {:.2}, {:.2}",
        machine.load_avg.0, machine.load_avg.1, machine.load_avg.2
    ));
    b.line(&format!(
        "- Memory: {} / {} bytes available",
        machine.memory_available_bytes, machine.memory_total_bytes
    ));
    b.line(&format!(
        "- Home filesystem: {}% space, {}% inodes",
        machine.home_fs.space_percent, machine.home_fs.inode_percent
    ));
    b.line(&format!(
        "- Temp filesystem: {}% space, {}% inodes",
        machine.temp_fs.space_percent, machine.temp_fs.inode_percent
    ));
    let pids = machine
        .daemon_descendant_pids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    b.line(&format!("- Daemon descendant PIDs: {pids}"));
    b.line(&format!("- Tasks directory size: {} bytes", machine.tasks_dir_bytes));
    b.line(&format!(
        "- Worktrees directory size: {} bytes",
        machine.worktrees_dir_bytes
    ));
    if !unreadable.is_empty() {
        b.blank();
        b.line("### Unreadable Parts");
        b.blank();
        for u in unreadable {
            b.line(&format!("- `{}`: {}", u.name, u.reason));
        }
    }
    b.blank();
}

/// Renders the `SUMMARY.md` markdown text from `input`.
///
/// Output is guaranteed to be strictly less than [`MAX_SUMMARY_BYTES`] (100,000 bytes),
/// truncating long text bodies while preserving the end and all 12 section headings.
#[must_use]
pub fn render_summary(input: &SummaryInput, now: Timestamp, offset: UtcOffset) -> String {
    let mut b = Buffer::new();

    b.line(&format!("# Debug Summary — Team `{}`", input.team.id));
    b.blank();
    b.line(&format!("Collected at: {}", format_time(input.collected_at, offset)));
    b.blank();

    for section in SummarySection::ALL {
        b.line(section.heading());
        b.blank();
        match section {
            SummarySection::Anomalies => render_anomalies(&input.anomalies, &mut b),
            SummarySection::DaemonAndVersions => render_daemon(input.daemon.as_ref(), offset, &mut b),
            SummarySection::TeamItemsRounds => render_team(&input.team, offset, &mut b),
            SummarySection::TaskTable => render_task_table(&input.tasks, now, offset, &mut b),
            SummarySection::TaskDetails => render_task_details(&input.tasks, &mut b),
            SummarySection::GitWorktrees => render_git(&input.git, offset, &mut b),
            SummarySection::GateRecords => render_gate(&input.gate, &mut b),
            SummarySection::HistoryTail => {
                let body = input.history_tail.as_deref().unwrap_or("No history recorded.");
                let truncated = truncate_keep_tail(body, 20_000);
                render_history(Some(&truncated), &mut b);
            }
            SummarySection::DaemonLog => {
                let raw_lines = input.daemon_log_lines.as_ref().map(|lines| lines.join(""));
                if let Some(text) = raw_lines {
                    let truncated = truncate_keep_tail(&text, 25_000);
                    render_daemon_log(Some(&[truncated]), &mut b);
                } else {
                    render_daemon_log(None, &mut b);
                }
            }
            SummarySection::Bus => render_bus(&input.bus_messages, &mut b),
            SummarySection::Notifications => render_notifications(&input.notifications, &mut b),
            SummarySection::Machine => render_machine(&input.machine, &input.unreadable_parts, &mut b),
        }
    }

    if b.text.len() >= MAX_SUMMARY_BYTES {
        truncate_keep_tail(&b.text, MAX_SUMMARY_BYTES.saturating_sub(1))
    } else {
        b.text
    }
}
