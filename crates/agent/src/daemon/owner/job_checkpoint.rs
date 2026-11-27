//! The checkpoints of the jobs that are no item of a round: a worker's request, a review and an investigation.
//!
//! A job is registered when it starts ([`Owner::job_started`]) with what is meant by it and where its worktree started;
//! from then on a step of it ([`Owner::job_checkpoint`]) takes a snapshot of its worktree and records what is meant, what
//! is done and what is left. Whoever takes the job over reads the last of them ([`job_briefing`]). A checkpoint that cannot
//! be taken is logged as `checkpoint_failed` and never stops the job.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use agent_core::config::RepoName;
use agent_core::history::{
    CheckResult, CheckpointCause, CheckpointContent, CheckpointJob, CommitLine, EventRecord, HistoryEvent, JobWork,
    Record, ReviewedItem, last_check_of, review_meant,
};
use agent_core::ids::{RoundId, TaskId, TeamId};
use agent_core::kinds::Tier;
use agent_core::log::LogEvent;
use agent_core::prompt::{HarnessPrompt, Variables};
use agent_core::state::{CommitId, ItemPhase, PhaseKind, PrNumber, ReviewOutcome, Timestamp};
use agent_runtime::git::{CommitIdentity, Git, GitError, Revision};
use agent_runtime::harness_prompts::HarnessPrompts;
use agent_runtime::history::{LastCheckpoint, Reader};
use agent_runtime::process::error_chain;
use agent_runtime::run::compose::{CheckpointFacts, checkpoint_state_variables};
use agent_runtime::snapshot::{WorkSpan, take_checkpoint_snapshot, work_since};
use agent_runtime::store::Home;

use super::Owner;
use super::checkpoint::{LOOK_SECONDS, LastCheckpoint as Kept};
use super::rounds::prompts::SectionError;
use super::snapshots::validate_worktree_path;

/// What the owner knows of a job that keeps checkpoints, from its start.
#[derive(Debug, Clone)]
pub(super) struct JobFacts {
    /// The team of the round the job works for.
    pub(super) team: TeamId,
    /// The round the job works for.
    pub(super) round: RoundId,
    /// What kind of job it is; never [`CheckpointJob::Item`].
    pub(super) job: CheckpointJob,
    /// The tier of the task that does it.
    pub(super) tier: Tier,
    /// The task that does it.
    pub(super) task: TaskId,
    /// The worktree of the task.
    pub(super) worktree: PathBuf,
    /// What is to be done, as the checkpoint tells it.
    pub(super) meant: String,
    /// The commit the worktree started at: what the job did is what came after it.
    pub(super) start: CommitId,
    /// The pull request under review, for a review.
    pub(super) pull_request: Option<PrNumber>,
    /// The review pass, for a review.
    pub(super) review_pass: Option<u32>,
    /// The file the reviewer writes its findings to, for a review.
    pub(super) findings_file: Option<PathBuf>,
    /// The findings the file held at the last checkpoint, for a review.
    pub(super) findings_seen: String,
    /// The last result of the check of a worker's request, when it ran.
    pub(super) last_check: Option<(bool, String)>,
}

/// The round whose jobs are read back after a start of the daemon, with its team and the repository of the team.
struct JobsOf<'a> {
    /// The team of the round.
    team: &'a TeamId,
    /// The round.
    round: &'a RoundId,
    /// The repository of the team, whose worktrees the jobs have.
    repo: &'a RepoName,
}

/// What the checkpoints of one task, read in order, add up to: where its worktree started, what it meant, and the last
/// of what each kind of checkpoint told.
#[derive(Default)]
struct Seen<'a> {
    /// Whether a checkpoint that was no change of holder was seen: the first of them tells where the worktree started.
    begun: bool,
    /// The head of the first checkpoint that was no change of holder: where the worktree started.
    start: Option<&'a CommitId>,
    /// The last checkpoint of the task.
    last: Option<&'a LastCheckpoint>,
    /// What the newest checkpoint with a content says is meant.
    meant: Option<&'a str>,
    /// What the newest checkpoint with a content says is done.
    done: Option<&'a str>,
    /// The reference of the newest snapshot.
    snapshot: Option<&'a str>,
}

