//! Pure bounded summary renderer for `agent debug`.
//!
//! Renders `SUMMARY.md` from collected facts in strict specification order, with UTC and local
//! times, task tables, and byte-safe bounded truncation (< 100,000 bytes) preserving section headings.

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use time::UtcOffset;

use crate::config::RepoName;
use crate::ids::{AgentName, BranchName, ItemId, RoundId, TaskId, TeamId};
use crate::kinds::Role;
use crate::state::{CommitId, GateOutcome, GateRecord, GateTree, RoundLife, Timestamp};
use crate::status::{ItemStatus, TaskStatus};

/// Maximum byte limit for `SUMMARY.md`. The output must be strictly less than this limit.
pub const MAX_SUMMARY_BYTES: usize = 100_000;

/// Marker indicating that content was truncated to remain under [`MAX_SUMMARY_BYTES`].
pub const CUT_MARKER: &str = "\n[... cut ...]\n";

/// Fixed section headings in specification order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    /// Collection failures without a dedicated section, listed below machine facts.
    CollectionFailures,
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
            Self::CollectionFailures => "### Unreadable Parts",
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
    /// Version of the command running the summary, as `agent --version` prints it.
    pub command_version: String,
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
    /// Configuration status or loaded facts.
    pub configuration: Option<String>,
    /// Secret table and key names configured in `secrets.toml`.
    pub secret_keys: Vec<String>,
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
    pub repo: RepoName,
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
    /// Status of the round.
    pub status: RoundLife,
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
    pub uncommitted_files: Vec<PathBuf>,
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
    pub path: PathBuf,
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
    /// Last gate record.
    pub record: Option<GateRecord>,
    /// Tails of the last up to 3 gate outputs.
    pub recent_output_tails: Vec<String>,
}

/// Summary of a process descending from the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescendantProcessSummary {
    /// Process identifier.
    pub pid: u32,
    /// Age in seconds at collection time, if known.
    pub age_seconds: Option<u64>,
    /// Command line arguments, redacted.
    pub cmdline: Vec<String>,
}

/// Machine and resource metrics.
#[derive(Debug, Clone, PartialEq)]
pub struct MachineSummary {
    /// 1-, 5-, 15-minute load averages.
    pub load_avg: Option<(f64, f64, f64)>,
    /// Total system memory in bytes.
    pub memory_total_bytes: Option<u64>,
    /// Available system memory in bytes.
    pub memory_available_bytes: Option<u64>,
    /// Total swap memory in bytes.
    pub swap_total_bytes: Option<u64>,
    /// Free swap memory in bytes.
    pub swap_free_bytes: Option<u64>,
    /// Filesystem usage for home directory.
    pub home_fs: Option<FsUsageSummary>,
    /// Filesystem usage for temporary directory.
    pub temp_fs: Option<FsUsageSummary>,
    /// Processes descending from the daemon.
    pub daemon_descendants: Vec<DescendantProcessSummary>,
    /// Process IDs of daemon descendants.
    pub daemon_descendant_pids: Vec<u32>,
    /// Total size in bytes of tasks directory.
    pub tasks_dir_bytes: Option<u64>,
    /// Total size in bytes of worktrees directory.
    pub worktrees_dir_bytes: Option<u64>,
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
    /// Section responsible for this observation, independent of its display name.
    pub section: SummarySection,
    /// Name or path of the part.
    pub name: String,
    /// Reason it could not be read.
    pub reason: String,
}

/// Formats a timestamp into human-readable UTC and local time representation.
#[must_use]
pub fn format_time(ts: Timestamp, offset: UtcOffset) -> String {
    let utc = ts.as_utc();
    let (hours, mins, secs) = offset.as_hms();
    let sign = if hours < 0 || mins < 0 || secs < 0 { '-' } else { '+' };
    let offset_str = if secs != 0 {
        format!(
            "{}{:02}:{:02}:{:02}",
            sign,
            hours.unsigned_abs(),
            mins.unsigned_abs(),
            secs.unsigned_abs()
        )
    } else {
        format!("{}{:02}:{:02}", sign, hours.unsigned_abs(), mins.unsigned_abs())
    };

    let utc_str = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        utc.year(),
        u8::from(utc.month()),
        utc.day(),
        utc.hour(),
        utc.minute(),
        utc.second(),
    );

    let local_str = match utc.checked_to_offset(offset) {
        Some(local) if (0..=9999).contains(&local.year()) => format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} {}",
            local.year(),
            u8::from(local.month()),
            local.day(),
            local.hour(),
            local.minute(),
            local.second(),
            offset_str,
        ),
        _ => format!("(local offset out of range: {offset_str})"),
    };

    format!("{utc_str} ({local_str})")
}

