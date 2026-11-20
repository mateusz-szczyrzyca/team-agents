//! The briefing of a task the ladder started anew: what its runs left, gathered from facts just before the run of its
//! new session begins — the snapshot of the worktree first, then the commits of its branch, the files left
//! uncommitted, its plan of steps and the messages of its team that no run of it read.

use std::str::FromStr;

use agent_core::config::{AbsolutePath, InvalidValue};
use agent_core::decide::ladder::Trouble;
use agent_core::history::{CommitRecord, Record};
use agent_core::ids::{AgentName, BranchName, InvalidName, RoundId, TaskId};
use agent_core::log::LogEvent;
use agent_core::plan::{PLAN_FILE, StepState, parse_plan};
use agent_core::prompt::HarnessPrompt;
use agent_core::state::{InvalidTimestamp, Timestamp};
use agent_core::task_list::MAX_TASK_LIST_BYTES;
use agent_runtime::agentbus::{Reader, UnreadMessage, UnreadMessagesError};
use agent_runtime::git::{CommitIdentity, Git, GitError, Revision};
use agent_runtime::harness_prompts::HarnessPrompts;
use agent_runtime::process::error_chain;
use agent_runtime::run::compose::{
    Briefing, BriefingCommit, BriefingMessage, BriefingStep, ComposeError, briefing_variables,
};
use agent_runtime::snapshot::{SnapshotError, take_snapshot};
use agent_runtime::text_file::read_text_file;

use super::Owner;
use super::start::{StartProblem, work_branch};

/// One megabyte, in bytes: the unit of `[snapshot] warn_mb`.
const MEGABYTE: u64 = 1024 * 1024;

/// What the briefing of a task says, gathered before the run of its new session begins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BriefingFacts {
    /// Why the task was started anew.
    reason: Trouble,
    /// The branch the task's branch was forked from.
    base: BranchName,
    /// The task's branch.
    branch: BranchName,
    /// The commits of the branch since its base, oldest first.
    commits: Vec<CommitRecord>,
    /// The files of the worktree that differ from its head.
    uncommitted: Vec<String>,
    /// The steps of the task's plan as id, title and state; `None` when the worktree has no plan that reads.
    plan: Option<Vec<(String, String, &'static str)>>,
    /// The messages of its team that no run of the task read.
    messages: Vec<UnreadMessage>,
    /// The reference of the snapshot taken of the worktree.
    snapshot: String,
}