impl<'a> Seen<'a> {
    /// Adds the next checkpoint of the task.
    fn take(&mut self, checkpoint: &'a LastCheckpoint) {
        if !self.begun && checkpoint.cause != CheckpointCause::HolderChanged {
            self.begun = true;
            self.start = checkpoint.head.as_ref();
        }
        self.last = Some(checkpoint);
        if let Some(content) = &checkpoint.content {
            self.meant = Some(&content.meant);
            self.done = Some(&content.done);
        }
        if let Some(snapshot) = &checkpoint.snapshot {
            self.snapshot = Some(snapshot);
        }
    }
}

/// Whether a job whose last checkpoint is of `cause` has ended: no run of it is going, and none is started again.
fn is_an_end(cause: CheckpointCause) -> bool {
    match cause {
        CheckpointCause::RunEnded
        | CheckpointCause::ReviewSubmitted
        | CheckpointCause::InvestigationAnswered
        | CheckpointCause::HolderChanged => true,
        CheckpointCause::RoundOpened
        | CheckpointCause::Commit
        | CheckpointCause::PullOpened
        | CheckpointCause::PullPushed
        | CheckpointCause::PullReady
        | CheckpointCause::Checked
        | CheckpointCause::SplitDecided
        | CheckpointCause::FixPassStarted
        | CheckpointCause::FindingAnswered
        | CheckpointCause::FindingJudged
        | CheckpointCause::WaitsForHuman
        | CheckpointCause::QuestionAsked
        | CheckpointCause::QuestionAnswered
        | CheckpointCause::PullApproved
        | CheckpointCause::MergeRefused
        | CheckpointCause::PullMerged
        | CheckpointCause::PullHandedOver
        | CheckpointCause::PullClosed
        | CheckpointCause::RoundOverdue
        | CheckpointCause::DaemonStarted
        | CheckpointCause::RequestStarted
        | CheckpointCause::RequestChecked
        | CheckpointCause::ReviewStarted
        | CheckpointCause::FindingsWritten
        | CheckpointCause::InvestigationStarted => false,
    }
}

/// The first seven characters of a commit's hash: how a checkpoint names a commit.
fn short(hash: &str) -> String {
    hash.chars().take(7).collect()
}

impl Owner {
    /// Registers the job and takes its first checkpoint, of `cause`.
    pub(super) async fn job_started(&mut self, facts: JobFacts, cause: CheckpointCause) {
        let task = facts.task.clone();
        self.checkpoints.jobs.insert(task.clone(), facts);
        self.job_checkpoint(&task, cause).await;
    }

