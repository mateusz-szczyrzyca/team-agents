//! Bounded read-only observations and redacted bundle persistence.

#[cfg(test)]
mod tests;
use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_core::debug_summary::{
    GitCommitSummaryFacts, GitWorktreeSummary, ItemSummaryFacts, MachineSummary, QuestionsSummaryFacts,
    RoundSummaryFacts, SummaryInput, SummarySection, TaskGateSummary, TaskSummaryFacts, TeamSummaryFacts,
};
use agent_core::diagnostic_assessment::{Assessment, Branch, LeadGit, Origin, ReadFailure, Task};
use agent_core::ids::{AgentName, BranchName, TaskId};
use agent_core::kinds::Role;
use agent_core::questions::QuestionsFile;
use agent_core::state::{
    CommitId, GateRecord, ItemPhase, Member, RoundEnd, RoundLife, RunEnd, RunState, Team, Timestamp,
};
use tokio::io::AsyncReadExt;

use super::{DebugResult, DebugRuntimeError};
use crate::debug_redaction::SecretsCut;
use crate::process::{CaptureLimits, ExitReport, capture, error_chain};
use crate::store::Home;

/// Per-file and subprocess stream limit, in bytes.
const FILE_LIMIT: u64 = 4 * 1024 * 1024;
/// Maximum retained bytes across task files.
const TOTAL_LIMIT: usize = 64 * 1024 * 1024;
/// Maximum directory entries visited, including directories.
const ENTRY_LIMIT: usize = 4096;
/// Maximum depth of task artifacts.
const DEPTH_LIMIT: usize = 16;

/// Collected facts and bytes, kept in memory until all redaction inputs are known.
pub(super) struct Collection {
    /// Pure observations, assessed once by the caller.
    pub(super) assessment: Assessment,
    /// Presentation of the same observations.
    pub(super) summary: SummaryInput,
    /// Root of the repository clone, if available for removed worktree fallbacks.
    pub(super) checkout: Option<PathBuf>,
    /// Notice for merged worktrees whose retained branches were read in the clone.
    pub(super) git_notice: Option<String>,
    /// Local timezone offset for presentation; defaults to UTC.
    pub(super) local_offset: time::UtcOffset,
    /// Relative artifact paths and their unredacted bytes; never logged.
    artifacts: Vec<(PathBuf, Vec<u8>)>,
    /// Secret patterns applied before truncating or encoding structured observations.
    pub(super) cut: SecretsCut,
    /// Total copied task bytes, bounded independently of file count.
    copied_bytes: usize,
}

impl std::fmt::Debug for Collection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Collection")
            .field("artifact_count", &self.artifacts.len())
            .finish_non_exhaustive()
    }
}

fn empty_machine_summary() -> MachineSummary {
    MachineSummary {
        load_avg: None,
        memory_total_bytes: None,
        memory_available_bytes: None,
        swap_total_bytes: None,
        swap_free_bytes: None,
        home_fs: None,
        temp_fs: None,
        daemon_descendants: vec![],
        daemon_descendant_pids: vec![],
        tasks_dir_bytes: None,
        worktrees_dir_bytes: None,
    }
}