/// Computes minutes since activity relative to `now`, saturating at 0 if activity is in the future.
fn minutes_since(now: Timestamp, activity: Timestamp) -> i64 {
    let diff = now.as_utc() - activity.as_utc();
    let seconds = diff.whole_seconds();
    if seconds <= 0 { 0 } else { seconds / 60 }
}

/// A document fragment whose structure or text determines its budget policy.
#[derive(Debug)]
enum SummaryPart {
    /// Facts and Markdown structure that must survive intact.
    Required(String),
    /// Optional text whose most recent bytes are retained.
    Tail(String),
}

/// Selects the legacy escaped API or the command's readable paths.
#[derive(Debug, Default)]
enum PathPresentation {
    /// Preserve the original renderer's escaping contract.
    #[default]
    Escaped,
    /// Preserve Unicode text without Rust string delimiters.
    Plain,
}

/// Defers text budgets until every mandatory fact has been rendered.
#[derive(Debug, Default)]
struct SummaryWriter {
    /// Ordered fragments, including all structural delimiters.
    parts: Vec<SummaryPart>,
    /// Path spelling for this presentation API.
    paths: PathPresentation,
}

impl SummaryWriter {
    #[expect(
        clippy::unnecessary_debug_formatting,
        reason = "the legacy renderer promises Rust path escaping"
    )]
    fn path(&self, path: &Path) -> String {
        match self.paths {
            PathPresentation::Escaped => format!("{:?}", path.as_os_str()),
            PathPresentation::Plain => match path.to_str() {
                Some(text) => text.to_owned(),
                None => path.as_os_str().as_encoded_bytes().escape_ascii().to_string(),
            },
        }
    }

    fn push_str(&mut self, text: &str) {
        self.parts.push(SummaryPart::Required(text.to_owned()));
    }

    fn tail(&mut self, text: &str) {
        self.parts.push(SummaryPart::Tail(text.to_owned()));
    }

    fn finish(mut self, redact: impl Fn(&str) -> String) -> Result<String, SummaryError> {
        for part in &mut self.parts {
            match part {
                SummaryPart::Required(text) | SummaryPart::Tail(text) => *text = redact(text),
            }
        }
        let mut required = 0_usize;
        let mut lengths = Vec::new();
        for part in &self.parts {
            match part {
                SummaryPart::Required(text) => required = required.saturating_add(text.len()),
                SummaryPart::Tail(text) => lengths.push(text.len()),
            }
        }
        let minimum = lengths
            .iter()
            .fold(required, |sum, len| sum.saturating_add((*len).min(CUT_MARKER.len())));
        if minimum >= MAX_SUMMARY_BYTES {
            return Err(SummaryError::RequiredFactsTooLarge {
                required_bytes: minimum,
            });
        }
        let mut remaining = MAX_SUMMARY_BYTES.saturating_sub(1).saturating_sub(required);
        lengths.sort_unstable();
        let mut lengths = lengths.into_iter();
        let mut cap = usize::MAX;
        while let Some(len) = lengths.next() {
            // The current fragment makes the divisor nonzero.
            let share = remaining / lengths.len().saturating_add(1);
            if len > share {
                cap = share;
                break;
            }
            remaining = remaining.saturating_sub(len);
        }
        let mut result = String::new();
        for part in self.parts {
            match part {
                SummaryPart::Required(text) => result.push_str(&text),
                SummaryPart::Tail(text) => result.push_str(&truncate_keep_tail(&text, cap)),
            }
        }
        Ok(result)
    }
}

/// Keeps a UTF-8 tail within its shared document budget, marking omitted content.
fn truncate_keep_tail(text: &str, budget: usize) -> String {
    if text.len() <= budget {
        return text.to_owned();
    }
    let keep_bytes = budget.saturating_sub(CUT_MARKER.len());
    let first_byte = text.len().saturating_sub(keep_bytes);
    let mut result = String::with_capacity(budget);
    result.push_str(CUT_MARKER);
    for (_, ch) in text.char_indices().skip_while(|(index, _)| *index < first_byte) {
        result.push(ch);
    }
    result
}