    /// Registers the review the reviewer `task` is about to make and takes its first checkpoint; a reviewer that is
    /// registered already, or has no review pending, is left alone. A fact that cannot be read is logged and the review
    /// has no checkpoints.
    pub(super) async fn review_job_started(&mut self, task: &TaskId, team_id: &TeamId) {
        if self.checkpoints.jobs.contains_key(task) {
            return;
        }
        let Some(team) = self.parts.state.team(team_id) else {
            return;
        };
        let Some((progress, pr, reviewed)) = team.items().iter().find_map(|progress| match progress.phase() {
            ItemPhase::InReview { pr, reviews }
                if reviews
                    .iter()
                    .any(|review| review.task == *task && matches!(review.outcome, ReviewOutcome::Pending)) =>
            {
                Some((progress, pr.number, progress.completed_review_passes()))
            }
            ItemPhase::Pending
            | ItemPhase::InProgress { .. }
            | ItemPhase::InReview { .. }
            | ItemPhase::Fixing { .. }
            | ItemPhase::Approved { .. }
            | ItemPhase::Done { .. } => None,
        }) else {
            return;
        };
        let (Some(listed), Some(round)) = (
            team.task_list().list().item(progress.id()),
            team.open_rounds().find(|round| round.item() == Some(progress.id())),
        ) else {
            return;
        };
        let worktree = self.parts.store.home().worktree_dir(team.repo(), task);
        let head = match worktree_head(&worktree).await {
            Ok(head) => head,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::CheckpointFailed.as_str(),
                    team = %team_id,
                    task = %task,
                    cause = %CheckpointCause::ReviewStarted,
                    error = %error_chain(&error),
                    "the head of a review's worktree cannot be read: the review has no checkpoints"
                );
                return;
            }
        };
        let Ok(pass) = u32::try_from(reviewed.saturating_add(1)) else {
            tracing::warn!(
                event = LogEvent::CheckpointFailed.as_str(),
                team = %team_id,
                task = %task,
                cause = %CheckpointCause::ReviewStarted,
                error = "the review pass is too large for a checkpoint",
                "the review has no checkpoints"
            );
            return;
        };
        let requirements = listed.requirements();
        let item = ReviewedItem {
            id: progress.id(),
            title: &listed.title,
            requirements: &requirements,
        };
        let facts = JobFacts {
            team: team_id.clone(),
            round: round.id().clone(),
            job: CheckpointJob::Review,
            tier: Tier::Investigator,
            task: task.clone(),
            worktree,
            meant: review_meant(pr, &short(head.as_str()), pass, &item),
            start: head,
            pull_request: Some(pr),
            review_pass: Some(pass),
            findings_file: Some(self.parts.store.home().review_findings_file(task, pr, pass)),
            findings_seen: String::new(),
            last_check: None,
        };
        self.job_started(facts, CheckpointCause::ReviewStarted).await;
    }

    /// Takes the checkpoint of `cause` for the job of `task`; a task that is no job is left alone.
    pub(super) async fn job_checkpoint(&mut self, task: &TaskId, cause: CheckpointCause) {
        if self.history.is_disabled() {
            return;
        }
        let Some(mut facts) = self.checkpoints.jobs.get(task).cloned() else {
            return;
        };
        let at = match Timestamp::now(&self.parts.clock) {
            Ok(at) => at,
            Err(error) => {
                Self::clock_failed(&facts.team, &facts.round, cause, &error_chain(&error));
                return;
            }
        };
        let key = (facts.round.clone(), task.clone());
        let previous = self.checkpoints.last.get(&key).and_then(|last| last.snapshot.clone());
        let taken = match validate_worktree_path(&facts.worktree) {
            Ok(worktree) => take_checkpoint_snapshot(&worktree, &facts.round, task, previous.as_deref())
                .await
                .map_err(|error| error_chain(&error)),
            Err(error) => Err(error),
        };
        if cause == CheckpointCause::DaemonStarted
            && let Ok(taken) = &taken
            && previous.as_deref() == Some(taken.reference.as_str())
        {
            // The worktree is as the last checkpoint kept it: a start of the daemon changed nothing to keep.
            return;
        }
        let (head, tree, snapshot) = match taken {
            Ok(taken) => {
                self.checkpoints.last.insert(
                    key,
                    Kept {
                        head: Some(taken.snapshot.head.clone()),
                        snapshot: Some(taken.reference.clone()),
                    },
                );
                (
                    Some(taken.snapshot.head),
                    Some(taken.snapshot.commit),
                    Some(taken.reference),
                )
            }
            Err(error) => {
                Self::job_failed(&facts, cause, &error, "a checkpoint has no snapshot of the worktree");
                (None, None, None)
            }
        };
        let content = match (&head, &tree) {
            (Some(head), Some(tree)) => self.job_content(&mut facts, cause, head, tree).await,
            _ => None,
        };
        self.checkpoints.jobs.insert(task.clone(), facts.clone());
        let event = HistoryEvent::Checkpoint {
            round: facts.round.clone(),
            item: None,
            task: facts.task,
            job: facts.job,
            tier: facts.tier,
            cause,
            pull_request: facts.pull_request,
            head,
            draft: false,
            phase: PhaseKind::InProgress,
            review_pass: facts.review_pass,
            fix_pass: None,
            snapshot,
            content,
        };
        let round = facts.round.clone();
        self.history.record(Record::Event(Box::new(EventRecord {
            at,
            team: facts.team,
            rounds: vec![facts.round],
            event,
        })));
        // Whoever takes the job over reads this checkpoint from the history: it is there before the owner goes on.
        self.wait_for_history(&round).await;
    }

    /// After a start of the daemon: registers again the jobs of the open rounds that were going when it ended, from their
    /// last checkpoints, and takes a `daemon_started` checkpoint of each whose worktree is not as that checkpoint kept it.
    /// A history that cannot be read is logged and leaves the jobs of that round without one.
    pub(super) async fn jobs_daemon_started(&mut self) {
        let history_file = self.parts.store.home().history_file();
        let mut going = Vec::new();
        for team in self.parts.state.teams() {
            for round in team.open_rounds() {
                let kept =
                    match Reader::open(&history_file).and_then(|reader| reader.job_checkpoints_of_round(round.id())) {
                        Ok(kept) => kept,
                        Err(error) => {
                            tracing::warn!(
                                event = LogEvent::CheckpointFailed.as_str(),
                                team = %team.id(),
                                round = %round.id(),
                                cause = %CheckpointCause::DaemonStarted,
                                error = %error_chain(&error),
                                "the checkpoints of the jobs of an open round cannot be read at the start of the daemon"
                            );
                            continue;
                        }
                    };
                let whose = JobsOf {
                    team: team.id(),
                    round: round.id(),
                    repo: team.repo(),
                };
                going.extend(going_jobs(self.parts.store.home(), &whose, &kept));
            }
        }
        for (facts, last) in going {
            let task = facts.task.clone();
            self.checkpoints.last.insert((facts.round.clone(), task.clone()), last);
            self.checkpoints.jobs.insert(task.clone(), facts);
            self.job_checkpoint(&task, CheckpointCause::DaemonStarted).await;
        }
    }

    /// Hands the job of the task `from` over to the task `to`: `to` gets a `holder_changed` checkpoint that tells what the
    /// last checkpoint of `from` told, and its run is briefed from it. Nothing is handed over when `from` left no
    /// checkpoint with work in it. A history that cannot be read is logged and leaves the job without a hand-over.
    pub(super) async fn job_handed_over(&mut self, from: &TaskId, to: &TaskId, team: &TeamId, rounds: Vec<RoundId>) {
        if self.history.is_disabled() {
            return;
        }
        let history = self.parts.store.home().history_file();
        let (lookup, holder) = (rounds, from.clone());
        let read = tokio::task::spawn_blocking(move || {
            if !history.exists() {
                return Ok(None);
            }
            newest_checkpoint_of(&Reader::open(&history)?, &lookup, &holder)
        })
        .await;
        let found = match read {
            Ok(Ok(Some(found))) => found,
            Ok(Ok(None)) => return,
            Ok(Err(error)) => {
                Self::handover_failed(team, to, &error_chain(&error));
                return;
            }
            Err(error) => {
                Self::handover_failed(team, to, &error_chain(&error));
                return;
            }
        };
        let (round, last) = found;
        let at = match Timestamp::now(&self.parts.clock) {
            Ok(at) => at,
            Err(error) => {
                Self::clock_failed(team, &round, CheckpointCause::HolderChanged, &error_chain(&error));
                return;
            }
        };
        let event = HistoryEvent::Checkpoint {
            round: round.clone(),
            item: None,
            task: to.clone(),
            job: last.job,
            tier: last.tier,
            cause: CheckpointCause::HolderChanged,
            pull_request: last.pull_request,
            head: last.head,
            draft: false,
            phase: PhaseKind::InProgress,
            review_pass: last.review_pass,
            fix_pass: None,
            snapshot: last.snapshot,
            content: last.content,
        };
        self.history.record(Record::Event(Box::new(EventRecord {
            at,
            team: team.clone(),
            rounds: vec![round.clone()],
            event,
        })));
        // The run of `to` is briefed from this checkpoint, read from the history: it is there before the owner goes on.
        self.wait_for_history(&round).await;
    }

    /// Logs that the hand-over of a job to `to` has no checkpoint, for the `error` told.
    fn handover_failed(team: &TeamId, to: &TaskId, error: &str) {
        tracing::warn!(
            event = LogEvent::CheckpointFailed.as_str(),
            team = %team,
            task = %to,
            cause = %CheckpointCause::HolderChanged,
            error = %error,
            "the last checkpoint of the job handed over cannot be read: its new holder starts without a briefing"
        );
    }

    /// Takes the checkpoint of the end of the run of the job of `task`, of `cause`, and forgets the job: no run of the task
    /// follows, a job that goes on is another task's.
    pub(super) async fn job_ended(&mut self, task: &TaskId, cause: CheckpointCause) {
        self.job_checkpoint(task, cause).await;
        self.checkpoints.jobs.remove(task);
    }

    /// Logs that a checkpoint of a job lacks what `what` says.
    fn job_failed(facts: &JobFacts, cause: CheckpointCause, error: &str, what: &str) {
        tracing::warn!(
            event = LogEvent::CheckpointFailed.as_str(),
            team = %facts.team,
            round = %facts.round,
            task = %facts.task,
            cause = %cause,
            error = %error,
            "{what}: the job goes on"
        );
    }

    /// What the checkpoint says of the job: what is meant, what its worktree holds, what is left. `None`, logged, when a
    /// fact cannot be read.
    async fn job_content(
        &self,
        facts: &mut JobFacts,
        cause: CheckpointCause,
        head: &CommitId,
        tree: &CommitId,
    ) -> Option<CheckpointContent> {
        let worktree = match validate_worktree_path(&facts.worktree) {
            Ok(worktree) => worktree,
            Err(error) => {
                Self::job_failed(facts, cause, &error, "a checkpoint has no content");
                return None;
            }
        };
        let span = WorkSpan {
            start: &facts.start,
            head,
            tree,
        };
        let work = match work_since(&worktree, &facts.task, &span).await {
            Ok(work) => work,
            Err(error) => {
                Self::job_failed(facts, cause, &error_chain(&error), "a checkpoint has no content");
                return None;
            }
        };
        let commits: Vec<CommitLine> = work
            .commits
            .iter()
            .map(|commit| CommitLine {
                id: short(commit.id.as_str()),
                step: agent_core::decide::commit_guard::step_of_message(&commit.message),
            })
            .collect();
        let check = facts.last_check.clone();
        let findings = match &facts.findings_file {
            Some(file) => match read_findings(file).await {
                Ok(text) => text,
                Err(error) => {
                    Self::job_failed(facts, cause, &error_chain(&error), "a checkpoint has no findings");
                    return None;
                }
            },
            None => String::new(),
        };
        facts.findings_seen.clone_from(&findings);
        let work = match facts.job {
            CheckpointJob::WorkerRequest => JobWork::Request {
                last_check: check
                    .as_ref()
                    .map(|(passed, line)| CheckResult { passed: *passed, line }),
                commits: &commits,
                changed: &work.changed,
            },
            CheckpointJob::Review => {
                let Some(pass) = facts.review_pass else {
                    Self::job_failed(facts, cause, "the review has no pass", "a checkpoint has no content");
                    return None;
                };
                JobWork::Review {
                    pass,
                    findings: &findings,
                }
            }
            CheckpointJob::Investigation | CheckpointJob::Item => JobWork::Investigation {
                commits: &commits,
                changed: &work.changed,
            },
        };
        Some(agent_core::history::job_content(facts.meant.clone(), &work))
    }

    /// Looks at the jobs of `team` that have a task going, once a minute each: a `commit` checkpoint when the head of the
    /// worktree is not the one of the last checkpoint, a `findings_written` one when a reviewer's findings file changed.
    pub(super) async fn look_at_jobs(&mut self, team: &TeamId) {
        let tasks: Vec<TaskId> = self
            .checkpoints
            .jobs
            .values()
            .filter(|facts| facts.team == *team)
            .map(|facts| facts.task.clone())
            .collect();
        for task in tasks {
            self.look_at_job(&task).await;
        }
    }

    /// One look at the job of `task`.
    async fn look_at_job(&mut self, task: &TaskId) {
        let Some(facts) = self.checkpoints.jobs.get(task).cloned() else {
            return;
        };
        let now = match Timestamp::now(&self.parts.clock) {
            Ok(now) => now,
            Err(error) => {
                Self::clock_failed(&facts.team, &facts.round, CheckpointCause::Commit, &error_chain(&error));
                return;
            }
        };
        match self.checkpoints.looked_at.get(task) {
            // The first sighting only starts the minute: the checkpoint of the job's start was taken just before it.
            None => {
                self.checkpoints.looked_at.insert(task.clone(), now);
                return;
            }
            Some(last) if (now.as_utc() - last.as_utc()).whole_seconds() < LOOK_SECONDS => return,
            Some(_) => {}
        }
        self.checkpoints.looked_at.insert(task.clone(), now);
        if let Some(file) = &facts.findings_file {
            match read_findings(file).await {
                Ok(text) if text != facts.findings_seen && !text.trim().is_empty() => {
                    self.job_checkpoint(task, CheckpointCause::FindingsWritten).await;
                    return;
                }
                Ok(_) => {}
                // The next look tries again: a file that cannot be read now is no change, but a checkpoint may be lost.
                Err(error) => Self::job_failed(
                    &facts,
                    CheckpointCause::FindingsWritten,
                    &error_chain(&error),
                    "the findings file of a review cannot be read",
                ),
            }
        }
        match worktree_head(&facts.worktree).await {
            Ok(head) => {
                let kept = self.checkpoints.last.get(&(facts.round.clone(), task.clone()));
                if kept.is_some_and(|kept| kept.head.as_ref() != Some(&head)) {
                    self.job_checkpoint(task, CheckpointCause::Commit).await;
                }
            }
            Err(error) => tracing::warn!(
                event = LogEvent::GitCallFailed.as_str(),
                task = %task,
                operation = "checkpoint_look",
                error = %error_chain(&error),
                "the worktree of a job was not looked at for a new commit"
            ),
        }
    }
}