impl Collection {
    pub(super) fn new(team: &Team, now: Timestamp, stale_after: Duration) -> Result<Self, DebugRuntimeError> {
        let lead = team
            .crew()
            .roster()
            .first()
            .map(|(_, member)| member.task().clone())
            .ok_or_else(|| DebugRuntimeError::UnknownTarget {
                id: team.id().to_string(),
            })?;
        let items = team
            .items()
            .iter()
            .map(|item| {
                let listed =
                    team.task_list()
                        .list()
                        .item(item.id())
                        .ok_or_else(|| DebugRuntimeError::UnknownTarget {
                            id: item.id().to_string(),
                        })?;
                Ok(ItemSummaryFacts {
                    id: item.id(),
                    title: listed.title.clone(),
                    phase: item.status(),
                    assignee: match item.phase() {
                        ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. } => {
                            Some(assignee.clone())
                        }
                        ItemPhase::Pending
                        | ItemPhase::InReview { .. }
                        | ItemPhase::Approved { .. }
                        | ItemPhase::Done { .. } => None,
                    },
                })
            })
            .collect::<Result<Vec<_>, DebugRuntimeError>>()?;
        Ok(Self {
            assessment: Assessment {
                now,
                stale_after,
                daemon: None,
                command_version: Some(format!("agent {}", env!("CARGO_PKG_VERSION"))),
                lead,
                tasks: vec![],
                items: team.items().iter().map(|item| item.phase().clone()).collect(),
                lead_git: None,
                branches: vec![],
                filesystems: vec![],
                unanswered: vec![],
                unreadable: vec![],
            },
            summary: SummaryInput {
                collected_at: now,
                anomalies: vec![],
                daemon: None,
                command_version: format!("agent {}", env!("CARGO_PKG_VERSION")),
                team: TeamSummaryFacts {
                    id: team.id().clone(),
                    repo: team.repo().clone(),
                    items,
                    rounds: team
                        .rounds()
                        .iter()
                        .map(|round| RoundSummaryFacts {
                            id: round.id().clone(),
                            item: round.item(),
                            opened_at: round.opened_at(),
                            status: round.life().clone(),
                        })
                        .collect(),
                },
                tasks: vec![],
                git: vec![],
                gate: vec![],
                history_tail: None,
                daemon_log_lines: None,
                bus_messages: vec![],
                notifications: vec![],
                configuration: None,
                secret_keys: vec![],
                machine: empty_machine_summary(),
                unreadable_parts: vec![],
            },
            checkout: None,
            git_notice: None,
            local_offset: time::UtcOffset::UTC,
            artifacts: vec![],
            copied_bytes: 0,
            cut: SecretsCut::new(&agent_core::config::Secrets::default(), []),
        })
    }

    pub(super) fn add(&mut self, path: impl Into<PathBuf>, bytes: Vec<u8>) {
        self.artifacts.push((path.into(), bytes));
    }

    /// Adds the daemon artifact and its structured summary facts.
    pub(super) fn add_daemon(&mut self, mut daemon: agent_core::debug_summary::DaemonSummary) {
        for field in daemon
            .cmdline
            .iter_mut()
            .chain(&mut daemon.env_var_names)
            .chain([&mut daemon.exe_path, &mut daemon.version])
        {
            *field = self.cut.redact_text(field);
        }
        self.add("daemon-process.txt", format!("{daemon:#?}").into_bytes());
        self.summary.daemon = Some(daemon);
    }

    fn failed(&mut self, part: impl Into<String>, reason: impl Into<String>) {
        self.failed_in(SummarySection::CollectionFailures, part, reason);
    }

    fn task_failed(&mut self, part: impl Into<String>, reason: impl Into<String>) {
        self.failed_in(SummarySection::TaskDetails, part, reason);
    }

    fn gate_failed(&mut self, part: impl Into<String>, reason: impl Into<String>) {
        self.failed_in(SummarySection::GateRecords, part, reason);
    }

    fn git_failed(&mut self, part: impl Into<String>, reason: impl Into<String>) {
        self.failed_in(SummarySection::GitWorktrees, part, reason);
    }

    fn failed_in(&mut self, section: SummarySection, part: impl Into<String>, reason: impl Into<String>) {
        self.assessment.unreadable.push(ReadFailure {
            section,
            part: part.into(),
            reason: reason.into(),
        });
    }

    pub(super) async fn observe(
        &mut self,
        home: &Home,
        team: &Team,
        status: Option<Result<String, String>>,
    ) -> Result<(), DebugRuntimeError> {
        self.add(
            "team.json",
            serde_json::to_vec_pretty(team).map_err(DebugRuntimeError::Encode)?,
        );
        match status {
            Some(Ok(text)) => self.add("status.txt", text.into_bytes()),
            Some(Err(reason)) => self.failed("agent status (daemon does not answer)", reason),
            None => self.failed(
                "agent status",
                "daemon does not answer; no status response was available",
            ),
        }
        let roster = team
            .crew()
            .roster()
            .into_iter()
            .chain(team.reviewers().iter().map(|member| (Role::Reviewer, member)));
        for (role, member) in roster {
            self.task(home, member, role).await?;
        }
        self.git(home, team).await;
        self.history(home, team).await?;
        Ok(())
    }

    async fn task(&mut self, home: &Home, member: &Member, role: Role) -> Result<(), DebugRuntimeError> {
        let id = member.task();
        self.add(
            PathBuf::from("state").join(format!("{id}.json")),
            serde_json::to_vec_pretty(member).map_err(DebugRuntimeError::Encode)?,
        );
        let last = member.runs().last();
        let ended = last.and_then(|run| match run.state() {
            RunState::Running => None,
            RunState::Ended { at, .. } => Some(*at),
        });
        let row = TaskSummaryFacts {
            id: id.clone(),
            agent: member.agent().name().parse().map_err(DebugRuntimeError::Name)?,
            role,
            status: member.status(),
            run_count: member.runs().len(),
            last_run_started: last.map(agent_core::state::Run::started_at),
            last_run_ended: ended,
            last_activity: None,
            last_run_end_detail: last.map(|run| run_detail_at(run.state(), self.local_offset)),
            questions: None,
        };
        self.summary.tasks.push(row);
        self.assessment.tasks.push(Task {
            id: id.clone(),
            status: member.status(),
            last_activity: None,
        });
        self.tree(home.task_dir(id), PathBuf::from("tasks").join(id.as_str()), member)
            .await;
        self.gate(home, id).await;
        Ok(())
    }

    async fn tree(&mut self, root: PathBuf, destination: PathBuf, member: &Member) {
        let task = member.task();
        let mut pending = vec![(root, destination, 0_usize)];
        let mut visited = 0_usize;
        while let Some((source, target, depth)) = pending.pop() {
            if visited >= ENTRY_LIMIT {
                self.task_failed("task artifacts", "directory entry limit reached");
                break;
            }
            visited = visited.saturating_add(1);
            let metadata = match tokio::fs::symlink_metadata(&source).await {
                Ok(value) => value,
                Err(error)
                    if depth == 0 && error.kind() == std::io::ErrorKind::NotFound && member.runs().is_empty() =>
                {
                    continue;
                }
                Err(error) => {
                    self.task_failed(format!("task artifact {}", escaped_path(&source)), error.to_string());
                    continue;
                }
            };
            if metadata.is_dir() {
                if depth >= DEPTH_LIMIT {
                    self.task_failed(
                        format!("task artifact {}", escaped_path(&source)),
                        "directory nesting limit reached",
                    );
                    continue;
                }
                match tokio::fs::read_dir(&source).await {
                    Ok(mut entries) => {
                        let mut children = Vec::new();
                        loop {
                            match entries.next_entry().await {
                                Ok(Some(entry))
                                    if pending.len().saturating_add(children.len()).saturating_add(visited)
                                        < ENTRY_LIMIT =>
                                {
                                    children.push((
                                        entry.path(),
                                        target.join(entry.file_name()),
                                        depth.saturating_add(1),
                                    ));
                                }
                                Ok(Some(_)) => {
                                    self.task_failed(
                                        format!("task artifact {}", escaped_path(&source)),
                                        "directory entry limit reached",
                                    );
                                    break;
                                }
                                Ok(None) => break,
                                Err(error) => {
                                    self.task_failed(
                                        format!("task artifact {}", escaped_path(&source)),
                                        error.to_string(),
                                    );
                                    break;
                                }
                            }
                        }
                        children.sort_by(|left, right| numbered_order(&right.0, &left.0));
                        pending.extend(children);
                    }
                    Err(error) => {
                        self.task_failed(format!("task artifact {}", escaped_path(&source)), error.to_string());
                    }
                }
            } else if metadata.is_file() {
                self.task_file(&source, target, task, &metadata).await;
            } else {
                self.task_failed(
                    format!("task artifact {}", escaped_path(&source)),
                    "symlink or special file is not followed",
                );
            }
        }
    }

    async fn task_file(&mut self, source: &Path, target: PathBuf, task: &TaskId, metadata: &std::fs::Metadata) {
        self.activity(source, task, metadata);
        match bounded_bytes(source).await {
            Ok(bytes) if self.copied_bytes.saturating_add(bytes.len()) <= TOTAL_LIMIT => {
                self.copied_bytes = self.copied_bytes.saturating_add(bytes.len());
                self.questions(source, task, &bytes);
                self.add(target, bytes);
            }
            Ok(_) => self.task_failed(
                format!("task artifact {}", escaped_path(source)),
                "total artifact byte limit reached",
            ),
            Err(error) => self.task_failed(format!("task artifact {}", escaped_path(source)), error_chain(&error)),
        }
    }

    fn activity(&mut self, source: &Path, task: &TaskId, metadata: &std::fs::Metadata) {
        if !source
            .file_name()
            .is_some_and(|name| name == "stdout" || name == "activity.jsonl" || name == "out.jsonl")
        {
            return;
        }
        let modified = match metadata.modified() {
            Ok(value) => value,
            Err(error) => {
                self.task_failed(format!("output time {}", escaped_path(source)), error.to_string());
                return;
            }
        };
        let at = match Timestamp::from_utc(time::UtcDateTime::from(modified)) {
            Ok(at) => at,
            Err(error) => {
                self.task_failed(format!("output time {}", escaped_path(source)), error.to_string());
                return;
            }
        };
        if let Some(row) = self.summary.tasks.iter_mut().find(|row| &row.id == task) {
            row.last_activity = Some(row.last_activity.map_or(at, |previous| previous.max(at)));
        }
        if let Some(row) = self.assessment.tasks.iter_mut().find(|row| &row.id == task) {
            row.last_activity = Some(row.last_activity.map_or(at, |previous| previous.max(at)));
        }
    }

    fn questions(&mut self, source: &Path, task: &TaskId, bytes: &[u8]) {
        if !source.file_name().is_some_and(|name| {
            name.as_encoded_bytes().starts_with(b"questions") && name.as_encoded_bytes().ends_with(b".md")
        }) {
            return;
        }
        let text = match std::str::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) => {
                self.task_failed(format!("questions {}", escaped_path(source)), error.to_string());
                return;
            }
        };
        let questions = match QuestionsFile::parse(text) {
            Ok(value) => value,
            Err(error) => {
                self.task_failed(format!("questions {}", escaped_path(source)), error.to_string());
                return;
            }
        };
        let unanswered = questions.entries().iter().any(|entry| entry.verdict().is_none());
        if unanswered {
            self.assessment.unanswered.push(source.to_path_buf());
        }
        if let Some(row) = self.summary.tasks.iter_mut().find(|row| &row.id == task) {
            let summary = row.questions.get_or_insert_with(|| QuestionsSummaryFacts {
                has_unanswered: false,
                content: String::new(),
            });
            summary.has_unanswered |= unanswered;
            summary.content.push_str(&escaped_path(source));
            summary.content.push('\n');
            summary.content.push_str(text);
            summary.content.push('\n');
        }
    }

    async fn gate(&mut self, home: &Home, task: &TaskId) {
        let path = home.gate_record_file(task);
        let record = match bounded_bytes(&path).await {
            Ok(bytes) => match serde_json::from_slice::<GateRecord>(&bytes) {
                Ok(record) => Some(record),
                Err(error) => {
                    self.gate_failed(format!("gate {}", escaped_path(&path)), error.to_string());
                    None
                }
            },
            Err(ObservationError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                self.gate_failed(format!("gate {}", escaped_path(&path)), error_chain(&error));
                None
            }
        };
        let prefix = PathBuf::from("tasks").join(task.as_str());
        let mut outputs: Vec<_> = self
            .artifacts
            .iter()
            .filter(|(path, _)| {
                path.starts_with(&prefix)
                    && path.file_name().is_some_and(|name| {
                        name.as_encoded_bytes().starts_with(b"gate-") && name.as_encoded_bytes().ends_with(b".log")
                    })
            })
            .collect();
        outputs.sort_by(|(left, _), (right, _)| numbered_order(left, right));
        let selected: Vec<_> = outputs
            .into_iter()
            .rev()
            .take(3)
            .map(|(path, bytes)| (path.clone(), bytes.clone()))
            .collect();
        let mut tails = Vec::new();
        for (path, bytes) in selected {
            let source = home.root().join(&path);
            let modified = match tokio::fs::metadata(&source)
                .await
                .and_then(|metadata| metadata.modified())
            {
                Ok(at) => match Timestamp::from_utc(time::UtcDateTime::from(at)) {
                    Ok(at) => agent_core::debug_summary::format_time(at, self.local_offset),
                    Err(error) => {
                        self.gate_failed(format!("gate time {}", escaped_path(&source)), error_chain(&error));
                        "unreadable".to_owned()
                    }
                },
                Err(error) => {
                    self.gate_failed(format!("gate time {}", escaped_path(&source)), error_chain(&error));
                    "unreadable".to_owned()
                }
            };
            let text = match std::str::from_utf8(&bytes) {
                Ok(text) => text
                    .lines()
                    .rev()
                    .take(40)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n"),
                Err(error) => {
                    self.gate_failed(format!("gate text {}", escaped_path(&source)), error_chain(&error));
                    "gate output is not UTF-8; raw bytes are retained".to_owned()
                }
            };
            tails.push(format!("{} (modified: {modified})\n{text}", escaped_path(&source)));
        }
        self.summary.gate.push(TaskGateSummary {
            task_id: task.clone(),
            record,
            recent_output_tails: tails,
        });
    }

    async fn history(&mut self, home: &Home, team: &Team) -> Result<(), DebugRuntimeError> {
        let path = home.history_file();
        let id = team.id().clone();
        let result = tokio::task::spawn_blocking(move || super::history::read_live_team_history(&path, &id))
            .await
            .map_err(DebugRuntimeError::Worker)?;
        match result {
            Ok(records) if records.is_empty() => {
                self.summary.history_tail = Some("No history recorded for this team.".to_owned());
                self.add("history.txt", b"No history recorded for this team.\n".to_vec());
                self.add("history.jsonl", Vec::new());
            }
            Ok(records) => {
                let selection = agent_core::story::Selection::Team(team.id().clone());
                let text = agent_core::story::render(
                    &records,
                    &selection,
                    agent_core::story::StoryOptions {
                        prompts: agent_core::story::PromptView::Whole,
                        steps: agent_core::story::StepView::Listed,
                    },
                );
                self.summary.history_tail = Some(text.clone());
                self.enrich_last_run_details(&records, team);
                self.add("history.txt", text.into_bytes());
                self.add(
                    "history.jsonl",
                    agent_core::story::json_lines(&records, &selection)
                        .join("\n")
                        .into_bytes(),
                );
            }
            Err(error) => self.assessment.unreadable.push(ReadFailure {
                section: agent_core::debug_summary::SummarySection::HistoryTail,
                part: "team history".to_owned(),
                reason: error_chain(&error),
            }),
        }
        Ok(())
    }

    fn enrich_last_run_details(&mut self, records: &[agent_core::history::SequencedRecord], team: &Team) {
        let selection = agent_core::story::Selection::Team(team.id().clone());
        let roster = team
            .crew()
            .roster()
            .into_iter()
            .chain(team.reviewers().iter().map(|member| (Role::Reviewer, member)));
        for (_role, member) in roster {
            let Some(last_run) = member.runs().last() else {
                continue;
            };
            let run_id = last_run.id();
            let mut run_records: Vec<_> = records
                .iter()
                .filter(|seq| super::history::record_run_id(&seq.record) == Some(run_id))
                .cloned()
                .collect();
            if run_records.is_empty() {
                continue;
            }
            // The message has its own budget so its tail cannot evict the run's reason and steps.
            for record in &mut run_records {
                if let agent_core::history::Record::RunEnd(end) = &mut record.record
                    && let Some(message) = &mut end.final_message
                {
                    /// Maximum final-message tail in the summary; complete text stays in history artifacts.
                    const FINAL_MESSAGE_BYTES: usize = 4096;
                    *message = self.cut.redact_text(message);
                    if message.len() > FINAL_MESSAGE_BYTES {
                        let first = message.len().saturating_sub(FINAL_MESSAGE_BYTES);
                        let tail: String = message
                            .char_indices()
                            .skip_while(|(index, _)| *index < first)
                            .map(|(_, ch)| ch)
                            .collect();
                        *message = format!("{}{tail}", agent_core::debug_summary::CUT_MARKER);
                    }
                }
            }
            let story_text = agent_core::story::render(
                &run_records,
                &selection,
                agent_core::story::StoryOptions {
                    prompts: agent_core::story::PromptView::Preview,
                    steps: agent_core::story::StepView::Listed,
                },
            );
            if let Some(task_summary) = self.summary.tasks.iter_mut().find(|t| t.id == *member.task()) {
                match &mut task_summary.last_run_end_detail {
                    Some(existing) => {
                        existing.push('\n');
                        existing.push_str(&story_text);
                    }
                    None => {
                        task_summary.last_run_end_detail = Some(story_text);
                    }
                }
            }
        }
    }

    async fn git(&mut self, home: &Home, team: &Team) {
        let mut has_removed_merged_worktrees = false;

        let tasks: Vec<_> = self
            .summary
            .tasks
            .iter()
            .map(|task| (task.id.clone(), task.agent.clone()))
            .collect();
        for (task, agent) in tasks {
            let (latest_round, recorded) = task_round(team, &task, &agent);
            let round_merged = latest_round.is_some_and(|round| {
                matches!(
                    round.life(),
                    RoundLife::Closed {
                        how: RoundEnd::Merged,
                        ..
                    }
                )
            });
            let path = home.worktree_dir(team.repo(), &task);
            let worktree_missing = match worktree_is_missing(&path).await {
                Ok(missing) => missing,
                Err(error) => {
                    self.git_failed(
                        format!("worktree metadata {}", escaped_path(&path)),
                        diagnostic_error(&error),
                    );
                    continue;
                }
            };

            let observation = if round_merged && worktree_missing {
                if let Some(ref checkout) = self.checkout {
                    match read_git_from_checkout(
                        checkout,
                        &task,
                        &agent,
                        recorded,
                        latest_round.filter(|_| task != self.assessment.lead),
                    )
                    .await
                    {
                        Ok(obs) => {
                            has_removed_merged_worktrees = true;
                            Ok(obs)
                        }
                        Err(err) => Err(err),
                    }
                } else {
                    read_git(&path, &task).await
                }
            } else {
                read_git(&path, &task).await
            };

            match observation {
                Ok(observation) => {
                    self.assessment.unreadable.extend(observation.unreadable);
                    if let Some((artifact_path, bytes)) = observation.untracked_artifact {
                        self.artifacts.push((artifact_path, bytes));
                    }
                    if task == self.assessment.lead {
                        self.assessment.lead_git = Some(LeadGit {
                            tip: observation.facts.tip.clone(),
                            dirty: observation.dirty,
                            gate: self
                                .summary
                                .gate
                                .iter()
                                .find(|gate| gate.task_id == task)
                                .and_then(|gate| gate.record.clone()),
                        });
                    }
                    self.summary.git.push(observation.facts);
                }
                Err(error) => self.git_failed(format!("worktree {}", escaped_path(&path)), diagnostic_error(&error)),
            }
        }
        if has_removed_merged_worktrees {
            self.git_notice = Some("Worktrees for merged rounds were removed; retained branches and commits were collected from the repository checkout.".to_owned());
            self.add(
                PathBuf::from("git").join("removed-worktrees.txt"),
                b"Worktrees for merged rounds were removed; retained branches and commits were collected from the repository checkout.\n".to_vec(),
            );
        }
        self.git_details(home, team).await;
        for git in &self.summary.git {
            self.artifacts.push((
                PathBuf::from("git").join(format!("{}.txt", git.task_id)),
                format!("{git:#?}").into_bytes(),
            ));
        }
    }

    async fn git_details(&mut self, home: &Home, team: &Team) {
        if self.summary.git.is_empty() {
            return;
        }
        let lead_tip = self
            .summary
            .git
            .iter()
            .find(|git| git.task_id == self.assessment.lead)
            .map(|git| git.tip.clone());
        let lead_path = home.worktree_dir(team.repo(), &self.assessment.lead);
        let lead_worktree_missing = match worktree_is_missing(&lead_path).await {
            Ok(missing) => missing,
            Err(error) => {
                self.git_failed(
                    format!("worktree metadata {}", escaped_path(&lead_path)),
                    diagnostic_error(&error),
                );
                return;
            }
        };
        let ancestor_dir = if lead_worktree_missing {
            self.checkout.clone().unwrap_or(lead_path.clone())
        } else {
            lead_path.clone()
        };

        for index in 0..self.summary.git.len() {
            let Some(git) = self.summary.git.get(index) else {
                continue;
            };
            let task = git.task_id.clone();
            let tip = git.tip.clone();
            let contains = match &lead_tip {
                Some(lead) => match git_capture(
                    &ancestor_dir,
                    &["merge-base", "--is-ancestor", tip.as_str(), lead.as_str()],
                )
                .await
                {
                    Ok(output) => match output.exit {
                        ExitReport::Exited { code: 0 } => Some(true),
                        ExitReport::Exited { code: 1 } => Some(false),
                        other => {
                            self.git_failed(
                                format!("git containment {task}"),
                                diagnostic_error(&ObservationError::Git {
                                    exit: other,
                                    stderr: output.stderr.bytes,
                                }),
                            );
                            None
                        }
                    },
                    Err(error) => {
                        self.git_failed(format!("git containment {task}"), diagnostic_error(&error));
                        None
                    }
                },
                None => None,
            };
            if let Some(git) = self.summary.git.get_mut(index) {
                git.lead_contains_tip = contains;
                let is_lead = task == self.assessment.lead;
                let origin = if is_lead {
                    Some(git.origin_tip.clone().map_or(Origin::Missing, Origin::Tip))
                } else {
                    git.origin_tip.clone().map(Origin::Tip)
                };
                self.assessment.branches.push(Branch {
                    task: task.clone(),
                    name: git.branch.clone(),
                    tip: git.tip.clone(),
                    origin,
                    contained_in_lead: contains,
                });
            }
        }
        self.commits(home, team).await;
    }

    async fn commits(&mut self, home: &Home, team: &Team) {
        let path = home.config_file();
        let base = match super::required_text(&path, agent_core::config::MAX_CONFIG_BYTES).await {
            Ok(file) => match agent_core::config::Config::from_toml_str(&file.text) {
                Ok(config) => config.repo(team.repo()).map(|repo| repo.base_branch.clone()),
                Err(error) => {
                    self.git_failed("git base configuration", error_chain(&error));
                    return;
                }
            },
            Err(error) => {
                self.git_failed("git base configuration", error_chain(&error));
                return;
            }
        };
        let Some(base) = base else {
            self.git_failed("git base configuration", "repository has no configured base branch");
            return;
        };
        let tasks: Vec<_> = self
            .summary
            .git
            .iter()
            .map(|git| (git.task_id.clone(), git.branch.clone()))
            .collect();
        for (task, branch) in tasks {
            let path = home.worktree_dir(team.repo(), &task);
            let worktree_missing = match worktree_is_missing(&path).await {
                Ok(missing) => missing,
                Err(error) => {
                    self.git_failed(
                        format!("worktree metadata {}", escaped_path(&path)),
                        diagnostic_error(&error),
                    );
                    continue;
                }
            };

            let (target_dir, rev_range) = if worktree_missing {
                if let Some(ref checkout) = self.checkout {
                    (checkout.as_path(), format!("{base}..{branch}"))
                } else {
                    (path.as_path(), format!("{base}..HEAD"))
                }
            } else {
                (path.as_path(), format!("{base}..HEAD"))
            };

            match git_text(
                target_dir,
                &["log", "--format=%H%x00%aI%x00%an%x00%s", "--no-decorate", &rev_range],
            )
            .await
            {
                Ok(text) => {
                    let parsed = text.lines().map(parse_commit).collect::<Result<Vec<_>, _>>();
                    match parsed {
                        Ok(commits) => {
                            if let Some(git) = self.summary.git.iter_mut().find(|git| git.task_id == task) {
                                git.commits_over_base = commits;
                            }
                        }
                        Err(error) => self.git_failed(format!("git commit details {task}"), diagnostic_error(&error)),
                    }
                    self.add(
                        PathBuf::from("git").join(format!("{task}-commits.txt")),
                        text.into_bytes(),
                    );
                }
                Err(error) => self.git_failed(format!("git commits {task}"), diagnostic_error(&error)),
            }
        }
    }
}