fn write_line(out: &mut SummaryWriter, s: &str) {
    out.push_str(s);
    out.push_str("\n");
}

fn render_anomalies(anomalies: &[SummaryAnomaly], out: &mut SummaryWriter) {
    if anomalies.is_empty() {
        write_line(out, "none of the known ones");
        write_line(out, "");
    } else {
        let mut seen = HashSet::new();
        for a in anomalies {
            if !seen.insert((&a.message, &a.command)) {
                continue;
            }
            write_line(out, &format!("- {}", a.message));
            if let Some(cmd) = &a.command {
                write_line(out, &format!("  Command: `{cmd}`"));
            }
        }
        write_line(out, "");
    }
}

/// Preserves ordinary process facts while oversized values share the optional tail budget.
fn write_process_line(out: &mut SummaryWriter, label: &str, value: &str) {
    /// Maximum process field treated as an intact required fact, in bytes.
    const INTACT_PROCESS_FIELD_BYTES: usize = 4096;
    out.push_str(label);
    if value.len() <= INTACT_PROCESS_FIELD_BYTES {
        out.push_str(value);
    } else {
        out.tail(value);
    }
    write_line(out, "");
}

fn render_daemon(input: &SummaryInput, offset: UtcOffset, out: &mut SummaryWriter) {
    let command_version = &input.command_version;
    if let Some(d) = &input.daemon {
        write_line(out, &format!("- PID: {}", d.pid));
        if let Some(started) = d.started_at {
            write_line(out, &format!("- Started: {}", format_time(started, offset)));
        }
        write_process_line(out, "- Command version: ", command_version);
        write_process_line(out, "- Daemon version: ", &d.version);
        write_process_line(out, "- Executable: ", &d.exe_path);
        if d.disk_newer_than_process {
            write_line(out, "  (Warning: binary on disk is newer than running process)");
        }
        write_process_line(out, "- Command line: ", &d.cmdline.join(" "));
        write_process_line(out, "- Environment variables: ", &d.env_var_names.join(", "));
    } else {
        if !input
            .unreadable_parts
            .iter()
            .any(|part| part.section == SummarySection::DaemonAndVersions && part.name == "daemon process")
        {
            write_line(out, "The daemon does not answer (unreachable).");
        }
        write_process_line(out, "- Command version: ", command_version);
    }

    if let Some(conf) = &input.configuration {
        if conf.contains('\n') {
            write_line(out, "- Configuration:");
            let mut text = String::new();
            for line in conf.lines() {
                text.push_str("    ");
                text.push_str(line);
                text.push('\n');
            }
            out.tail(&text);
        } else {
            write_process_line(out, "- Configuration: ", conf);
        }
    }
    if !input.secret_keys.is_empty() {
        write_process_line(out, "- Secret keys: ", &input.secret_keys.join(", "));
    }
    write_line(out, "");
}

fn render_team(team: &TeamSummaryFacts, offset: UtcOffset, out: &mut SummaryWriter) {
    write_line(out, &format!("Team `{}` (repository `{}`)", team.id, team.repo));
    write_line(out, "");
    write_line(out, "### Items");
    write_line(out, "");
    write_line(out, "| Item | Title | Phase | Assignee |");
    write_line(out, "|---|---|---|---|");
    for item in &team.items {
        let assignee = item.assignee.as_ref().map_or("-", AgentName::as_str);
        write_line(
            out,
            &format!("| {} | {} | {} | {} |", item.id, item.title, item.phase, assignee),
        );
    }
    write_line(out, "");
    write_line(out, "### Rounds");
    write_line(out, "");
    write_line(out, "| Round | Item | Opened | Status |");
    write_line(out, "|---|---|---|---|");
    for r in &team.rounds {
        let item_str = r.item.as_ref().map_or_else(|| "-".to_owned(), ToString::to_string);
        let status_str = match r.status {
            RoundLife::Open => "open".to_owned(),
            RoundLife::Closed { at, how } => format!("closed ({how}) at {}", format_time(at, offset)),
        };
        write_line(
            out,
            &format!(
                "| {} | {} | {} | {} |",
                r.id,
                item_str,
                format_time(r.opened_at, offset),
                status_str
            ),
        );
    }
    write_line(out, "");
}