/// Why the head of a job's worktree cannot be read.
#[derive(Debug, thiserror::Error)]
enum WorktreeHeadError {
    /// The worktree path is no path git can be run in.
    #[error("{reason}")]
    Path {
        /// What is wrong with the path.
        reason: String,
    },
    /// Git could not say what the head is.
    #[error("git cannot read the head of the worktree")]
    Git(#[source] GitError),
}

/// The commit the worktree at `path` has out.
///
/// # Errors
/// [`WorktreeHeadError::Path`] when the path is no worktree path, [`WorktreeHeadError::Git`] when git cannot read its head.
async fn worktree_head(path: &Path) -> Result<CommitId, WorktreeHeadError> {
    let worktree = validate_worktree_path(path).map_err(|reason| WorktreeHeadError::Path { reason })?;
    Git::new(
        worktree,
        CommitIdentity::new(LOOK_NAME.to_owned(), LOOK_EMAIL.to_owned(), None),
    )
    .rev_parse(&Revision::Head)
    .await
    .map_err(WorktreeHeadError::Git)
}

/// The name git is given for the look at a worktree's head, which commits nothing.
const LOOK_NAME: &str = "team-agents";

/// The e-mail address git is given for the look at a worktree's head, which commits nothing.
const LOOK_EMAIL: &str = "team-agents@team-agents.invalid";

/// The findings a reviewer has written to `file`; none before the file exists.
///
/// # Errors
/// The [`std::io::Error`] of a file that exists and cannot be read.
async fn read_findings(file: &Path) -> Result<String, std::io::Error> {
    match tokio::fs::read_to_string(file).await {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error),
    }
}