/// Distinguishes removed worktrees from metadata failures before choosing a clone fallback.
async fn worktree_is_missing(path: &Path) -> Result<bool, ObservationError> {
    match tokio::fs::metadata(path).await {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(ObservationError::Io(error)),
    }
}

/// A bounded observation failure, converted to a read-failure only at its part's boundary.
#[derive(Debug, thiserror::Error)]
enum ObservationError {
    /// File access failed.
    #[error("cannot read file")]
    Io(#[source] std::io::Error),
    /// The source exceeded the collection budget.
    #[error("source exceeds diagnostic byte limit")]
    TooLarge,
    /// A subprocess could not complete within its limits.
    #[error("cannot run read-only git command")]
    Capture(#[source] crate::process::CaptureError),
    /// Git returned a failure status.
    #[error("read-only git command exited {exit:?}: {stderr:?}", stderr = std::ffi::OsStr::from_bytes(.stderr))]
    Git {
        /// Checked process exit.
        exit: ExitReport,
        /// Bounded error bytes, escaped without losing data.
        stderr: Vec<u8>,
    },
    /// Git did not return a complete porcelain status record.
    #[error("git status record lacks required fields")]
    StatusFields,
    /// Git did not return a complete retained reference record.
    #[error("git reference record lacks required fields")]
    RefFields,
    /// Git did not return all requested commit fields.
    #[error("git commit record lacks required fields")]
    CommitFields,
    /// A commit timestamp is not representable.
    #[error("git commit timestamp is invalid")]
    CommitTime(#[source] agent_core::state::InvalidTimestamp),
    /// Git output is not UTF-8.
    #[error("git output is not UTF-8")]
    Utf8(#[source] std::string::FromUtf8Error),
    /// A branch name is invalid.
    #[error("git branch is invalid")]
    Branch(#[source] agent_core::ids::InvalidBranchName),
    /// A git object ID is invalid.
    #[error("git tip is invalid")]
    Commit(#[source] agent_core::state::InvalidCommitId),
}

fn parse_commit(line: &str) -> Result<GitCommitSummaryFacts, ObservationError> {
    let mut fields = line.splitn(4, '\0');
    let id = fields.next().ok_or(ObservationError::CommitFields)?;
    let time = fields.next().ok_or(ObservationError::CommitFields)?;
    let author = fields.next().ok_or(ObservationError::CommitFields)?;
    let subject = fields.next().ok_or(ObservationError::CommitFields)?;
    Ok(GitCommitSummaryFacts {
        id: id.parse().map_err(ObservationError::Commit)?,
        time: time.parse().map_err(ObservationError::CommitTime)?,
        author: author.to_owned(),
        subject: subject.to_owned(),
    })
}

async fn bounded_bytes(path: &Path) -> Result<Vec<u8>, ObservationError> {
    let file = tokio::fs::File::open(path).await.map_err(ObservationError::Io)?;
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT.saturating_add(1))
        .read_to_end(&mut bytes)
        .await
        .map_err(ObservationError::Io)?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err(ObservationError::TooLarge);
    }
    Ok(bytes)
}

#[cfg(test)]
pub(super) static TEST_GIT_COMMANDS: std::sync::Mutex<Vec<(PathBuf, Vec<String>)>> = std::sync::Mutex::new(Vec::new());

async fn git_capture(path: &Path, args: &[&str]) -> Result<crate::process::Captured, ObservationError> {
    #[cfg(test)]
    if let Ok(mut log) = TEST_GIT_COMMANDS.lock() {
        log.push((path.to_path_buf(), args.iter().map(|s| (*s).to_owned()).collect()));
    }
    let mut command = tokio::process::Command::new("git");
    command
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(path)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0");
    let result = capture(
        command,
        CaptureLimits {
            timeout: Duration::from_secs(10),
            max_bytes: FILE_LIMIT,
        },
    )
    .await
    .map_err(ObservationError::Capture)?;
    if result.stdout.dropped != 0 || result.stderr.dropped != 0 {
        return Err(ObservationError::TooLarge);
    }
    Ok(result)
}

async fn git_text(path: &Path, args: &[&str]) -> Result<String, ObservationError> {
    String::from_utf8(git_bytes(path, args).await?).map_err(ObservationError::Utf8)
}

async fn git_bytes(path: &Path, args: &[&str]) -> Result<Vec<u8>, ObservationError> {
    let result = git_capture(path, args).await?;
    match result.exit {
        ExitReport::Exited { code: 0 } => Ok(result.stdout.bytes),
        exit => Err(ObservationError::Git {
            exit,
            stderr: result.stderr.bytes,
        }),
    }
}

fn parse_status(bytes: &[u8]) -> Result<Vec<PathBuf>, ObservationError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let body = bytes.strip_suffix(&[0]).ok_or(ObservationError::StatusFields)?;
    let mut records = body.split(|byte| *byte == 0);
    let mut paths = Vec::new();
    while let Some(record) = records.next() {
        let [index, worktree, b' ', path @ ..] = record else {
            return Err(ObservationError::StatusFields);
        };
        if path.is_empty() {
            return Err(ObservationError::StatusFields);
        }
        if [index, worktree]
            .iter()
            .any(|status| **status == b'R' || **status == b'C')
        {
            records
                .next()
                .filter(|source| !source.is_empty())
                .ok_or(ObservationError::StatusFields)?;
        }
        if (*index, *worktree) != (b'?', b'?') {
            paths.push(PathBuf::from(OsString::from_vec(path.to_vec())));
        }
    }
    Ok(paths)
}

fn parse_untracked(bytes: &[u8]) -> Result<Vec<PathBuf>, ObservationError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let body = bytes.strip_suffix(&[0]).ok_or(ObservationError::StatusFields)?;
    let mut records = body.split(|byte| *byte == 0);
    let mut paths = Vec::new();
    while let Some(record) = records.next() {
        let [index, worktree, b' ', path @ ..] = record else {
            return Err(ObservationError::StatusFields);
        };
        if path.is_empty() {
            return Err(ObservationError::StatusFields);
        }
        if [index, worktree]
            .iter()
            .any(|status| **status == b'R' || **status == b'C')
        {
            records
                .next()
                .filter(|source| !source.is_empty())
                .ok_or(ObservationError::StatusFields)?;
        }
        if (*index, *worktree) == (b'?', b'?') {
            paths.push(PathBuf::from(OsString::from_vec(path.to_vec())));
        }
    }
    Ok(paths)
}

/// Worktree facts plus dirtiness including files that git does not track.
#[derive(Debug)]
struct GitObservation {
    /// Details retained in the summary and worktree artifact.
    facts: GitWorktreeSummary,
    /// Any status record makes the worktree dirty, including untracked files.
    dirty: bool,
    /// Complete untracked files artifact to write into the diagnostic bundle.
    untracked_artifact: Option<(PathBuf, Vec<u8>)>,
    /// Unreadable untracked paths with reasons.
    unreadable: Vec<ReadFailure>,
}

async fn read_git(path: &Path, task: &TaskId) -> Result<GitObservation, ObservationError> {
    let branch: BranchName = git_text(path, &["symbolic-ref", "--short", "HEAD"])
        .await?
        .trim()
        .parse()
        .map_err(ObservationError::Branch)?;
    let tip: CommitId = git_text(path, &["rev-parse", "--verify", "HEAD"])
        .await?
        .trim()
        .parse()
        .map_err(ObservationError::Commit)?;
    let origin = git_text(
        path,
        &[
            "for-each-ref",
            "--format=%(objectname)",
            &format!("refs/remotes/origin/{branch}"),
        ],
    )
    .await?;
    let origin_tip = if origin.trim().is_empty() {
        None
    } else {
        Some(origin.trim().parse().map_err(ObservationError::Commit)?)
    };
    let status = git_bytes(path, &["status", "--porcelain=v1", "-z", "--untracked-files=all"]).await?;
    let published = format!("refs/team-agents/published/{branch}");
    let reviewed = format!("refs/team-agents/reviewed/{branch}");
    let harness = git_text(
        path,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            &published,
            &reviewed,
        ],
    )
    .await?;
    let uncommitted_files = parse_status(&status)?;
    let untracked_paths = parse_untracked(&status)?;
    let untracked = super::untracked::collect(path, &untracked_paths).await;
    let untracked_artifact = if untracked.artifact_text.is_empty() {
        None
    } else {
        Some((
            PathBuf::from("tasks").join(task.as_str()).join("untracked.txt"),
            untracked.artifact_text.into_bytes(),
        ))
    };
    Ok(GitObservation {
        dirty: !status.is_empty(),
        facts: GitWorktreeSummary {
            task_id: task.clone(),
            branch,
            tip,
            origin_tip,
            commits_over_base: vec![],
            uncommitted_files,
            harness_refs: harness
                .lines()
                .filter(|line| {
                    line.split_once(' ')
                        .is_some_and(|(name, _)| name == published || name == reviewed)
                })
                .map(str::to_owned)
                .collect(),
            lead_contains_tip: None,
            newest_untracked: untracked.summary_facts,
        },
        untracked_artifact,
        unreadable: untracked.unreadable,
    })
}

/// Resolves a task's last round and its recorded merged branch, including older states without run-round links.
fn task_round<'a>(
    team: &'a Team,
    task: &TaskId,
    agent: &AgentName,
) -> (Option<&'a agent_core::state::Round>, Option<&'a BranchName>) {
    let prefix = format!("agents/{agent}/{task}-");
    let member = team
        .crew()
        .roster()
        .into_iter()
        .chain(team.reviewers().iter().map(|member| (Role::Reviewer, member)))
        .find(|(_, member)| member.task() == task)
        .map(|(_, member)| member);
    let run_round = member
        .and_then(|member| member.runs().last())
        .and_then(|run| run.rounds().last())
        .and_then(|id| team.rounds().iter().find(|round| round.id() == id));
    let recorded = team.items().iter().rev().find_map(|item| match item.phase() {
        ItemPhase::Done {
            completion: agent_core::state::Completion::Merged { pr, .. },
        } => (pr.branch.as_str().starts_with(&prefix) && run_round.is_none_or(|round| round.item() == Some(item.id())))
            .then_some((item.id(), &pr.branch)),
        ItemPhase::Done {
            completion: agent_core::state::Completion::Listed,
        }
        | ItemPhase::Pending
        | ItemPhase::InProgress { .. }
        | ItemPhase::InReview { .. }
        | ItemPhase::Fixing { .. }
        | ItemPhase::Approved { .. } => None,
    });
    let latest_round = run_round
        .or_else(|| recorded.and_then(|(item, _)| team.rounds().iter().find(|round| round.item() == Some(item))))
        .or_else(|| {
            (team
                .crew()
                .roster()
                .first()
                .is_some_and(|(_, member)| member.task() == task))
            .then(|| team.rounds().last())
            .flatten()
        });
    (latest_round, recorded.map(|(_, branch)| branch))
}