fn render_task_table(tasks: &[TaskSummaryFacts], now: Timestamp, offset: UtcOffset, out: &mut SummaryWriter) {
    write_line(
        out,
        "| Task | Agent | Role | Status | Runs | Started | Ended | Last Activity | Minutes Since |",
    );
    write_line(out, "|---|---|---|---|---|---|---|---|---|");
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
        write_line(
            out,
            &format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                t.id, t.agent, t.role, t.status, t.run_count, started, ended, act_str, mins_str
            ),
        );
    }
    write_line(out, "");
}

fn render_task_details(tasks: &[TaskSummaryFacts], out: &mut SummaryWriter) {
    for t in tasks {
        write_line(out, &format!("### Task `{}` (`{}`)", t.id, t.agent));
        write_line(out, "");
        let detail = t.last_run_end_detail.as_deref().unwrap_or("-");
        out.push_str("- Last run end detail: ");
        out.tail(detail);
        write_line(out, "");
        if let Some(q) = &t.questions {
            write_line(out, &format!("- Questions (unanswered: {}):", q.has_unanswered));
            out.tail(&q.content);
            write_line(out, "");
        } else {
            write_line(out, "- Questions: none");
        }
        write_line(out, "");
    }
}

fn render_git(worktrees: &[GitWorktreeSummary], offset: UtcOffset, out: &mut SummaryWriter) {
    for w in worktrees {
        write_line(out, &format!("### Worktree for Task `{}`", w.task_id));
        write_line(out, "");
        let origin = w.origin_tip.as_ref().map_or("-", CommitId::as_str);
        write_line(
            out,
            &format!("- Branch: `{}` (Tip: `{}`, Origin: `{}`)", w.branch, w.tip, origin),
        );
        let lead_contains = w.lead_contains_tip.map_or("unknown", |c| if c { "yes" } else { "no" });
        write_line(out, &format!("- Lead contains tip: {lead_contains}"));
        let files = w
            .uncommitted_files
            .iter()
            .map(|p| out.path(p))
            .collect::<Vec<_>>()
            .join(", ");
        write_line(
            out,
            &format!("- Uncommitted files ({}): {}", w.uncommitted_files.len(), files),
        );
        write_line(out, &format!("- Harness refs: {}", w.harness_refs.join(", ")));
        write_line(out, "- Commits over base:");
        for c in &w.commits_over_base {
            write_line(
                out,
                &format!(
                    "  * `{}` by {}: {} ({})",
                    c.id,
                    c.author,
                    c.subject,
                    format_time(c.time, offset)
                ),
            );
        }
        write_line(
            out,
            &format!("- Untracked files (newest {}):", w.newest_untracked.len()),
        );
        for u in &w.newest_untracked {
            let mod_str = u.modified.map_or_else(|| "-".to_owned(), |m| format_time(m, offset));
            write_line(
                out,
                &format!(
                    "  * {} ({} bytes, modified: {})",
                    out.path(&u.path),
                    u.size_bytes,
                    mod_str
                ),
            );
        }
        write_line(out, "");
    }
}

fn render_gate(gates: &[TaskGateSummary], offset: UtcOffset, out: &mut SummaryWriter) {
    for g in gates {
        write_line(out, &format!("### Gate for Task `{}`", g.task_id));
        write_line(out, "");
        if let Some(rec) = &g.record {
            let tree = match rec.tree {
                GateTree::Clean => "clean",
                GateTree::NotClean => "dirty",
            };
            let outcome = match rec.result.outcome {
                GateOutcome::Passed => "passed".to_owned(),
                GateOutcome::Failed { exit_code } => format!("failed (exit {exit_code})"),
                GateOutcome::TimedOut => "timed out".to_owned(),
            };
            write_line(
                out,
                &format!(
                    "- Record: Commit `{}`, tree {}, outcome: {}",
                    rec.result.head, tree, outcome
                ),
            );
            write_line(
                out,
                &format!("  Finished: {}", format_time(rec.result.finished_at, offset)),
            );
            write_line(out, &format!("  Log: `{}`", out.path(rec.result.log.as_path())));
        } else {
            write_line(out, "- Record: none");
        }
        write_line(out, "- Recent gate outputs:");
        for tail in g
            .recent_output_tails
            .iter()
            .skip(g.recent_output_tails.len().saturating_sub(3))
        {
            write_line(out, "```text");
            out.tail(tail);
            write_line(out, "");
            write_line(out, "```");
        }
        write_line(out, "");
    }
}

fn render_history(history: Option<&str>, out: &mut SummaryWriter) {
    if let Some(h) = history {
        out.tail(h);
    } else {
        out.push_str("No history recorded.");
    }
    write_line(out, "");
    write_line(out, "");
}