/// The jobs `kept` (the checkpoints of the jobs of one round, oldest first) leaves going: each with the facts the
/// registry holds of a job and the last checkpoint it kept. A job whose last checkpoint ended it, or tells nothing of
/// what is meant or where its worktree started, is not going: the latter is logged.
fn going_jobs(home: &Home, whose: &JobsOf<'_>, kept: &[LastCheckpoint]) -> Vec<(JobFacts, Kept)> {
    let mut order: Vec<&TaskId> = Vec::new();
    let mut seen: HashMap<&TaskId, Seen<'_>> = HashMap::new();
    for checkpoint in kept {
        let entry = seen.entry(&checkpoint.task).or_insert_with(|| {
            order.push(&checkpoint.task);
            Seen::default()
        });
        entry.take(checkpoint);
    }
    let mut going = Vec::new();
    for task in order {
        let Some(seen) = seen.remove(task) else {
            continue;
        };
        let Some(last) = seen.last else {
            continue;
        };
        if is_an_end(last.cause) {
            continue;
        }
        let (Some(start), Some(meant)) = (seen.start, seen.meant) else {
            tracing::warn!(
                event = LogEvent::CheckpointFailed.as_str(),
                team = %whose.team,
                round = %whose.round,
                task = %task,
                cause = %CheckpointCause::DaemonStarted,
                error = "the first checkpoint of the job has no head, or none tells what is meant",
                "the job has no checkpoint at the start of the daemon"
            );
            continue;
        };
        let findings_file = match (last.job, last.pull_request, last.review_pass) {
            (CheckpointJob::Review, Some(pr), Some(pass)) => Some(home.review_findings_file(task, pr, pass)),
            _ => None,
        };
        let last_check = match last.job {
            CheckpointJob::WorkerRequest => seen.done.and_then(last_check_of),
            CheckpointJob::Review | CheckpointJob::Investigation | CheckpointJob::Item => None,
        };
        let facts = JobFacts {
            team: whose.team.clone(),
            round: whose.round.clone(),
            job: last.job,
            tier: last.tier,
            task: task.clone(),
            worktree: home.worktree_dir(whose.repo, task),
            meant: meant.to_owned(),
            start: start.clone(),
            pull_request: last.pull_request,
            review_pass: last.review_pass,
            findings_file,
            findings_seen: String::new(),
            last_check,
        };
        going.push((
            facts,
            Kept {
                head: last.head.clone(),
                snapshot: seen.snapshot.map(str::to_owned),
            },
        ));
    }
    going
}

