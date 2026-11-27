//! The checkpoints of the jobs that are no item of a round: a worker's request, a review and an investigation.
//!
//! A job is registered when it starts ([`Owner::job_started`]) with what is meant by it and where its worktree started;
//! from then on a step of it ([`Owner::job_checkpoint`]) takes a snapshot of its worktree and records what is meant, what
//! is done and what is left. Whoever takes the job over reads the last of them ([`job_briefing`]). A checkpoint that cannot
//! be taken is logged as `checkpoint_failed` and never stops the job.

use std::path::PathBuf;

use agent_core::history::{
    CheckResult, CheckpointCause, CheckpointContent, CheckpointJob, CommitLine, EventRecord, HistoryEvent, JobWork,
    Record, ReviewedItem, review_meant,
};
use agent_core::ids::{RoundId, TaskId, TeamId};
use agent_core::kinds::Tier;
use agent_core::log::LogEvent;
use agent_core::prompt::{HarnessPrompt, Variables};
use agent_core::state::{CommitId, ItemPhase, PhaseKind, PrNumber, ReviewOutcome, Timestamp};
use agent_runtime::git::{CommitIdentity, Git, Revision};
use agent_runtime::harness_prompts::HarnessPrompts;
use agent_runtime::history::{LastCheckpoint, Reader};
use agent_runtime::process::error_chain;
use agent_runtime::run::compose::{CheckpointFacts, checkpoint_state_variables};
use agent_runtime::snapshot::{WorkSpan, take_checkpoint_snapshot, work_since};

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
                    error = %error,
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
                    Self::job_failed(facts, cause, &error, "a checkpoint has no findings");
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
            CheckpointJob::Review => JobWork::Review {
                pass: facts.review_pass.unwrap_or(1),
                findings: &findings,
            },
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
            // A file that cannot be read is no change: the next look tries again.
            if let Ok(text) = read_findings(file).await
                && text != facts.findings_seen
                && !text.trim().is_empty()
            {
                self.job_checkpoint(task, CheckpointCause::FindingsWritten).await;
                return;
            }
        }
        let head = match validate_worktree_path(&facts.worktree) {
            Ok(worktree) => Git::new(
                worktree,
                CommitIdentity::new(LOOK_NAME.to_owned(), LOOK_EMAIL.to_owned(), None),
            )
            .rev_parse(&Revision::Head)
            .await
            .map_err(|error| error_chain(&error)),
            Err(error) => Err(error),
        };
        match head {
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
                error = %error,
                "the worktree of a job was not looked at for a new commit"
            ),
        }
    }
}

/// The commit the worktree at `path` has out.
///
/// # Errors
/// The cause, when the path is no worktree or git cannot read its head.
async fn worktree_head(path: &std::path::Path) -> Result<CommitId, String> {
    let worktree = validate_worktree_path(path)?;
    Git::new(
        worktree,
        CommitIdentity::new(LOOK_NAME.to_owned(), LOOK_EMAIL.to_owned(), None),
    )
    .rev_parse(&Revision::Head)
    .await
    .map_err(|error| error_chain(&error))
}

/// The name git is given for the look at a worktree's head, which commits nothing.
const LOOK_NAME: &str = "team-agents";

/// The e-mail address git is given for the look at a worktree's head, which commits nothing.
const LOOK_EMAIL: &str = "team-agents@team-agents.invalid";

/// The findings a reviewer has written to `file`; none before the file exists.
///
/// # Errors
/// The cause, when the file exists and cannot be read.
async fn read_findings(file: &std::path::Path) -> Result<String, String> {
    match tokio::fs::read_to_string(file).await {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error_chain(&error)),
    }
}

/// Which checkpoint of an earlier holder a job taken over starts from.
#[derive(Debug, Clone, Copy)]
pub(super) enum TakenOver<'a> {
    /// A worker's request: the checkpoint of the same task, as its worktree is kept for the next worker.
    Request {
        /// The worker's task.
        task: &'a TaskId,
    },
    /// A review: the checkpoint of another reviewer of the same pull request and pass.
    Review {
        /// The task of the reviewer that takes over.
        task: &'a TaskId,
        /// The pull request under review.
        pull_request: PrNumber,
        /// The review pass.
        pass: u32,
    },
    /// An investigation: the checkpoint of another investigator of the same question.
    Investigation {
        /// The task of the investigator that takes over.
        task: &'a TaskId,
        /// What is meant by the investigation, as its checkpoints tell it.
        meant: &'a str,
    },
}

/// The newest checkpoint of an earlier holder in `rounds` that `taken_over` names.
fn earlier_checkpoint(
    reader: &Reader,
    rounds: &[RoundId],
    taken_over: &TakenOver<'_>,
) -> Result<Option<LastCheckpoint>, agent_runtime::history::ReadError> {
    let mut newest = None;
    for round in rounds {
        for kept in reader.job_checkpoints_of_round(round)? {
            let belongs = match taken_over {
                TakenOver::Request { task } => kept.job == CheckpointJob::WorkerRequest && kept.task == **task,
                TakenOver::Review {
                    task,
                    pull_request,
                    pass,
                } => {
                    kept.job == CheckpointJob::Review
                        && kept.task != **task
                        && kept.pull_request == Some(*pull_request)
                        && kept.review_pass == Some(*pass)
                }
                TakenOver::Investigation { task, meant } => {
                    kept.job == CheckpointJob::Investigation
                        && kept.task != **task
                        && kept.content.as_ref().is_some_and(|content| content.meant == *meant)
                }
            };
            // A change of holder says nothing of the work: the checkpoint before it does.
            if belongs && kept.cause != CheckpointCause::HolderChanged {
                newest = Some(kept);
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
    let task = match named {
        TakenOver::Request { task } | TakenOver::Review { task, .. } | TakenOver::Investigation { task, .. } => {
            task.clone()
        }
    };
    let owned = OwnedTakenOver::of(&named);
    let read = tokio::task::spawn_blocking(move || {
        let reader = Reader::open(&path)?;
        earlier_checkpoint(&reader, &lookup, &owned.as_taken_over(&task))
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

/// [`TakenOver`] without borrows, to cross into a blocking task.
enum OwnedTakenOver {
    /// See [`TakenOver::Request`].
    Request,
    /// See [`TakenOver::Review`].
    Review(PrNumber, u32),
    /// See [`TakenOver::Investigation`].
    Investigation(String),
}

impl OwnedTakenOver {
    /// The owned form of `taken_over`.
    fn of(taken_over: &TakenOver<'_>) -> Self {
        match taken_over {
            TakenOver::Request { .. } => Self::Request,
            TakenOver::Review { pull_request, pass, .. } => Self::Review(*pull_request, *pass),
            TakenOver::Investigation { meant, .. } => Self::Investigation((*meant).to_owned()),
        }
    }

    /// The borrowed form, for the holder `task`.
    fn as_taken_over<'a>(&'a self, task: &'a TaskId) -> TakenOver<'a> {
        match self {
            Self::Request => TakenOver::Request { task },
            Self::Review(pull_request, pass) => TakenOver::Review {
                task,
                pull_request: *pull_request,
                pass: *pass,
            },
            Self::Investigation(meant) => TakenOver::Investigation { task, meant },
        }
    }
}