fn render_daemon_log(lines: Option<&[String]>, out: &mut SummaryWriter) {
    if let Some(ls) = lines {
        if ls.is_empty() {
            write_line(out, "No daemon log lines about the team and no recent warnings.");
        }
        let mut text = String::new();
        for line in ls {
            text.push_str(line);
            if !line.ends_with('\n') {
                text.push('\n');
            }
        }
        out.tail(&text);
    } else {
        out.push_str("Daemon log not found.");
    }
    write_line(out, "");
    write_line(out, "");
}

fn render_bus(messages: &[String], out: &mut SummaryWriter) {
    if messages.is_empty() {
        write_line(out, "No bus messages.");
    } else {
        let mut text = String::new();
        for msg in messages {
            text.push_str("- ");
            text.push_str(msg);
            text.push('\n');
        }
        out.tail(&text);
    }
    write_line(out, "");
}

fn render_notifications(notifications: &[String], out: &mut SummaryWriter) {
    if notifications.is_empty() {
        write_line(out, "No notifications.");
    } else {
        let mut text = String::new();
        for n in notifications {
            text.push_str("- ");
            text.push_str(n);
            text.push('\n');
        }
        out.tail(&text);
    }
    write_line(out, "");
}

fn render_machine(machine: &MachineSummary, unreadable: &[UnreadablePartSummary], out: &mut SummaryWriter) {
    if let Some(avg) = machine.load_avg {
        write_line(
            out,
            &format!("- Load average: {:.2}, {:.2}, {:.2}", avg.0, avg.1, avg.2),
        );
    } else {
        write_line(out, "- Load average: unknown");
    }
    if let (Some(avail), Some(total)) = (machine.memory_available_bytes, machine.memory_total_bytes) {
        write_line(out, &format!("- Memory: {avail} / {total} bytes available"));
    } else {
        write_line(out, "- Memory: unknown");
    }
    if let Some(home) = machine.home_fs {
        write_line(
            out,
            &format!(
                "- Home filesystem: {}% space, {}% inodes",
                home.space_percent, home.inode_percent
            ),
        );
    } else {
        write_line(out, "- Home filesystem: unknown");
    }
    if let Some(temp) = machine.temp_fs {
        write_line(
            out,
            &format!(
                "- Temp filesystem: {}% space, {}% inodes",
                temp.space_percent, temp.inode_percent
            ),
        );
    } else {
        write_line(out, "- Temp filesystem: unknown");
    }
    let pids = machine
        .daemon_descendant_pids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    write_line(out, &format!("- Daemon descendant PIDs: {pids}"));
    if let Some(bytes) = machine.tasks_dir_bytes {
        write_line(out, &format!("- Tasks directory size: {bytes} bytes"));
    } else {
        write_line(out, "- Tasks directory size: unknown");
    }
    if let Some(bytes) = machine.worktrees_dir_bytes {
        write_line(out, &format!("- Worktrees directory size: {bytes} bytes"));
    } else {
        write_line(out, "- Worktrees directory size: unknown");
    }
    render_unreadable(unreadable, out);
    write_line(out, "");
}

/// Keeps collection failures visible independently of deferred machine observations.
fn render_unreadable(unreadable: &[UnreadablePartSummary], out: &mut SummaryWriter) {
    if !unreadable.is_empty() {
        write_line(out, "");
        write_line(out, "### Unreadable Parts");
        write_line(out, "");
        for u in unreadable {
            write_line(out, &format!("- `{}`: {}", u.name, u.reason));
        }
    }
}

/// Renders the `SUMMARY.md` markdown text from `input`.
///
/// Output is guaranteed to be strictly less than [`MAX_SUMMARY_BYTES`] (100,000 bytes),
/// All anomalies, structural headings, table rows and gate records are retained.
/// Long text fields share the remaining budget and retain marked UTF-8 tails.
///
/// # Errors
/// Returns [`SummaryError::RequiredFactsTooLarge`] when mandatory facts and cut
/// markers alone would reach the byte limit; no partial document is returned.
pub fn render_summary(input: &SummaryInput, offset: UtcOffset) -> Result<String, SummaryError> {
    render_with_redaction(input, offset, str::to_owned, PathPresentation::Escaped, None)
}