/// Which job a run takes over, and whose checkpoint it starts from: the checkpoint of its own task, which the hand-over
/// of an earlier holder wrote ([`Owner::job_handed_over`]) or the task's own earlier run left.
#[derive(Debug, Clone, Copy)]
pub(super) enum TakenOver<'a> {
    /// A worker's request: its task's worktree is kept for the next worker.
    Request {
        /// The worker's task.
        task: &'a TaskId,
    },
    /// A review.
    Review {
        /// The task of the reviewer that takes over.
        task: &'a TaskId,
        /// The pull request under review.
        pull_request: PrNumber,
        /// The review pass.
        pass: u32,
    },
    /// An investigation.
    Investigation {
        /// The task of the investigator that takes over.
        task: &'a TaskId,
    },
}

impl TakenOver<'_> {
    /// The kind of job and the task whose checkpoint it starts from.
    fn job_and_task(&self) -> (CheckpointJob, &TaskId) {
        match self {
            Self::Request { task } => (CheckpointJob::WorkerRequest, task),
            Self::Review { task, .. } => (CheckpointJob::Review, task),
            Self::Investigation { task } => (CheckpointJob::Investigation, task),
        }
    }
}

/// Whether a checkpoint of `cause` is the start of a job: it tells where the worktree began and nothing of the work.
fn is_a_start(cause: CheckpointCause) -> bool {
    matches!(
        cause,
        CheckpointCause::RequestStarted | CheckpointCause::ReviewStarted | CheckpointCause::InvestigationStarted
    )
}