/// Why the briefing of a task cannot be gathered.
#[derive(Debug, thiserror::Error)]
pub(super) enum BriefingProblem {
    /// The task is not in a team of the state.
    #[error("task {task} is not in the state")]
    NoTask {
        /// The task.
        task: TaskId,
    },
    /// The worktree's path is not an absolute path.
    #[error("the worktree of task {task} has no usable path")]
    Path {
        /// The task.
        task: TaskId,
        /// What is wrong with the path.
        source: InvalidValue,
    },
    /// The worktree's path is not text.
    #[error("the worktree of task {task} has a path that is not text")]
    PathText {
        /// The task.
        task: TaskId,
    },
    /// What the state names cannot be used.
    #[error("the branch or the agent of the task cannot be named")]
    Start(#[from] StartProblem),
    /// The agent's name is not an agent name.
    #[error("the agent of the task cannot be named")]
    Name(#[from] InvalidName),
    /// The snapshot of the worktree cannot be taken.
    #[error("cannot take the snapshot of the worktree")]
    Snapshot(#[from] SnapshotError),
    /// Git cannot tell the commits or the files of the worktree.
    #[error("cannot read the branch or the worktree of the task")]
    Git(#[from] GitError),
    /// The unread messages cannot be read.
    #[error("cannot read the unread messages of the task")]
    Messages(#[from] UnreadMessagesError),
    /// The clock gives no valid time for the snapshot.
    #[error("cannot read the time of the snapshot")]
    Clock(#[from] InvalidTimestamp),
}

impl Owner {
    /// The facts of the briefing of `task` when the run it starts now is the first of a new session the ladder started
    /// it in, with the snapshot of its worktree taken and recorded first; `None` for any other run.
    ///
    /// # Errors
    /// [`BriefingProblem`] when a fact cannot be read.
    pub(super) async fn briefing_facts(&mut self, task: &TaskId) -> Result<Option<BriefingFacts>, BriefingProblem> {
        let no_task = || BriefingProblem::NoTask { task: task.clone() };
        let found = self.parts.state.find_task(task).ok_or_else(no_task)?;
        let Some(renewals) = found.member.renewals().filter(|_| found.member.awaits_new_session()) else {
            return Ok(None);
        };
        let (team, member) = (found.team, found.member);
        let reason = renewals.reason();
        let config = &self.parts.settings.config;
        let base = config
            .repo(team.repo())
            .map(|repo| repo.base_branch.clone())
            .ok_or_else(|| StartProblem::RepoGone {
                repo: team.repo().to_string(),
            })?;
        let branch = work_branch(team, member)?;
        let round = member
            .runs()
            .iter()
            .rev()
            .flat_map(|run| run.rounds().iter().rev())
            .find(|round| team.round(round).is_some_and(agent_core::state::Round::is_open))
            .cloned();
        let Some(round) = round else {
            // A snapshot is kept under a round: a task that works for none (a state of an older version) starts its new
            // session with its whole prompt, unbriefed, rather than not at all.
            tracing::warn!(
                event = LogEvent::HistoryRecordIncomplete.as_str(),
                task = %task,
                "a task started anew works for no open round: its new session is not briefed"
            );
            return Ok(None);
        };
        let agent = AgentName::from_str(member.agent().name())?;
        let path = self.parts.store.home().worktree_dir(team.repo(), task);
        let text = path
            .to_str()
            .ok_or_else(|| BriefingProblem::PathText { task: task.clone() })?;
        let worktree = AbsolutePath::from_str(text).map_err(|source| BriefingProblem::Path {
            task: task.clone(),
            source,
        })?;
        let reader = (agent.clone(), team.id().clone());
        let git = Git::new(worktree.clone(), CommitIdentity::for_agent(config.git(), &agent));
        let snapshot = self.snapshot(&worktree, &round, task).await?;
        let head = git.rev_parse(&Revision::Head).await?;
        let forked = git.merge_base(&Revision::Head, &Revision::Branch(base.clone())).await?;
        let commits = git.range_log(&forked, &head).await?;
        let uncommitted = git
            .uncommitted_changes()
            .await?
            .into_iter()
            .map(|change| change.path.to_string())
            .collect();
        let plan = read_steps(&worktree, task).await;
        let messages = self
            .parts
            .bus
            .unread_messages(Reader {
                agent: &reader.0,
                team: &reader.1,
                task,
            })
            .await?
            .shown;
        Ok(Some(BriefingFacts {
            reason,
            base,
            branch,
            commits,
            uncommitted,
            plan,
            messages,
            snapshot,
        }))
    }

    /// Takes the snapshot of `worktree` for `task` of `round`, records it in the history, and returns its reference.
    async fn snapshot(
        &mut self,
        worktree: &AbsolutePath,
        round: &RoundId,
        task: &TaskId,
    ) -> Result<String, BriefingProblem> {
        let now = Timestamp::now(&self.parts.clock)?;
        let warn_bytes = self.parts.settings.config.snapshot().warn_mb.saturating_mul(MEGABYTE);
        let record = take_snapshot(worktree, round, task, now, warn_bytes).await?;
        let reference = format!("refs/agents/snapshots/{round}/{task}/{}", record.number);
        self.history.record(Record::Snapshot(Box::new(record)));
        Ok(reference)
    }
}

impl BriefingFacts {
    /// The briefing of `task`, whose own prompt is `prompt`, rendered through `texts`.
    ///
    /// # Errors
    /// [`ComposeError`] when the text of the briefing cannot be filled.
    pub(super) fn render(&self, texts: &HarnessPrompts, task: &TaskId, prompt: &str) -> Result<String, ComposeError> {
        let commits: Vec<String> = self.commits.iter().map(|commit| commit.id.to_string()).collect();
        let listed: Vec<BriefingCommit<'_>> = self
            .commits
            .iter()
            .zip(&commits)
            .map(|(commit, id)| BriefingCommit {
                id: id.get(..8).unwrap_or(id),
                subject: &commit.subject,
            })
            .collect();
        let uncommitted: Vec<&str> = self.uncommitted.iter().map(String::as_str).collect();
        let steps: Option<Vec<BriefingStep<'_>>> = self.plan.as_ref().map(|steps| {
            steps
                .iter()
                .map(|(id, title, state)| BriefingStep { id, title, state })
                .collect()
        });
        let ids: Vec<String> = self.messages.iter().map(|message| message.id.to_string()).collect();
        let messages: Vec<BriefingMessage<'_>> = self
            .messages
            .iter()
            .zip(&ids)
            .map(|(message, id)| BriefingMessage {
                id,
                from: &message.from,
                text: &message.text,
            })
            .collect();
        let facts = Briefing {
            reason: reason_text(self.reason),
            task: task.as_str(),
            prompt,
            base: self.base.as_str(),
            branch: self.branch.as_str(),
            commits: &listed,
            uncommitted: &uncommitted,
            plan: steps.as_deref(),
            messages: &messages,
            left_out: None,
            snapshot: &self.snapshot,
        };
        let values = briefing_variables(&facts)?;
        Ok(texts.render(HarnessPrompt::Briefing, &values)?)
    }
}

/// Why the session before went bad, in the words of the briefing.
const fn reason_text(reason: Trouble) -> &'static str {
    match reason {
        Trouble::NoStatus => "its last runs ended without a `STATUS:` line",
        Trouble::Hang => "its last runs showed no activity until the watchdog stopped them",
        Trouble::TurnLimit => "its last runs used up their turn budget",
    }
}

/// The steps of the plan in the worktree as id, title and state; `None` when there is no plan or it does not read. A
/// plan that does not read is logged and left out: the new session reads the file itself.
async fn read_steps(worktree: &AbsolutePath, task: &TaskId) -> Option<Vec<(String, String, &'static str)>> {
    let read = read_text_file(&worktree.as_path().join(PLAN_FILE), MAX_TASK_LIST_BYTES).await;
    let parsed = match read {
        Ok(None) => return None,
        Ok(Some(file)) => parse_plan(&file.text).map_err(|error| error_chain(&error)),
        Err(error) => Err(error_chain(&error)),
    };
    match parsed {
        Ok(plan) => Some(
            plan.steps()
                .map(|step| (step.id().to_string(), step.title().to_owned(), state_text(step.state())))
                .collect(),
        ),
        Err(error) => {
            tracing::warn!(
                event = LogEvent::HistoryRecordIncomplete.as_str(),
                task = %task,
                error = %error,
                "the plan of a task started anew does not read: its briefing has no steps"
            );
            None
        }
    }
}

/// The state of a step, in the words of the briefing.
const fn state_text(state: StepState) -> &'static str {
    match state {
        StepState::Todo => "to do",
        StepState::InHand => "in hand",
        StepState::Done => "done (declared by the agent)",
        StepState::GivenUp => "given up",
    }
}