/// Renders collected text after redaction, before measuring or truncating any fragment.
///
/// The callback replaces confidential values in each complete presentation fragment.
///
/// # Errors
/// Returns [`SummaryError::RequiredFactsTooLarge`] when redacted required facts exceed the budget.
pub fn render_redacted_summary(
    input: &SummaryInput,
    offset: UtcOffset,
    redact: impl Fn(&str) -> String,
) -> Result<String, SummaryError> {
    render_with_redaction(input, offset, redact, PathPresentation::Escaped, None)
}

fn render_with_redaction(
    input: &SummaryInput,
    offset: UtcOffset,
    redact: impl Fn(&str) -> String,
    paths: PathPresentation,
    git_notice: Option<&str>,
) -> Result<String, SummaryError> {
    let now = input.collected_at;
    let mut out = SummaryWriter {
        paths,
        ..SummaryWriter::default()
    };

    write_line(&mut out, &format!("# Debug Summary — Team `{}`", input.team.id));
    write_line(&mut out, "");
    write_line(
        &mut out,
        &format!("Collected at: {}", format_time(input.collected_at, offset)),
    );
    write_line(&mut out, "");

    for section in SummarySection::ALL {
        write_line(&mut out, section.heading());
        write_line(&mut out, "");

        let unread = input
            .unreadable_parts
            .iter()
            .filter(|part| part.section == section)
            .collect::<Vec<_>>();
        if !unread.is_empty() {
            for part in unread {
                write_line(&mut out, &format!("Could not read {}: {}", part.name, part.reason));
            }
            write_line(&mut out, "");
            if matches!(
                section,
                SummarySection::HistoryTail
                    | SummarySection::DaemonLog
                    | SummarySection::Bus
                    | SummarySection::Notifications
            ) {
                continue;
            }
        }
        match section {
            SummarySection::Anomalies => render_anomalies(&input.anomalies, &mut out),
            SummarySection::DaemonAndVersions => {
                render_daemon(input, offset, &mut out);
            }
            SummarySection::TeamItemsRounds => render_team(&input.team, offset, &mut out),
            SummarySection::TaskTable => render_task_table(&input.tasks, now, offset, &mut out),
            SummarySection::TaskDetails => render_task_details(&input.tasks, &mut out),
            SummarySection::GitWorktrees => {
                if let Some(notice) = git_notice {
                    write_line(&mut out, notice);
                    write_line(&mut out, "");
                }
                render_git(&input.git, offset, &mut out);
            }
            SummarySection::GateRecords => render_gate(&input.gate, offset, &mut out),
            SummarySection::HistoryTail => render_history(input.history_tail.as_deref(), &mut out),
            SummarySection::DaemonLog => render_daemon_log(input.daemon_log_lines.as_deref(), &mut out),
            SummarySection::Bus => render_bus(&input.bus_messages, &mut out),
            SummarySection::Notifications => render_notifications(&input.notifications, &mut out),
            SummarySection::Machine => render_machine(&input.machine, &input.unreadable_parts, &mut out),
            SummarySection::CollectionFailures => render_unreadable(&input.unreadable_parts, &mut out),
        }

        write_line(&mut out, "");
    }

    out.finish(redact)
}

/// Mandatory summary facts exceed the pasteable document budget.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum SummaryError {
    /// Anomalies, records and structural rows cannot all fit without omission.
    #[error("mandatory summary facts need {required_bytes} bytes, exceeding the summary budget")]
    RequiredFactsTooLarge {
        /// Bytes required before allocating any optional text tails.
        required_bytes: usize,
    },
}

/// Renders a command summary with readable paths, redacting before byte budgeting.
///
/// # Errors
/// Returns [`SummaryError::RequiredFactsTooLarge`] when required facts exceed the budget.
pub fn render_command_summary(
    input: &SummaryInput,
    offset: UtcOffset,
    redact: impl Fn(&str) -> String,
) -> Result<String, SummaryError> {
    render_with_redaction(input, offset, redact, PathPresentation::Plain, None)
}

/// Renders a command summary with a retained-worktree notice in the git section.
///
/// # Errors
/// Returns [`SummaryError::RequiredFactsTooLarge`] when required facts exceed the budget.
pub fn render_command_summary_with_git_notice(
    input: &SummaryInput,
    offset: UtcOffset,
    redact: impl Fn(&str) -> String,
    git_notice: Option<&str>,
) -> Result<String, SummaryError> {
    render_with_redaction(input, offset, redact, PathPresentation::Plain, git_notice)
}
