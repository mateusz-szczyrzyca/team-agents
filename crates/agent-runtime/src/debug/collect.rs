//! Bounded read-only observations and redacted bundle persistence.
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_core::debug_summary::{
    GitWorktreeSummary, ItemSummaryFacts, MachineSummary, QuestionsSummaryFacts, RoundSummaryFacts, SummaryInput,
    TaskGateSummary, TaskSummaryFacts, TeamSummaryFacts,
};
use agent_core::diagnostic_assessment::{Assessment, Branch, LeadGit, Origin, ReadFailure, Task};
use agent_core::ids::{BranchName, TaskId};
use agent_core::kinds::Role;
use agent_core::questions::QuestionsFile;
use agent_core::state::{CommitId, GateRecord, ItemPhase, Member, RunState, Team, Timestamp};
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
    /// Relative artifact paths and their unredacted bytes; never logged.
    artifacts: Vec<(PathBuf, Vec<u8>)>,
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
            .map(|item| ItemSummaryFacts {
                id: item.id(),
                title: item.id().to_string(),
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
            .collect();
        Ok(Self {
            assessment: Assessment {
                now,
                stale_after,
                daemon: None,
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
                machine: MachineSummary {
                    load_avg: None,
                    memory_total_bytes: None,
                    memory_available_bytes: None,
                    home_fs: None,
                    temp_fs: None,
                    daemon_descendant_pids: vec![],
                    tasks_dir_bytes: None,
                    worktrees_dir_bytes: None,
                },
                unreadable_parts: vec![],
            },
            artifacts: vec![],
            copied_bytes: 0,
        })
    }

    pub(super) fn add(&mut self, path: impl Into<PathBuf>, bytes: Vec<u8>) {
        self.artifacts.push((path.into(), bytes));
    }

    fn failed(&mut self, part: impl Into<String>, reason: impl Into<String>) {
        self.assessment.unreadable.push(ReadFailure {
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
            last_run_end_detail: last.map(|run| format!("{:?}", run.state())),
            questions: None,
        };
        self.summary.tasks.push(row);
        self.assessment.tasks.push(Task {
            id: id.clone(),
            status: member.status(),
            last_activity: None,
        });
        self.tree(home.task_dir(id), PathBuf::from("tasks").join(id.as_str()), id)
            .await;
        self.gate(home, id).await;
        Ok(())
    }

    async fn tree(&mut self, root: PathBuf, destination: PathBuf, task: &TaskId) {
        let mut pending = vec![(root, destination, 0_usize)];
        let mut visited = 0_usize;
        while let Some((source, target, depth)) = pending.pop() {
            if visited >= ENTRY_LIMIT {
                self.failed("task artifacts", "directory entry limit reached");
                break;
            }
            visited = visited.saturating_add(1);
            let metadata = match tokio::fs::symlink_metadata(&source).await {
                Ok(value) => value,
                Err(error) => {
                    self.failed(format!("task artifact {}", escaped_path(&source)), error.to_string());
                    continue;
                }
            };
            if metadata.is_dir() {
                if depth >= DEPTH_LIMIT {
                    self.failed(
                        format!("task artifact {}", escaped_path(&source)),
                        "directory nesting limit reached",
                    );
                    continue;
                }
                match tokio::fs::read_dir(&source).await {
                    Ok(mut entries) => loop {
                        match entries.next_entry().await {
                            Ok(Some(entry)) if pending.len().saturating_add(visited) < ENTRY_LIMIT => {
                                pending.push((entry.path(), target.join(entry.file_name()), depth.saturating_add(1)));
                            }
                            Ok(Some(_)) => {
                                self.failed(
                                    format!("task artifact {}", escaped_path(&source)),
                                    "directory entry limit reached",
                                );
                                break;
                            }
                            Ok(None) => break,
                            Err(error) => {
                                self.failed(format!("task artifact {}", escaped_path(&source)), error.to_string());
                                break;
                            }
                        }
                    },
                    Err(error) => self.failed(format!("task artifact {}", escaped_path(&source)), error.to_string()),
                }
            } else if metadata.is_file() {
                match bounded_bytes(&source).await {
                    Ok(bytes) if self.copied_bytes.saturating_add(bytes.len()) <= TOTAL_LIMIT => {
                        self.copied_bytes = self.copied_bytes.saturating_add(bytes.len());
                        self.questions(&source, task, &bytes);
                        self.activity(&source, task, &metadata);
                        self.add(target, bytes);
                    }
                    Ok(_) => self.failed(
                        format!("task artifact {}", escaped_path(&source)),
                        "total artifact byte limit reached",
                    ),
                    Err(error) => self.failed(format!("task artifact {}", escaped_path(&source)), error_chain(&error)),
                }
            } else {
                self.failed(
                    format!("task artifact {}", escaped_path(&source)),
                    "symlink or special file is not followed",
                );
            }
        }
    }

    fn activity(&mut self, source: &Path, task: &TaskId, metadata: &std::fs::Metadata) {
        if !source
            .file_name()
            .is_some_and(|name| name == "stdout" || name == "activity.jsonl")
        {
            return;
        }
        let modified = match metadata.modified() {
            Ok(value) => value,
            Err(error) => {
                self.failed(format!("output time {}", escaped_path(source)), error.to_string());
                return;
            }
        };
        let at = match Timestamp::from_utc(time::UtcDateTime::from(modified)) {
            Ok(at) => at,
            Err(error) => {
                self.failed(format!("output time {}", escaped_path(source)), error.to_string());
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
                self.failed(format!("questions {}", escaped_path(source)), error.to_string());
                return;
            }
        };
        let questions = match QuestionsFile::parse(text) {
            Ok(value) => value,
            Err(error) => {
                self.failed(format!("questions {}", escaped_path(source)), error.to_string());
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
                    self.failed(format!("gate {}", escaped_path(&path)), error.to_string());
                    None
                }
            },
            Err(ObservationError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                self.failed(format!("gate {}", escaped_path(&path)), error_chain(&error));
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
        outputs.sort_by(|(left, _), (right, _)| left.cmp(right));
        let tails = outputs
            .into_iter()
            .rev()
            .take(3)
            .map(|(_, bytes)| match std::str::from_utf8(bytes) {
                Ok(text) => text
                    .lines()
                    .rev()
                    .take(40)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n"),
                Err(error) => format!("gate output is not UTF-8: {error}; raw bytes are retained"),
            })
            .collect();
        self.summary.gate.push(TaskGateSummary {
            task_id: task.clone(),
            record,
            recent_output_tails: tails,
        });
    }

    async fn history(&mut self, home: &Home, team: &Team) -> Result<(), DebugRuntimeError> {
        let path = home.history_file();
        let id = team.id().clone();
        let result = tokio::task::spawn_blocking(move || {
            let reader = crate::history::Reader::open(&path)?;
            reader.team_records(&id)
        })
        .await
        .map_err(DebugRuntimeError::Worker)?;
        match result {
            Ok(records) if records.is_empty() => self.failed("team history", "no history records for this team"),
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
                self.add("history.txt", text.into_bytes());
                self.add(
                    "history.jsonl",
                    agent_core::story::json_lines(&records, &selection)
                        .join("\n")
                        .into_bytes(),
                );
            }
            Err(error) => self.failed("team history", error_chain(&error)),
        }
        Ok(())
    }

    async fn git(&mut self, home: &Home, team: &Team) {
        let tasks: Vec<_> = self.summary.tasks.iter().map(|task| task.id.clone()).collect();
        for task in tasks {
            let path = home.worktree_dir(team.repo(), &task);
            match read_git(&path, &task).await {
                Ok(git) => {
                    self.add(
                        PathBuf::from("git").join(format!("{task}.txt")),
                        format!("{git:#?}").into_bytes(),
                    );
                    self.summary.git.push(git);
                }
                Err(error) => self.failed(format!("worktree {}", escaped_path(&path)), error_chain(&error)),
            }
        }
        self.git_details(home, team).await;
    }

    async fn git_details(&mut self, home: &Home, team: &Team) {
        let lead_tip = self
            .summary
            .git
            .iter()
            .find(|git| git.task_id == self.assessment.lead)
            .map(|git| git.tip.clone());
        let lead_path = home.worktree_dir(team.repo(), &self.assessment.lead);
        for index in 0..self.summary.git.len() {
            let Some(git) = self.summary.git.get(index) else {
                continue;
            };
            let task = git.task_id.clone();
            let tip = git.tip.clone();
            let contains = match &lead_tip {
                Some(lead) => match git_capture(
                    &lead_path,
                    &["merge-base", "--is-ancestor", tip.as_str(), lead.as_str()],
                )
                .await
                {
                    Ok(output) => match output.exit {
                        ExitReport::Exited { code: 0 } => Some(true),
                        ExitReport::Exited { code: 1 } => Some(false),
                        other => {
                            self.failed(format!("git containment {task}"), format!("git exited {other:?}"));
                            None
                        }
                    },
                    Err(error) => {
                        self.failed(format!("git containment {task}"), error_chain(&error));
                        None
                    }
                },
                None => None,
            };
            if let Some(git) = self.summary.git.get_mut(index) {
                git.lead_contains_tip = contains;
                self.assessment.branches.push(Branch {
                    task: task.clone(),
                    name: git.branch.clone(),
                    tip: git.tip.clone(),
                    origin: Some(git.origin_tip.clone().map_or(Origin::Missing, Origin::Tip)),
                    contained_in_lead: contains,
                });
                if task == self.assessment.lead {
                    self.assessment.lead_git = Some(LeadGit {
                        tip,
                        dirty: !git.uncommitted_files.is_empty(),
                        gate: self
                            .summary
                            .gate
                            .iter()
                            .find(|gate| gate.task_id == task)
                            .and_then(|gate| gate.record.clone()),
                    });
                }
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
                    self.failed("git base configuration", error_chain(&error));
                    return;
                }
            },
            Err(error) => {
                self.failed("git base configuration", error_chain(&error));
                return;
            }
        };
        let Some(base) = base else {
            self.failed("git base configuration", "repository has no configured base branch");
            return;
        };
        let tasks: Vec<_> = self.summary.git.iter().map(|git| git.task_id.clone()).collect();
        for task in tasks {
            let path = home.worktree_dir(team.repo(), &task);
            match git_text(
                &path,
                &[
                    "log",
                    "--format=%H%x00%aI%x00%an%x00%s",
                    "--no-decorate",
                    &format!("{base}..HEAD"),
                ],
            )
            .await
            {
                Ok(text) => self.add(
                    PathBuf::from("git").join(format!("{task}-commits.txt")),
                    text.into_bytes(),
                ),
                Err(error) => self.failed(format!("git commits {task}"), error_chain(&error)),
            }
        }
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
    #[error("read-only git command exited {exit:?}: {stderr:?}")]
    Git {
        /// Checked process exit.
        exit: ExitReport,
        /// Bounded error bytes, escaped without losing data.
        stderr: Vec<u8>,
    },
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

async fn git_capture(path: &Path, args: &[&str]) -> Result<crate::process::Captured, ObservationError> {
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
    let result = git_capture(path, args).await?;
    match result.exit {
        ExitReport::Exited { code: 0 } => String::from_utf8(result.stdout.bytes).map_err(ObservationError::Utf8),
        exit => Err(ObservationError::Git {
            exit,
            stderr: result.stderr.bytes,
        }),
    }
}

async fn read_git(path: &Path, task: &TaskId) -> Result<GitWorktreeSummary, ObservationError> {
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
    let status = git_text(path, &["status", "--porcelain=v1", "--untracked-files=all"]).await?;
    let harness = git_text(
        path,
        &["for-each-ref", "--format=%(refname) %(objectname)", "refs/team-agents/"],
    )
    .await?;
    Ok(GitWorktreeSummary {
        task_id: task.clone(),
        branch,
        tip,
        origin_tip,
        commits_over_base: vec![],
        uncommitted_files: status.lines().map(PathBuf::from).collect(),
        harness_refs: harness.lines().map(str::to_owned).collect(),
        lead_contains_tip: None,
        newest_untracked: vec![],
    })
}

pub(super) async fn persist(
    collection: Collection,
    cut: SecretsCut,
    parent: PathBuf,
    prefix: String,
) -> Result<DebugResult, DebugRuntimeError> {
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        use std::os::unix::fs::DirBuilderExt;
        let artifacts: Vec<_> = collection
            .artifacts
            .into_iter()
            .map(|(path, bytes)| {
                (
                    PathBuf::from(OsString::from_vec(cut.redact(path.as_os_str().as_encoded_bytes()))),
                    cut.redact(&bytes),
                )
            })
            .collect();
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

/// Escapes paths without losing non-UTF-8 bytes or allowing control characters to hide file names.
#[expect(
    clippy::unnecessary_debug_formatting,
    reason = "debug preserves invalid UTF-8 and escapes control bytes in diagnostic paths"
)]
fn escaped_path(path: &Path) -> String {
    format!("{:?}", path.as_os_str())
}