/// Decodes the name and object ID fields of a retained reference record.
fn ref_fields(line: &str) -> Result<(&str, &str), ObservationError> {
    let (name, hash) = line.split_once(' ').ok_or(ObservationError::RefFields)?;
    if name.is_empty() || hash.is_empty() {
        return Err(ObservationError::RefFields);
    }
    Ok((name, hash))
}

async fn read_git_from_checkout(
    checkout: &Path,
    task: &TaskId,
    agent: &AgentName,
    recorded: Option<&BranchName>,
    round: Option<&agent_core::state::Round>,
) -> Result<GitObservation, ObservationError> {
    let branches_output = git_text(
        checkout,
        &["for-each-ref", "--format=%(refname:short) %(objectname)", "refs/heads/"],
    )
    .await?;

    let exact = format!("agents/{agent}/{task}");
    let prefix = format!("agents/{agent}/{task}-");

    let derived = round
        .and_then(agent_core::state::Round::item)
        .map(|item| format!("{prefix}{item}").parse::<BranchName>())
        .transpose()
        .map_err(ObservationError::Branch)?;
    let recorded = recorded.or(derived.as_ref());

    let mut candidate: Option<(BranchName, CommitId)> = None;

    for line in branches_output.lines() {
        let (ref_name, hash) = ref_fields(line)?;

        let is_match = match recorded {
            Some(branch) => ref_name == branch.as_str(),
            None => ref_name == exact || ref_name.starts_with(&prefix),
        };

        if is_match {
            let branch: BranchName = ref_name.parse().map_err(ObservationError::Branch)?;
            let tip: CommitId = hash.parse().map_err(ObservationError::Commit)?;
            if ref_name.starts_with(&prefix) || candidate.is_none() {
                candidate = Some((branch, tip));
            }
        }
    }

    let (branch, tip) = candidate.ok_or_else(|| {
        ObservationError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("no retained branch found in checkout for task {task}"),
        ))
    })?;

    let origin = git_text(
        checkout,
        &[
            "for-each-ref",
            "--format=%(objectname)",
            &format!("refs/remotes/origin/{branch}"),
        ],
    )
    .await?;
    let origin_tip = if origin.trim().is_empty() {
        None
    } else {
        Some(origin.trim().parse().map_err(ObservationError::Commit)?)
    };

    let published = format!("refs/team-agents/published/{branch}");
    let reviewed = format!("refs/team-agents/reviewed/{branch}");
    let harness = git_text(
        checkout,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            &published,
            &reviewed,
        ],
    )
    .await?;

    Ok(GitObservation {
        dirty: false,
        facts: GitWorktreeSummary {
            task_id: task.clone(),
            branch,
            tip,
            origin_tip,
            commits_over_base: vec![],
            uncommitted_files: vec![],
            harness_refs: harness
                .lines()
                .filter(|line| {
                    line.split_once(' ')
                        .is_some_and(|(name, _)| name == published || name == reviewed)
                })
                .map(str::to_owned)
                .collect(),
            lead_contains_tip: None,
            newest_untracked: vec![],
        },
        untracked_artifact: None,
        unreadable: vec![],
    })
}