/// The newest checkpoint of the job of `task` in `rounds` that tells some work, with the round it was taken in.
fn newest_checkpoint_of(
    reader: &Reader,
    rounds: &[RoundId],
    task: &TaskId,
) -> Result<Option<(RoundId, LastCheckpoint)>, agent_runtime::history::ReadError> {
    let mut newest = None;
    for round in rounds {
        for kept in reader.job_checkpoints_of_round(round)? {
            if kept.task == *task && !is_a_start(kept.cause) {
                newest = Some((round.clone(), kept));
            }
        }
    }
    Ok(newest)
}

/// The briefing a job taken over starts with: the last checkpoint of the holder before, told as `prompt` asks; `None`
/// when no holder left one. A history that cannot be read is logged and leaves the job without a briefing.
///
/// # Errors
/// [`SectionError`] when the briefing cannot be rendered.
pub(super) async fn job_briefing(
    texts: &HarnessPrompts,
    history: PathBuf,
    rounds: Vec<RoundId>,
    taken_over: TakenOver<'_>,
) -> Result<Option<String>, SectionError> {
    let path = history;
    if !path.exists() {
        return Ok(None);
    }
    let (named, lookup) = (taken_over, rounds);
    let (job, task) = named.job_and_task();
    let (job, task) = (job, task.clone());
    let read = tokio::task::spawn_blocking(move || {
        let reader = Reader::open(&path)?;
        Ok::<_, agent_runtime::history::ReadError>(
            newest_checkpoint_of(&reader, &lookup, &task)?
                .map(|(_, kept)| kept)
                .filter(|kept| kept.job == job),
        )
    })
    .await;
    let kept = match read {
        Ok(Ok(kept)) => kept,
        Ok(Err(error)) => {
            tracing::warn!(
                event = LogEvent::CheckpointFailed.as_str(),
                cause = "briefing_read",
                error = %error_chain(&error),
                "the last checkpoint of a job taken over cannot be read: it starts without a briefing"
            );
            return Ok(None);
        }
        Err(error) => {
            tracing::warn!(
                event = LogEvent::CheckpointFailed.as_str(),
                cause = "briefing_read",
                error = %error_chain(&error),
                "the read of the last checkpoint of a job taken over failed: it starts without a briefing"
            );
            return Ok(None);
        }
    };
    let Some(kept) = kept else {
        return Ok(None);
    };
    let state = checkpoint_state_variables(&CheckpointFacts {
        cause: kept.cause,
        at: kept.at,
        head: kept.head.as_ref(),
        content: kept.content.as_ref(),
    })
    .map_err(SectionError::Variable)?;
    let checkpoint = texts.render(HarnessPrompt::CheckpointState, &state)?;
    let (prompt, values) = match named {
        TakenOver::Request { .. } => (HarnessPrompt::BriefingWorker, Variables::new()),
        TakenOver::Review { pull_request, pass, .. } => (
            HarnessPrompt::BriefingReview,
            Variables::new()
                .with("pull_request", pull_request.to_string())?
                .with("pass", pass.to_string())?,
        ),
        TakenOver::Investigation { .. } => (HarnessPrompt::BriefingInvestigation, Variables::new()),
    };
    let values = values.with("checkpoint", checkpoint.trim_end())?;
    Ok(Some(texts.render(prompt, &values)?.trim_end().to_owned()))
}

#[cfg(test)]
mod tests;