pub(super) async fn persist(
    collection: Collection,
    cut: SecretsCut,
    parent: PathBuf,
    prefix: String,
    summary: String,
) -> Result<DebugResult, DebugRuntimeError> {
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        use std::os::unix::fs::DirBuilderExt;
        let mut artifacts: Vec<_> = collection
            .artifacts
            .into_iter()
            .map(|(path, bytes)| {
                (
                    PathBuf::from(OsString::from_vec(cut.redact(path.as_os_str().as_encoded_bytes()))),
                    cut.redact(&bytes),
                )
            })
            .collect();
        // The summary was redacted before budgeting and must not expand after its size was checked.
        artifacts.push((PathBuf::from("SUMMARY.md"), summary.into_bytes()));
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&parent)
            .map_err(|source| DebugRuntimeError::Write {
                path: parent.clone(),
                source,
            })?;
        let directory = tempfile::Builder::new()
            .prefix(&format!("{prefix}-"))
            .tempdir_in(&parent)
            .map_err(|source| DebugRuntimeError::Write { path: parent, source })?;
        let result = (|| {
            for (relative, bytes) in artifacts {
                let path = directory.path().join(relative);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|source| DebugRuntimeError::Write {
                        path: parent.to_path_buf(),
                        source,
                    })?;
                }
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|source| DebugRuntimeError::Write {
                        path: path.clone(),
                        source,
                    })?;
                file.write_all(&bytes)
                    .and_then(|()| file.flush())
                    .map_err(|source| DebugRuntimeError::Write { path, source })?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            if let Err(cleanup) = directory.close() {
                // Cleanup cannot replace the original write error; preserve both in diagnostics.
                tracing::warn!(error = %cleanup, "cannot remove incomplete diagnostic directory");
            }
            return Err(error);
        }
        let bundle_dir = directory.keep();
        Ok(DebugResult {
            summary_path: bundle_dir.join("SUMMARY.md"),
            bundle_dir,
        })
    })
    .await
    .map_err(DebugRuntimeError::Worker)?
}

/// Spells paths as text while retaining otherwise invalid UTF-8 bytes.
fn escaped_path(path: &Path) -> String {
    match path.to_str() {
        Some(text) => text.to_owned(),
        None => path.as_os_str().as_bytes().escape_ascii().to_string(),
    }
}

/// Orders numbered artifact names without numeric overflow or lexical 10-before-2 order.
fn numbered_order(left: &Path, right: &Path) -> std::cmp::Ordering {
    fn key(path: &Path) -> Option<(&Path, &str, usize, &str)> {
        let stem = path.file_stem()?.to_str()?;
        let (prefix, number) = stem.rsplit_once('-')?;
        if !number.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let digits = number.trim_start_matches('0');
        Some((path.parent()?, prefix, digits.len(), digits))
    }
    match (key(left), key(right)) {
        (Some(left_key), Some(right_key)) => left_key.cmp(&right_key).then_with(|| left.cmp(right)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => left.cmp(right),
    }
}

/// Converts the bounded process failure into prose without Rust value formatting.
fn diagnostic_error(error: &ObservationError) -> String {
    if let ObservationError::Git { exit, stderr } = error {
        let status = match exit {
            ExitReport::Exited { code } => format!("exited with {code}"),
            ExitReport::Signaled { signal } => format!("terminated by signal {signal}"),
        };
        let line = stderr.split(|byte| *byte == b'\n').next().unwrap_or_default();
        let text = match std::str::from_utf8(line) {
            Ok(text) => text.to_owned(),
            Err(_) => line.escape_ascii().to_string(),
        };
        return format!("read-only git command {status}: {text}");
    }
    error_chain(error)
}

/// Describes a run end as the diagnosis a reader can act on.
fn run_detail_at(state: &RunState, offset: time::UtcOffset) -> String {
    let RunState::Ended { at, end } = state else {
        return "running".to_owned();
    };
    let how = match end {
        RunEnd::Reported { outcome } => format!("reported {}", outcome.to_string().to_uppercase()),
        RunEnd::NoStatus { exit_code: Some(code) } => format!("without a status, exited with {code}"),
        RunEnd::NoStatus { exit_code: None } => "without a status, terminated by signal".to_owned(),
        RunEnd::QuotaExhausted { resets_at: Some(at) } => {
            format!(
                "quota exhausted, resets at {}",
                agent_core::debug_summary::format_time(*at, offset)
            )
        }
        RunEnd::QuotaExhausted { resets_at: None } => "quota exhausted".to_owned(),
        RunEnd::RateLimited => "rate limited".to_owned(),
        RunEnd::Hung => "stopped by the watchdog".to_owned(),
        RunEnd::TurnLimit => "turn limit reached".to_owned(),
        RunEnd::Stopped => "stopped by the human".to_owned(),
    };
    format!(
        "ended, {how}, at {}",
        agent_core::debug_summary::format_time(*at, offset)
    )
}
