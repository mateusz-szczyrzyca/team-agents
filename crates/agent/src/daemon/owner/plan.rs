//! The owner's side of the plan of a task (`.agent/plan.md`, see `docs/plan.md`): what the daemon does with it at the
//! end of a run.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use agent_core::config::{AbsolutePath, InvalidValue, RepoName};
use agent_core::decide::plan::{DoneEnds, PlanFacts, PlanRead, PlanVerdict, decide_plan_with_mismatch, done_ends_with};
use agent_core::ids::RunId;
use agent_core::ids::{AgentName, BranchName, InvalidName, ItemId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::plan::judge::{CheckRun, JudgementState, check_run, steps_to_judge_with};
use agent_core::plan::{PLAN_FILE, Plan, PlanError, Step, StepState, parse_plan};
use agent_core::prompt::{HarnessPrompt, Variables, VariablesError};
use agent_core::protocol::PlanStepSummary;
use agent_core::questions::{Entry, QuestionsFile, Verdict, quote};
use agent_core::repo_path::{InvalidRepoPath, RepoPath};
use agent_core::state::{ItemProgress, PlanFault, PlanKeeping, PlanReason, PlanStage};
use agent_core::state::{RunEnd, RunState, Team, Timestamp};
use agent_core::status::{RunOutcome, TaskStatus};
use agent_core::task_list::MAX_TASK_LIST_BYTES;
use agent_runtime::clock::SharedClock;
use agent_runtime::git::{CommitIdentity, Git, GitError, Revision, Untracked};
use agent_runtime::harness_prompts::{HarnessPrompts, RenderTextError};
use agent_runtime::history::{
    History, JudgementRecord, PlanRead as HistoryPlanRead, PlanRecord, ReadError, Reader, log_decision,
};
use agent_runtime::plan::check::{
    CheckContext, CheckError, JudgementResult, execute_command_check, execute_step_check, recover_checks,
};
use agent_runtime::process::error_chain;
use agent_runtime::run::compose::ComposeError;
use agent_runtime::text_file::{ReplaceError, TextFileError, read_text_file, replace_text_file};

use super::start::next_number;
use super::{FinishedRun, Located, Owner, OwnerError};
use crate::daemon::settle::Settled;

/// Why the plan of a task cannot be taken out of its branch.
#[derive(Debug, thiserror::Error)]
enum OutOfDeliveryError {
    /// The task's team is not in the state.
    #[error("the team {team} is not in the state")]
    TeamGone {
        /// The team.
        team: TeamId,
    },
    /// The worktree's path cannot be given to git.
    #[error("the worktree {path:?} cannot be named to git")]
    Worktree {
        /// The worktree.
        path: PathBuf,
        /// Why it is not an absolute UTF-8 path; `None` when it is not UTF-8.
        #[source]
        source: Option<InvalidValue>,
    },
    /// The agent's name in the state is not a name of an agent.
    #[error("the agent {name:?} cannot commit")]
    Agent {
        /// The name.
        name: String,
        /// What is wrong with it.
        source: InvalidName,
    },
    /// The plan's path is not a path of the repository.
    #[error("the plan's path cannot be given to git")]
    PlanPath(#[source] InvalidRepoPath),
    /// Whether the plan's file is in the worktree cannot be found out.
    #[error("cannot look for the plan {path:?}")]
    Look {
        /// The plan's file.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
    /// The task's repository is not in the configuration.
    #[error("the repository {repo} is not in the configuration")]
    RepoGone {
        /// The repository.
        repo: RepoName,
    },
    /// Git failed.
    #[error("git cannot take the plan out of the branch")]
    Git(#[source] GitError),
}

/// What became of a plan at the end of a run, as far as its branch goes.
#[derive(Debug)]
enum Kept {
    /// What git did with it: nothing, or out of the index, or out of the branch.
    Untracked(Untracked),
    /// The repository's base branch tracks the plan: it is left as it is.
    OnTheBase,
}

/// The subject of the commit that takes a plan committed by mistake out of a task's branch.
const OUT_OF_DELIVERY: &str = "Keep the plan out of the delivery";

impl Owner {
    /// Takes the plan of `task` out of its branch when its agent committed it, keeping the file in the worktree: the plan
    /// is the task's working file, never part of what it delivers. Only the roles whose branch is delivered (solo,
    /// coordinator, member) are looked at.
    ///
    /// What cannot be done is logged and does not stop the run's end: a plan left on the branch is the agent's mistake,
    /// not a reason to lose the run.
    pub(super) async fn keep_plan_out_of_delivery(&self, task: &TaskId, located: &Located) {
        match located.role {
            Role::Solo | Role::Coordinator | Role::Member => {}
            Role::Worker | Role::Reviewer => return,
        }
        match self.untrack_plan(task, located).await {
            Ok(Kept::Untracked(untracked)) => {
                log_decision("plan_out_of_delivery", &task.as_str(), &format!("{untracked:?}"));
            }
            Ok(Kept::OnTheBase) => log_decision(
                "plan_out_of_delivery",
                &task.as_str(),
                "left on the branch: the base branch tracks the plan",
            ),
            Err(error) => tracing::warn!(
                event = LogEvent::GitCallFailed.as_str(),
                task = %task,
                error = %error_chain(&error),
                "a plan committed by mistake cannot be taken out of the branch"
            ),
        }
    }

    /// Takes the plan of `task` out of git in its worktree. A worktree without the file is not asked: a plan the agent
    /// committed is in the worktree, and one it deleted since is not what its branch delivers as it is.
    async fn untrack_plan(&self, task: &TaskId, located: &Located) -> Result<Kept, OutOfDeliveryError> {
        let worktree = self.worktree_of(task, located)?;
        let file = worktree.as_path().join(PLAN_FILE);
        match tokio::fs::try_exists(&file).await {
            Ok(true) => {}
            Ok(false) => return Ok(Kept::Untracked(Untracked::NotTracked)),
            Err(source) => return Err(OutOfDeliveryError::Look { path: file, source }),
        }
        let plan = RepoPath::from_str(PLAN_FILE).map_err(OutOfDeliveryError::PlanPath)?;
        let agent = AgentName::from_str(&located.agent).map_err(|source| OutOfDeliveryError::Agent {
            name: located.agent.clone(),
            source,
        })?;
        let git = Git::new(
            worktree,
            CommitIdentity::for_agent(self.parts.settings.config.git(), &agent),
        );
        let base = self.base_of(located)?;
        if git
            .tracks(&Revision::Branch(base), &plan)
            .await
            .map_err(OutOfDeliveryError::Git)?
        {
            // The repository keeps a plan of its own on its base: a change of it is the repository's, not the harness's.
            return Ok(Kept::OnTheBase);
        }
        git.untrack(&plan, OUT_OF_DELIVERY)
            .await
            .map(Kept::Untracked)
            .map_err(OutOfDeliveryError::Git)
    }

    /// The base branch of the repository of the team of `located`.
    fn base_of(&self, located: &Located) -> Result<BranchName, OutOfDeliveryError> {
        let team = self
            .parts
            .state
            .team(&located.team)
            .ok_or_else(|| OutOfDeliveryError::TeamGone {
                team: located.team.clone(),
            })?;
        self.parts
            .settings
            .config
            .repo(team.repo())
            .map(|repo| repo.base_branch.clone())
            .ok_or_else(|| OutOfDeliveryError::RepoGone {
                repo: team.repo().clone(),
            })
    }

    /// The worktree of `task`.
    fn worktree_of(&self, task: &TaskId, located: &Located) -> Result<AbsolutePath, OutOfDeliveryError> {
        let team = self
            .parts
            .state
            .team(&located.team)
            .ok_or_else(|| OutOfDeliveryError::TeamGone {
                team: located.team.clone(),
            })?;
        let path = self.parts.store.home().worktree_dir(team.repo(), task);
        match path.to_str() {
            Some(text) => AbsolutePath::from_str(text).map_err(|source| OutOfDeliveryError::Worktree {
                path: path.clone(),
                source: Some(source),
            }),
            None => Err(OutOfDeliveryError::Worktree { path, source: None }),
        }
    }
}

/// Whether the human's question about a task's plan was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlanQuestion {
    /// The task's next questions file holds it, and the task's plan waits for its answer: the run's end is settled as
    /// a run that asked questions.
    Written,
    /// It could not be made or written (logged): the run's end is settled as the done it reported.
    NotWritten,
}

/// Whether the end of a run was held back for its plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlanHeld {
    /// The run ends as it reported.
    No,
    /// The run reported `DONE` and was sent back, or the human is asked: nobody is told it is done.
    Yes,
}

/// What the decision on the plan of a run reads besides the run's facts: the run's end, the plan read, the versions kept
/// before, and the first failed check of the run.
struct AtEnd<'a> {
    /// How the run ended.
    end: &'a RunEnd,
    /// What the end found of the plan.
    found: &'a Found,
    /// The versions of the plan kept from runs before this one.
    earlier: &'a [Plan],
    /// The first step of the run whose check failed, when the run judged one.
    mismatch: Option<PlanReason>,
}

/// What the end of a run found of the plan: what the decision reads, and the text as the task wrote it.
pub(super) struct Found {
    /// What the decision reads.
    read: PlanRead,
    /// The text of a plan that parses, as written; `None` otherwise.
    text: Option<String>,
}

/// Why existing step judgements in `history.db` could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PlanJudgementsError {
    /// Reading `history.db` failed.
    #[error("cannot read existing step judgements of task {task}")]
    Read {
        /// The task whose judgements were being read.
        task: TaskId,
        /// The underlying history reader failure.
        #[source]
        source: Box<ReadError>,
    },
    /// Joining the blocking history reader task failed.
    #[error("cannot join step judgements reader task for {task}")]
    Join {
        /// The task whose judgements were being read.
        task: TaskId,
        /// The underlying blocking task join failure.
        #[source]
        source: tokio::task::JoinError,
    },
}

/// Why a leftover or stopping command check could not be cleaned up.
#[derive(Debug, thiserror::Error)]
pub(crate) enum PlanCheckCleanupError {
    /// The `tasks` directory cannot be read on startup.
    #[error("cannot read the tasks directory {}", path.display())]
    TasksDirRead {
        /// The tasks directory.
        path: PathBuf,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// An entry of the `tasks` directory cannot be read on startup.
    #[error("cannot read an entry of the tasks directory {}", path.display())]
    TasksDirEntry {
        /// The tasks directory.
        path: PathBuf,
        /// The OS error.
        #[source]
        source: std::io::Error,
    },
    /// An entry of the `tasks` directory is not a valid task directory name.
    #[error("invalid task directory entry {}", path.display())]
    InvalidTaskEntry {
        /// The invalid entry path.
        path: PathBuf,
    },
    /// A running check record could not be recovered or its process group could not be ended.
    #[error("cannot end the running command check of task {task}")]
    Recover {
        /// The task.
        task: TaskId,
        /// The underlying recovery error.
        #[source]
        source: CheckError,
    },
    /// Joining the blocking recovery task failed.
    #[error("cannot join command check recovery task for {task}")]
    Join {
        /// The task.
        task: TaskId,
        /// The underlying blocking task join failure.
        #[source]
        source: tokio::task::JoinError,
    },
    /// Joining the blocking startup leftover recovery task failed.
    #[error("cannot join startup leftover command check recovery task for {}", path.display())]
    TasksDirJoin {
        /// The tasks directory.
        path: PathBuf,
        /// The underlying blocking task join failure.
        #[source]
        source: tokio::task::JoinError,
    },
}

/// Why the environment of a command check cannot be prepared.
#[derive(Debug, thiserror::Error)]
enum CheckEnvError {
    /// The agent's name in the state is not a valid [`AgentName`].
    #[error("invalid agent name {name:?}")]
    AgentName {
        /// The invalid name.
        name: String,
        /// What is wrong with it.
        #[source]
        source: InvalidName,
    },
    /// The directory of the daemon's `agent` binary cannot be resolved.
    #[error("cannot resolve the agent binary directory")]
    BinaryDir(#[from] crate::daemon::ExecutableError),
    /// The command check's environment variables cannot be built.
    #[error("cannot build the check environment")]
    Environment(#[from] agent_runtime::run::RunError),
}

/// The judgement of the steps of a plan at the end of a run: either all known now, or waiting for the command checks.
enum Judged {
    /// Every step is judged: the latest judgement of each step.
    Known(HashMap<ItemId, JudgementRecord>),
    /// The steps that run no command are judged (`inline_records`); the command checks of the others (`commands`, by
    /// step with the command) are ready to run in the background (`job`) and the run waits for them.
    Waiting {
        /// The judgements that were made without a command.
        inline_records: Vec<JudgementRecord>,
        /// The run whose end the judgements belong to.
        run: Option<RunId>,
        /// The steps whose commands run, in the order of the plan, with their commands.
        commands: Vec<(ItemId, String)>,
        /// The background job to spawn once `RunningPlanCheck` is durably saved.
        job: Box<CheckJob>,
    },
}

/// The decision on the plan of a run that waits for command checks: what the end of the run had to decide with.
pub(super) struct PlanWait {
    /// What the end of the run comes to without its plan.
    pub(super) settled: Settled,
    /// What the end found of the plan.
    pub(super) found: Found,
    /// The versions of the plan the history kept from runs before this one.
    pub(super) earlier: Vec<Plan>,
    /// The run whose end is judged.
    pub(super) run: Option<RunId>,
    /// The judgements made without a command, kept in memory so status snapshots keep them even without history.
    pub(super) inline_records: Vec<JudgementRecord>,
    /// The steps whose commands run in the background, in the order of the plan, with their commands.
    pub(super) commands: Vec<(ItemId, String)>,
}

/// What the plan of a finished run leaves to do: the end of the run is settled now, or it waits for command checks.
pub(super) enum PlanStep {
    /// The run's end is settled with this status and plan outcome.
    Done(Settled, PlanHeld),
    /// The run's end waits for its command checks, which are spawned after persisting `RunningPlanCheck`.
    Waiting(Box<PlanWait>, Box<CheckJob>),
}

/// A finished run whose end waits for its command checks, held by the owner until the checks end.
pub(super) struct Parked {
    /// The task's facts, as the run ended.
    pub(super) located: Located,
    /// The run that ended.
    pub(super) run: FinishedRun,
    /// What the end of the run waits to decide with.
    pub(super) wait: PlanWait,
    /// What aborts the background command check batch if the human stops or pauses the task.
    pub(super) abort: tokio::task::AbortHandle,
}

/// What a command check batch runs: the commands of one run, with what each needs, owned so that it runs in the
/// background. The batch runs them one after another and persists each judgement before starting the next command.
pub(super) struct CheckJob {
    /// The task whose steps are judged.
    task: TaskId,
    /// The task's team.
    team: TeamId,
    /// The task's worktree: where the commands run.
    worktree: PathBuf,
    /// The task's directory in the store: where the output and the record of the checks are kept.
    task_dir: PathBuf,
    /// The run whose end the checks judge.
    run: Option<RunId>,
    /// The time limit of each command, in seconds.
    check_seconds: u64,
    /// The prefixes of commands that are refused as checks.
    refuse: Vec<String>,
    /// The environment of each command.
    envs: Vec<(String, String)>,
    /// The steps to run, in the order of the plan, with their commands.
    commands: Vec<(Step, String)>,
    /// The history handle used to persist each command check's judgement as soon as it finishes.
    history: History,
    /// The clock used to stamp each command check's judgement.
    clock: SharedClock,
}

/// The results of a command check batch, one per command, in the order of the commands.
pub(super) struct CheckBatch {
    /// The judgement record of each command, or the clock error that prevented stamping a judgement.
    records: Result<Vec<JudgementRecord>, agent_core::state::InvalidTimestamp>,
}

/// Runs the commands of `job` one after another, in the background, persisting each command's judgement before
/// starting the next command, and returns their records.
async fn run_command_checks(job: CheckJob) -> CheckBatch {
    let mut records = Vec::with_capacity(job.commands.len());
    for (step, command) in &job.commands {
        let context = CheckContext {
            worktree: &job.worktree,
            task_dir: &job.task_dir,
            run: job.run.as_ref(),
            check_seconds: job.check_seconds,
            check_refuse: &job.refuse,
            envs: &job.envs,
        };
        let judged = match execute_command_check(&context, step, command).await {
            Ok(result) => result,
            Err(error) => mismatch(
                Some(command.clone()),
                format!("cannot run check: {}", error_chain(&error)),
            ),
        };
        let at = match Timestamp::now(&job.clock) {
            Ok(at) => at,
            Err(error) => {
                return CheckBatch { records: Err(error) };
            }
        };
        let record = judged.into_record(job.task.clone(), job.team.clone(), job.run.clone(), at, step.id());
        log_decision(
            "plan_check",
            &format!("task={}, step={}", job.task, record.step),
            &format!(
                "state={}, reason={:?}, output={:?}",
                record.state.as_str(),
                record.reason,
                record.output_path
            ),
        );
        if job.history.is_writing()
            && let Err(error) = job.history.persist_judgements(vec![record.clone()]).await
        {
            tracing::warn!(
                event = LogEvent::HistoryRecordIncomplete.as_str(),
                task = %job.task,
                error = %error_chain(&error),
                "plan judgements remain only in memory because history could not keep them"
            );
        }
        records.push(record);
    }
    CheckBatch { records: Ok(records) }
}

/// The first step of `plan`, in its order, that is done and whose judgement in `judgements` is a mismatch of run `run`, as
/// the mismatch a run is sent back for: its check, the reason and the last lines of its output. A check whose output
/// cannot be read says so in its tail.
async fn first_mismatch(
    task_dir: &Path,
    plan: &Plan,
    judgements: &HashMap<ItemId, JudgementRecord>,
    run: Option<&RunId>,
) -> Option<PlanReason> {
    for step in plan.steps() {
        if step.state() != StepState::Done {
            continue;
        }
        let Some(record) = judgements
            .get(&step.id())
            .filter(|record| record.state == JudgementState::Mismatch && record.run.as_ref() == run)
        else {
            continue;
        };
        let tail = match &record.output_path {
            None => String::new(),
            Some(output) => match agent_runtime::plan::check::read_check_tail(&task_dir.join(output)).await {
                Ok(tail) => tail,
                Err(error) => format!("cannot read the check output {output}: {}", error_chain(&error)),
            },
        };
        return Some(PlanReason::Mismatch {
            step: step.id(),
            title: step.title().to_owned(),
            check: record.check.clone(),
            reason: record.reason.clone(),
            tail,
        });
    }
    None
}

/// The commands of the judgements that the gate refused, with their steps, in the order of `results`.
fn refused_gates(results: &[(ItemId, JudgementResult)]) -> Vec<(ItemId, String)> {
    results
        .iter()
        .filter(|(_, judged)| judged.gate_refused)
        .filter_map(|(step, judged)| judged.check_line.clone().map(|command| (*step, command)))
        .collect()
}

/// A judgement of a step that failed, with the check it failed on, if any, and why.
fn mismatch(check: Option<String>, reason: String) -> JudgementResult {
    JudgementResult {
        state: JudgementState::Mismatch,
        check_line: check,
        reason: Some(reason),
        duration_ms: 0,
        output_path: None,
        tail: String::new(),
        gate_refused: false,
    }
}

/// The steps of `plan` that still need a judgement for `run`.
fn steps_for_run<'a>(
    plan: &'a Plan,
    run: Option<&RunId>,
    earlier: (Option<&'a Plan>, &'a HashMap<RunId, Plan>),
    previous: &HashMap<ItemId, JudgementRecord>,
) -> Vec<&'a Step> {
    let (earlier_plan, earlier_by_run) = earlier;
    steps_to_judge_with(
        plan,
        |id| {
            let latest_step = earlier_plan.and_then(|plan| plan.step(id));
            if latest_step.is_none_or(|step| step.state() != StepState::Done) {
                return latest_step;
            }
            if let Some(judgement) = previous.get(id)
                && let Some(run_id) = &judgement.run
                && let Some(plan_at_run) = earlier_by_run.get(run_id)
            {
                return plan_at_run.step(id);
            }
            earlier_plan.and_then(|p| p.step(id))
        },
        |id| previous.get(id).map(|j| j.state),
    )
    .into_iter()
    .filter(|step| {
        !run.is_some_and(|current_run| {
            previous
                .get(&step.id())
                .is_some_and(|judgement| judgement.run.as_ref() == Some(current_run))
        })
    })
    .collect()
}

/// What the human's answer to the question about a task's plan says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlanAnswer {
    /// Go on as it is: the plan is released, and the agent's next `DONE` is taken as it is.
    GoOn,
    /// Send the run back once more: one more send-back is granted and counted.
    OnceMore,
}

impl Owner {
    /// Writes the question to the human about the plan of `task`, sent back as often as `[plan] bounces` allows and still
    /// against its plan for `reason`, as the task's next questions file, and records that its plan waits for the answer.
    ///
    /// # Errors
    /// [`OwnerError`] when the task is gone or has no questions number left.
    pub(super) async fn write_plan_question(
        &mut self,
        task: &TaskId,
        located: &Located,
        reason: &PlanReason,
    ) -> Result<PlanQuestion, OwnerError> {
        let (number, count) = {
            let found = self
                .parts
                .state
                .find_task(task)
                .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
            let number = next_number(found.member).ok_or_else(|| OwnerError::QuestionNumbers { task: task.clone() })?;
            (number, found.member.plan().map_or(0, PlanKeeping::sent_back))
        };
        let path = self.parts.store.home().questions_file(task, number);
        if let Err(error) = self.put_plan_question(task, located, reason, count, &path).await {
            // The human is not asked, and the run's end is the done it reported: losing the run would be worse.
            tracing::warn!(
                event = LogEvent::QuestionsFileInvalid.as_str(),
                task = %task,
                path = %path.display(),
                error = %error_chain(&error),
                "the question about the plan cannot be written: the run counts as done"
            );
            return Ok(PlanQuestion::NotWritten);
        }
        self.member_mut(task)?.1.plan_asked(number);
        Ok(PlanQuestion::Written)
    }

    /// Renders the question about the plan of `task` and writes it to `path`.
    async fn put_plan_question(
        &self,
        task: &TaskId,
        located: &Located,
        reason: &PlanReason,
        count: u32,
        path: &Path,
    ) -> Result<(), PlanQuestionError> {
        let told = correction(&self.parts.texts, reason)?;
        let values = Variables::new()
            .with("task", task.as_str())?
            .with("agent", located.agent.as_str())?
            .with("count", count.to_string())?
            .with("reason", quote(&told))?;
        let text = self.parts.texts.render(HarnessPrompt::NoticePlanQuestion, &values)?;
        if let Some(directory) = path.parent() {
            tokio::fs::create_dir_all(directory)
                .await
                .map_err(|source| PlanQuestionError::Directory {
                    path: directory.to_path_buf(),
                    source,
                })?;
        }
        replace_text_file(path, &text, QUESTIONS_MODE).await?;
        Ok(())
    }

    /// Applies the human's answer `file` to questions `number` of `task` to its plan, when those are the questions about
    /// its plan; other questions change nothing of it. The task goes on with the answers either way.
    ///
    /// # Errors
    /// [`OwnerError`] when the task is gone.
    pub(super) fn apply_plan_answer(
        &mut self,
        task: &TaskId,
        number: NonZeroU32,
        file: &QuestionsFile,
    ) -> Result<(), OwnerError> {
        let stage = self
            .parts
            .state
            .find_task(task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?
            .member
            .plan()
            .map(PlanKeeping::stage);
        if stage != Some(PlanStage::Asked { question: number }) {
            return Ok(());
        }
        let answer = plan_answer(file);
        let (_, mut member) = self.member_mut(task)?;
        match answer {
            PlanAnswer::GoOn => member.release_plan(),
            PlanAnswer::OnceMore => member.send_back_once_more(),
        }
        // The run before asked nothing: the next one is given the answers all the same.
        member.plan_answered(number);
        log_decision("plan_answer", &task.as_str(), &format!("{answer:?}"));
        Ok(())
    }
}

impl Owner {
    /// What the end `end` of a run of `task` does with its plan: what the worktree has is kept in the history, its
    /// command checks are started in the background, and a run that reported `DONE` against its plan is sent back or the
    /// human asked, by [`decide_plan_with_mismatch`]. `settled` is what the run's end comes to otherwise. The decision is
    /// [`PlanStep::Waiting`] while command checks run: [`Owner::check_batch_done`] makes it when they have ended.
    ///
    /// # Errors
    /// [`OwnerError`] when the task is gone or has no questions number left.
    pub(super) async fn plan_step_at_end(
        &mut self,
        task: &TaskId,
        located: &Located,
        end: &RunEnd,
        settled: Settled,
    ) -> Result<PlanStep, OwnerError> {
        let worktree = match self.worktree_of(task, located) {
            Ok(worktree) => worktree,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::GitCallFailed.as_str(),
                    task = %task,
                    error = %error_chain(&error),
                    "the plan of the task cannot be looked for"
                );
                return Ok(PlanStep::Done(settled, PlanHeld::No));
            }
        };
        let found = read_plan(&worktree.as_path().join(PLAN_FILE)).await;
        let run = self
            .parts
            .state
            .find_task(task)
            .and_then(|found| found.member.current_run().map(|run| run.id().clone()));
        let (earlier, earlier_by_run) = if matches!(found.read, PlanRead::Parsed(_)) {
            self.earlier_plans(task, run.as_ref()).await
        } else {
            (Vec::new(), HashMap::new())
        };
        let already_keeping_check = self
            .parts
            .state
            .find_task(task)
            .and_then(|found| found.member.plan_running_check())
            .is_some_and(|check| Some(&check.run) == run.as_ref());
        if !already_keeping_check {
            if let Some((_, mut member)) = self.parts.state.find_task_mut(task) {
                let delivered = member.take_plan_refused_gate();
                if !delivered.is_empty() {
                    tracing::debug!(
                        task = %task,
                        refused = delivered.len(),
                        "the run finished after being told of its refused gate checks"
                    );
                }
            }
            self.keep_plan(task, located, run.clone(), &found)?;
        }
        let mut failed = None;
        if let PlanRead::Parsed(_) = &found.read {
            let judged = self
                .judge_steps_at_end(
                    task,
                    located,
                    run.as_ref(),
                    &found,
                    &worktree,
                    (earlier.last(), &earlier_by_run),
                )
                .await?;
            match judged {
                Judged::Known(latest) => {
                    if let PlanRead::Parsed(plan) = &found.read {
                        self.refresh_task_plan(task, plan, &latest);
                        let task_dir = self.parts.store.home().task_dir(task);
                        failed = first_mismatch(&task_dir, plan, &latest, run.as_ref()).await;
                    }
                }
                Judged::Waiting {
                    inline_records,
                    run,
                    commands,
                    job,
                } => {
                    return Ok(PlanStep::Waiting(
                        Box::new(PlanWait {
                            settled,
                            found,
                            earlier,
                            run,
                            inline_records,
                            commands,
                        }),
                        job,
                    ));
                }
            }
        } else {
            self.invalidate_task_plan(task);
        }
        let (settled, held) = self
            .decide_at_end(
                task,
                located,
                AtEnd {
                    end,
                    found: &found,
                    earlier: &earlier,
                    mismatch: failed,
                },
                settled,
            )
            .await?;
        Ok(PlanStep::Done(settled, held))
    }

    /// What [`Owner::plan_step_at_end`] comes to for a caller that cannot wait for command checks: the status and the
    /// plan outcome, with the run held (`PlanHeld::Yes`) while its checks run.
    ///
    /// # Errors
    /// [`OwnerError`] as [`Owner::plan_step_at_end`].
    #[cfg(test)]
    pub(super) async fn plan_at_end(
        &mut self,
        task: &TaskId,
        located: &Located,
        end: &RunEnd,
        settled: Settled,
    ) -> Result<(Settled, PlanHeld), OwnerError> {
        match self.plan_step_at_end(task, located, end, settled).await? {
            PlanStep::Done(settled, held) => Ok((settled, held)),
            PlanStep::Waiting(wait, _job) => Ok((wait.settled, PlanHeld::Yes)),
        }
    }

    /// The decision on the plan of a run of `task` that ended with `end`, once its judgements are known: a run that
    /// reported `DONE` against a plan that does not say so is sent back, and after the limit the human is asked.
    ///
    /// # Errors
    /// [`OwnerError`] when the task is gone or has no questions number left.
    async fn decide_at_end(
        &mut self,
        task: &TaskId,
        located: &Located,
        at: AtEnd<'_>,
        settled: Settled,
    ) -> Result<(Settled, PlanHeld), OwnerError> {
        let AtEnd {
            end,
            found,
            earlier,
            mismatch,
        } = at;
        let ends = self.parts.state.team(&located.team).map_or(DoneEnds::Work, |team| {
            done_ends_with(located.role, team.items().iter().map(ItemProgress::phase), |agent| {
                member_works(team, agent)
            })
        });
        if ends == DoneEnds::Turn {
            // The coordinator's members still work: its plan has steps for them, and the turn is not judged by it.
            log_decision("plan", &task.as_str(), &format!("{ends:?}"));
            return Ok((settled, PlanHeld::No));
        }
        let verdict = {
            let member = self
                .parts
                .state
                .find_task(task)
                .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?
                .member;
            let mismatch = if member.plan().is_some_and(PlanKeeping::judging) {
                mismatch
            } else {
                None
            };
            decide_plan_with_mismatch(
                PlanFacts {
                    role: located.role,
                    end,
                    read: &found.read,
                    earlier,
                    keeping: member.plan(),
                    policy: self.parts.settings.config.plan(),
                },
                mismatch,
            )
        };
        log_decision("plan", &task.as_str(), &format!("{verdict:?}"));
        Ok(match verdict {
            PlanVerdict::Accept => (settled, PlanHeld::No),
            PlanVerdict::SendBack(reason) => {
                self.member_mut(task)?.1.send_back(reason);
                (Settled::Status(TaskStatus::Queued), PlanHeld::Yes)
            }
            PlanVerdict::AskHuman(reason) => match self.write_plan_question(task, located, &reason).await? {
                PlanQuestion::Written => (Settled::Questions, PlanHeld::Yes),
                PlanQuestion::NotWritten => (settled, PlanHeld::No),
            },
        })
    }

    /// The versions of the plan of `task` the history kept from runs other than `run`, oldest first, along with a map by
    /// run ID; none when the history cannot be read (logged): a step lost since then is then not found. A kept version
    /// that does not parse is left out and logged.
    async fn earlier_plans(&self, task: &TaskId, run: Option<&RunId>) -> (Vec<Plan>, HashMap<RunId, Plan>) {
        let path = self.parts.store.home().history_file();
        let task_id = task.clone();
        let task_runs: Vec<RunId> = self
            .parts
            .state
            .find_task(task)
            .map(|found| found.member.runs().iter().map(|r| r.id().clone()).collect())
            .unwrap_or_default();
        let looked =
            tokio::task::spawn_blocking(move || Reader::open(&path).and_then(|reader| reader.plan_versions(&task_id)))
                .await;
        let records = match looked {
            Ok(Ok(records)) => records,
            Ok(Err(error)) => {
                // A home without a history (or one disabled) has kept no plan: nothing can be found lost.
                tracing::debug!(task = %task, error = %error_chain(&error), "no plan kept before can be read");
                return (Vec::new(), HashMap::new());
            }
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::HistoryRecordIncomplete.as_str(),
                    task = %task,
                    %error,
                    "the plans kept before cannot be read: no step is found lost"
                );
                return (Vec::new(), HashMap::new());
            }
        };
        let parsed: Vec<_> = records.iter().map(|record| (record, retained_plan(record))).collect();
        let plans = parsed
            .iter()
            .filter(|(record, _)| record.run.as_ref() != run)
            .filter_map(|(_, plan)| plan.clone())
            .collect();
        let by_run = expand_plans_by_run(&task_runs, &parsed);
        (plans, by_run)
    }

    /// Judges the steps of `found`'s plan that are to be judged at the end of `run`: the steps that run no command are
    /// judged now and recorded; the command checks of the others start in the background, and the run waits for them.
    async fn judge_steps_at_end(
        &mut self,
        task: &TaskId,
        located: &Located,
        run: Option<&RunId>,
        found: &Found,
        worktree: &AbsolutePath,
        earlier: (Option<&Plan>, &HashMap<RunId, Plan>),
    ) -> Result<Judged, OwnerError> {
        let PlanRead::Parsed(plan) = &found.read else {
            return Ok(Judged::Known(HashMap::new()));
        };
        let previous = self.latest_judgements(task).await?;
        let to_judge = steps_for_run(plan, run, earlier, &previous);
        let config = self.parts.settings.config.plan();
        let check_seconds = config.check_seconds();
        let refuse = config.check_refuse().to_vec();
        let task_dir = self.parts.store.home().task_dir(task);
        let context = CheckContext {
            worktree: worktree.as_path(),
            task_dir: &task_dir,
            run,
            check_seconds: 0,
            check_refuse: &refuse,
            envs: &[],
        };
        let mut inline = Vec::new();
        let mut commands: Vec<(Step, String)> = Vec::new();
        for step in to_judge {
            match check_run(step.check(), check_seconds, &refuse) {
                CheckRun::Run(command) => commands.push((step.clone(), command.to_owned())),
                CheckRun::Inline | CheckRun::Refused | CheckRun::Disabled => {
                    let result = match execute_step_check(&context, step).await {
                        Ok(result) => result,
                        Err(error) => mismatch(
                            step.check().map(ToString::to_string),
                            format!("cannot run check: {}", error_chain(&error)),
                        ),
                    };
                    inline.push((step.id(), result));
                }
            }
        }
        self.note_refused_gates(task, &inline)?;
        if commands.is_empty() {
            return self.known_judgements(task, located, run, inline).await;
        }
        match self.check_environment(located) {
            Ok(envs) => {
                let inline_records = if inline.is_empty() {
                    Vec::new()
                } else {
                    let records = self.records_of(task, located, run, inline)?;
                    drop(self.record_judgements(task, records.clone()).await?);
                    records
                };
                let ids = commands
                    .iter()
                    .map(|(step, command)| (step.id(), command.clone()))
                    .collect();
                let job = Box::new(CheckJob {
                    task: task.clone(),
                    team: located.team.clone(),
                    worktree: worktree.as_path().to_path_buf(),
                    task_dir,
                    run: run.cloned(),
                    check_seconds,
                    refuse,
                    envs,
                    commands,
                    history: self.history.clone(),
                    clock: self.parts.clock.clone(),
                });
                Ok(Judged::Waiting {
                    inline_records,
                    run: run.cloned(),
                    commands: ids,
                    job,
                })
            }
            Err(error) => {
                let reason = error_chain(&error);
                for (step, command) in &commands {
                    inline.push((
                        step.id(),
                        mismatch(
                            Some(command.clone()),
                            format!("cannot prepare the environment of the check: {reason}"),
                        ),
                    ));
                }
                self.known_judgements(task, located, run, inline).await
            }
        }
    }

    /// The environment command checks of the task of `located` run with: what a run of its agent gets from the daemon.
    ///
    /// # Errors
    /// [`CheckEnvError`] when the agent's name, the daemon's executable, the `PATH` or the home cannot be resolved.
    fn check_environment(&self, located: &Located) -> Result<Vec<(String, String)>, CheckEnvError> {
        let agent: AgentName = located.agent.parse().map_err(|source| CheckEnvError::AgentName {
            name: located.agent.clone(),
            source,
        })?;
        let identity = CommitIdentity::for_agent(self.parts.settings.config.git(), &agent);
        let binary_dir = crate::daemon::resolve_agent_binary_dir()?;
        let mut envs = agent_runtime::run::check_environment(
            &self.parts.settings.secrets,
            &binary_dir,
            &self.parts.settings.config.binaries().agent_path,
            self.parts.store.home().root(),
            &identity,
        )?;
        let incremental = if self.parts.settings.config.build().incremental {
            "1"
        } else {
            "0"
        };
        envs.retain(|(name, _)| name != "CARGO_INCREMENTAL");
        envs.push(("CARGO_INCREMENTAL".to_owned(), incremental.to_owned()));
        Ok(envs)
    }

    /// Starts the command checks of a run in the background: the owner goes on while they run, and their batch is
    /// taken by [`Owner::check_batch_done`] when it ends.
    pub(super) fn start_command_checks(&mut self, task: &TaskId, job: CheckJob) -> tokio::task::AbortHandle {
        let abort = self.checks.spawn(run_command_checks(job));
        self.checking.insert(abort.id(), task.clone());
        abort
    }

    /// Ends any recorded command check process group of `task` on the blocking thread pool so `/proc` scanning never
    /// blocks the async owner loop.
    async fn recover_task_checks(&self, task: &TaskId) -> Result<Option<NonZeroU32>, PlanCheckCleanupError> {
        let task_dir = self.parts.store.home().task_dir(task);
        let task_id = task.clone();
        let joined = tokio::task::spawn_blocking(move || recover_checks(&task_dir))
            .await
            .map_err(|source| PlanCheckCleanupError::Join {
                task: task_id.clone(),
                source,
            })?;
        joined.map_err(|source| PlanCheckCleanupError::Recover { task: task_id, source })
    }

    /// Ends the parked command check of `task` when the human stops or pauses the task, and settles the run with the
    /// requested halt.
    ///
    /// # Errors
    /// [`OwnerError`] when the check process group cannot be ended or the state cannot be saved.
    pub(super) async fn halt_parked_check(&mut self, task: &TaskId) -> Result<(), OwnerError> {
        let Some(parked) = self.parked.remove(task) else {
            return Ok(());
        };
        self.checking.remove(&parked.abort.id());
        self.aborted_checks.insert(parked.abort.id());
        parked.abort.abort();
        self.recover_task_checks(task).await?;
        let now = Timestamp::now(&self.parts.clock)?;
        let mut events = Vec::new();
        self.end_run(
            task,
            &parked.located.team,
            now,
            RunEnd::Stopped,
            parked.run.final_message.as_deref(),
            &mut events,
        )?;
        if let Some((_, mut member)) = self.parts.state.find_task_mut(task) {
            member.set_plan_running_check(None);
        }
        let status = self
            .halts
            .remove(task)
            .map_or(TaskStatus::Stopped, crate::daemon::control::halted_status);
        self.move_to(task, &parked.located.team, status, &mut events)?;
        self.hangs.remove(task);
        self.wakes.remove(task);
        self.commit(task, &parked.located, now, events, None).await?;
        self.release_run_worktree_targets(task);
        self.after_run(
            task,
            &parked.located,
            &RunEnd::Stopped,
            parked.run.final_message.as_deref(),
        )
    }

    /// Takes the command checks of a run that ended: their results are recorded, and the run is ended with its plan's
    /// decision, as if no check had waited.
    ///
    /// # Errors
    /// [`OwnerError`] when the batch belongs to no task that waits, or the run cannot be ended.
    pub(super) async fn check_batch_done(
        &mut self,
        joined: Result<(tokio::task::Id, CheckBatch), tokio::task::JoinError>,
    ) -> Result<(), OwnerError> {
        let (id, outcome) = match joined {
            Ok((id, batch)) => (id, Ok(batch)),
            Err(error) => (error.id(), Err(error)),
        };
        let Some(task) = self.checking.remove(&id) else {
            if self.aborted_checks.remove(&id) || outcome.as_ref().is_err_and(tokio::task::JoinError::is_cancelled) {
                return Ok(());
            }
            return Err(OwnerError::UnknownCheck { id });
        };
        self.aborted_checks.remove(&id);
        let parked = self
            .parked
            .remove(&task)
            .ok_or_else(|| OwnerError::LostTask { task: task.clone() })?;
        let PlanWait {
            settled,
            found,
            earlier,
            run,
            inline_records,
            commands,
        } = parked.wait;
        let mut latest = self.latest_judgements(&task).await?;
        for record in inline_records {
            latest.insert(record.step, record);
        }
        match outcome {
            Ok(batch) => {
                for record in batch.records? {
                    latest.insert(record.step, record);
                }
            }
            Err(error) => {
                // The batch ended without its results: every command of it is a mismatch, with the reason.
                let mut failed = Vec::with_capacity(commands.len());
                for (step, command) in &commands {
                    let reason = format!("the check did not finish: {}", error_chain(&error));
                    failed.push((*step, mismatch(Some(command.clone()), reason)));
                }
                let records = self.records_of(&task, &parked.located, run.as_ref(), failed)?;
                for (step, record) in self.record_judgements(&task, records).await? {
                    latest.insert(step, record);
                }
            }
        }
        let mut mismatch = None;
        if let PlanRead::Parsed(plan) = &found.read {
            self.refresh_task_plan(&task, plan, &latest);
            let task_dir = self.parts.store.home().task_dir(&task);
            mismatch = first_mismatch(&task_dir, plan, &latest, run.as_ref()).await;
        }
        let (settled, held) = self
            .decide_at_end(
                &task,
                &parked.located,
                AtEnd {
                    end: &parked.run.classified.end,
                    found: &found,
                    earlier: &earlier,
                    mismatch,
                },
                settled,
            )
            .await?;
        let result = self.finish(&task, parked.located, parked.run, settled, held).await;
        self.release_run_worktree_targets(&task);
        result
    }

    /// The judgements of a run that waits for no command, recorded.
    async fn known_judgements(
        &mut self,
        task: &TaskId,
        located: &Located,
        run: Option<&RunId>,
        inline: Vec<(ItemId, JudgementResult)>,
    ) -> Result<Judged, OwnerError> {
        let records = self.records_of(task, located, run, inline)?;
        Ok(Judged::Known(self.record_judgements(task, records).await?))
    }

    /// Keeps the commands of the judgements that the gate refused in the plan keeping of `task`, for the next run to be
    /// told once.
    ///
    /// # Errors
    /// [`OwnerError`] when the task is gone.
    fn note_refused_gates(&mut self, task: &TaskId, results: &[(ItemId, JudgementResult)]) -> Result<(), OwnerError> {
        let (_, mut member) = self.member_mut(task)?;
        for (step, command) in refused_gates(results) {
            member.add_plan_refused_gate(step, command);
        }
        Ok(())
    }

    /// The judgement records of `results` of a run of `task`, stamped now.
    ///
    /// # Errors
    /// [`OwnerError`] when the clock cannot give the time.
    fn records_of(
        &self,
        task: &TaskId,
        located: &Located,
        run: Option<&RunId>,
        results: Vec<(ItemId, JudgementResult)>,
    ) -> Result<Vec<JudgementRecord>, OwnerError> {
        let at = Timestamp::now(&self.parts.clock)?;
        Ok(results
            .into_iter()
            .map(|(step, result)| result.into_record(task.clone(), located.team.clone(), run.cloned(), at, step))
            .collect())
    }

    /// Keeps `records` in the history and returns the latest judgement of every step of `task`, as known now.
    pub(super) async fn record_judgements(
        &mut self,
        task: &TaskId,
        records: Vec<JudgementRecord>,
    ) -> Result<HashMap<ItemId, JudgementRecord>, OwnerError> {
        let mut latest = self.latest_judgements(task).await?;
        for record in &records {
            log_decision(
                "plan_check",
                &format!("task={task}, step={}", record.step),
                &format!(
                    "state={}, reason={:?}, output={:?}",
                    record.state.as_str(),
                    record.reason,
                    record.output_path
                ),
            );
            latest.insert(record.step, record.clone());
        }
        if !records.is_empty() && self.history.is_writing() {
            match self.history.persist_judgements(records).await {
                Ok(()) => {}
                Err(error) => {
                    // History degradation keeps the task running; the cached judgement is not durable.
                    tracing::warn!(
                        event = LogEvent::HistoryRecordIncomplete.as_str(),
                        task = %task,
                        error = %error_chain(&error),
                        "plan judgements remain only in memory because history could not keep them"
                    );
                }
            }
        }
        Ok(latest)
    }

    /// Ends the command checks still running when the daemon stops: each is aborted, and the process group it started is
    /// ended from its record, so that no check outlives the daemon.
    ///
    /// # Errors
    /// [`OwnerError`] when a recorded command check process group cannot be ended.
    pub(super) async fn stop_checks(&mut self) -> Result<(), OwnerError> {
        self.checks.abort_all();
        while let Some(joined) = self.checks.join_next_with_id().await {
            if let Err(error) = joined
                && !error.is_cancelled()
            {
                tracing::error!(
                    event = LogEvent::DaemonTaskFailed.as_str(),
                    error = %error_chain(&error),
                    "a command check ended with an error when the daemon stopped"
                );
            }
        }
        let mut first_error: Option<PlanCheckCleanupError> = None;
        let draining: Vec<TaskId> = self.checking.drain().map(|(_id, task)| task).collect();
        for task in draining {
            if let Err(error) = self.recover_task_checks(&task).await {
                tracing::error!(
                    event = LogEvent::DaemonTaskFailed.as_str(),
                    task = %task,
                    error = %error_chain(&error),
                    "the process of a command check could not be ended when the daemon stopped"
                );
                if first_error.is_none() {
                    first_error = Some(error);
                }
            }
        }
        self.aborted_checks.clear();
        self.parked.clear();
        if let Some(error) = first_error {
            return Err(error.into());
        }
        Ok(())
    }

    /// Ends the command checks that a daemon which died left running, from their records, on the blocking thread pool
    /// before any run is looked at so directory traversal and `/proc` scanning never block the async executor.
    ///
    /// # Errors
    /// [`OwnerError`] when the tasks directory cannot be read or a recorded check cannot be ended.
    pub(super) async fn recover_leftover_checks(&self) -> Result<(), OwnerError> {
        let tasks_dir = self.parts.store.home().tasks_dir();
        let dir_for_join = tasks_dir.clone();
        let joined = tokio::task::spawn_blocking(move || end_leftover_checks_in(&tasks_dir))
            .await
            .map_err(|source| PlanCheckCleanupError::TasksDirJoin {
                path: dir_for_join,
                source,
            })?;
        joined.map_err(Into::into)
    }

    /// Synchronous leftover-check helper for direct unit tests; production startup recovery calls
    /// [`Self::recover_leftover_checks`] on the blocking thread pool.
    #[cfg(test)]
    pub(super) fn end_leftover_checks(&self) -> Result<(), OwnerError> {
        end_leftover_checks_in(&self.parts.store.home().tasks_dir()).map_err(Into::into)
    }

    /// Finishes the runs that had ended before a restart and were waiting for their plan command checks or settlement.
    ///
    /// # Errors
    /// [`OwnerError`] when a recovered run cannot be finished or the state cannot be saved.
    pub(super) async fn recover_plan_checks(&mut self) -> Result<(), OwnerError> {
        let pending: Vec<(TaskId, agent_core::state::RunningPlanCheck)> = self
            .parts
            .state
            .teams()
            .iter()
            .flat_map(|team| {
                let crew = team.crew().roster().into_iter().map(|(_, member)| member);
                crew.chain(team.reviewers().iter()).filter_map(|member| {
                    member
                        .plan_running_check()
                        .cloned()
                        .map(|check| (member.task().clone(), check))
                })
            })
            .collect();
        if pending.is_empty() {
            return Ok(());
        }
        for (task, check) in pending {
            let has_current = self
                .parts
                .state
                .find_task(&task)
                .and_then(|found| found.member.current_run())
                .is_some_and(|run| run.id() == &check.run);
            if !has_current {
                if let Some((_, mut member)) = self.parts.state.find_task_mut(&task) {
                    member.set_plan_running_check(None);
                }
                self.save().await?;
                continue;
            }
            let progress = if check.made_progress {
                agent_runtime::run::Progress::Made
            } else {
                agent_runtime::run::Progress::Nothing
            };
            let run = FinishedRun {
                classified: agent_core::decide::outcome::Classified {
                    end: check.end,
                    pause: check.pause,
                },
                finish: agent_core::decide::outcome::RunFinish {
                    exit: agent_core::decide::outcome::ExitStatus::Code(0),
                    stopped_by: agent_core::decide::outcome::StoppedBy::Nobody,
                },
                progress,
                session: None,
                final_message: check.final_message,
            };
            self.finished(&task, run).await?;
        }
        Ok(())
    }

    /// The latest judgements of steps of `task` kept in history, keyed by step ID.
    ///
    /// # Errors
    /// [`OwnerError::PlanJudgementsRead`] when an existing `history.db` cannot be read.
    async fn latest_judgements(&self, task: &TaskId) -> Result<HashMap<ItemId, JudgementRecord>, OwnerError> {
        let path = self.parts.store.home().history_file();
        if !path.exists() {
            return Ok(HashMap::new());
        }
        let task_id = task.clone();
        let looked = tokio::task::spawn_blocking(move || {
            Reader::open(&path).and_then(|reader| reader.latest_judgements(&task_id))
        })
        .await;
        match looked {
            Ok(Ok(map)) => Ok(map),
            Ok(Err(ReadError::Empty { .. })) => Ok(HashMap::new()),
            Ok(Err(source)) => {
                let error = PlanJudgementsError::Read {
                    task: task.clone(),
                    source: Box::new(source),
                };
                tracing::warn!(
                    event = LogEvent::HistoryRecordIncomplete.as_str(),
                    task = %task,
                    reason = %error_chain(&error),
                    "the previous judgements could not be read"
                );
                Err(OwnerError::PlanJudgementsRead(error))
            }
            Err(source) => {
                let error = PlanJudgementsError::Join {
                    task: task.clone(),
                    source,
                };
                tracing::warn!(
                    event = LogEvent::HistoryRecordIncomplete.as_str(),
                    task = %task,
                    reason = %error_chain(&error),
                    "the previous judgements could not be read"
                );
                Err(OwnerError::PlanJudgementsRead(error))
            }
        }
    }

    /// Hands what was found of the plan of `task` at the end of `run` to the history.
    fn keep_plan(&self, task: &TaskId, located: &Located, run: Option<RunId>, found: &Found) -> Result<(), OwnerError> {
        let at = Timestamp::now(&self.parts.clock)?;
        let read = match (&found.read, &found.text) {
            (PlanRead::Parsed(_), Some(text)) => HistoryPlanRead::Version { text: text.clone() },
            (PlanRead::Parsed(plan), None) => HistoryPlanRead::Version { text: plan.to_text() },
            (PlanRead::Missing, _) => HistoryPlanRead::Missing,
            (PlanRead::Unreadable { line, detail }, _) => HistoryPlanRead::Unreadable {
                line: *line,
                detail: detail.clone(),
            },
        };
        self.history.write_plans(vec![PlanRecord {
            task: task.clone(),
            team: located.team.clone(),
            run,
            at,
            read,
        }]);
        Ok(())
    }

    /// Reads and parses the plan file in the worktree of `task` for snapshot recovery.
    async fn read_worktree_plan(&self, task: &TaskId) -> Option<Plan> {
        let located = match self.locate(task) {
            Ok(located) => located,
            Err(error) => {
                tracing::warn!(task = %task, error = %error_chain(&error), "cannot locate task for plan snapshot");
                return None;
            }
        };
        let worktree = match self.worktree_of(task, &located) {
            Ok(worktree) => worktree,
            Err(error) => {
                tracing::warn!(task = %task, error = %error_chain(&error), "cannot locate worktree for plan snapshot");
                return None;
            }
        };
        let plan_file = worktree.as_path().join(PLAN_FILE);
        let text = match read_text_file(&plan_file, MAX_TASK_LIST_BYTES).await {
            Ok(Some(file)) => file.text,
            Ok(None) => return None,
            Err(error) => {
                tracing::warn!(
                    task = %task,
                    path = %plan_file.display(),
                    error = %error_chain(&error),
                    "cannot read plan file for snapshot"
                );
                return None;
            }
        };
        match parse_plan(&text) {
            Ok(plan) => Some(plan),
            Err(error) => {
                tracing::warn!(
                    task = %task,
                    error = %error_chain(&error),
                    "cannot parse plan file for snapshot"
                );
                None
            }
        }
    }

    /// Asynchronously populates the in-memory plan summaries of all existing tasks at daemon recovery.
    pub(super) async fn load_plan_snapshots(&mut self) {
        let history_path = self.parts.store.home().history_file();
        let is_writing = self.history.is_writing();
        let mut parsed_tasks = Vec::new();
        for team in self.parts.state.teams() {
            let mut tasks = Vec::new();
            for (_role, member) in team.crew().roster() {
                tasks.push(member.task().clone());
            }
            for reviewer in team.reviewers() {
                tasks.push(reviewer.task().clone());
            }
            for task in tasks {
                if let Some(plan) = self.read_worktree_plan(&task).await {
                    parsed_tasks.push((task, plan));
                }
            }
        }

        let mut loaded = HashMap::new();
        for (task, plan) in parsed_tasks {
            let has_done = plan.steps().any(|s| s.state() == StepState::Done);

            let (judgements, earlier_by_run) = if has_done && is_writing {
                let path = history_path.clone();
                let task_id = task.clone();
                let task_runs: Vec<RunId> = self
                    .parts
                    .state
                    .find_task(&task)
                    .map(|found| found.member.runs().iter().map(|r| r.id().clone()).collect())
                    .unwrap_or_default();
                let looked = tokio::task::spawn_blocking(move || {
                    let reader = Reader::open(&path)?;
                    let latest = reader.latest_judgements(&task_id)?;
                    let records = reader.plan_versions(&task_id)?;
                    let parsed: Vec<_> = records.iter().map(|record| (record, retained_plan(record))).collect();
                    let by_run = expand_plans_by_run(&task_runs, &parsed);
                    Ok::<_, ReadError>((latest, by_run))
                })
                .await;

                match looked {
                    Ok(Ok(pair)) => pair,
                    Ok(Err(error)) => {
                        tracing::warn!(
                            event = LogEvent::HistoryRecordIncomplete.as_str(),
                            task = %task,
                            error = %error_chain(&error),
                            "latest plan judgements could not be read for snapshot"
                        );
                        (HashMap::new(), HashMap::new())
                    }
                    Err(join_err) => {
                        tracing::warn!(
                            event = LogEvent::HistoryRecordIncomplete.as_str(),
                            task = %task,
                            error = %join_err,
                            "failed to join task reading judgements for snapshot"
                        );
                        (HashMap::new(), HashMap::new())
                    }
                }
            } else {
                (HashMap::new(), HashMap::new())
            };

            let summaries: Vec<PlanStepSummary> = plan
                .steps()
                .map(|step| step_summary(step, &judgements, &earlier_by_run))
                .collect();
            loaded.insert(task, summaries);
        }

        self.task_plans = loaded;
    }

    /// Stores the parsed `plan` and latest `judgements` of `task` in cache for status snapshots.
    pub(super) fn refresh_task_plan(
        &mut self,
        task: &TaskId,
        plan: &Plan,
        judgements: &HashMap<ItemId, JudgementRecord>,
    ) {
        let summaries: Vec<PlanStepSummary> = plan
            .steps()
            .map(|step| {
                let is_done = step.state() == StepState::Done;
                let judgement = if is_done { judgements.get(&step.id()) } else { None };
                PlanStepSummary {
                    id: step.id(),
                    mark: step.state(),
                    title: step.title().to_owned(),
                    state: judgement.map(|j| j.state),
                    reason: judgement.and_then(|j| j.reason.clone()),
                }
            })
            .collect();
        self.task_plans.insert(task.clone(), summaries);
    }

    /// Removes `task`'s plan summaries from the status snapshot cache.
    pub(super) fn invalidate_task_plan(&mut self, task: &TaskId) {
        self.task_plans.remove(task);
    }
}

/// Builds a [`PlanStepSummary`] for a step from the recovered plan and latest judgements.
fn step_summary(
    step: &agent_core::plan::Step,
    judgements: &HashMap<ItemId, JudgementRecord>,
    earlier_by_run: &HashMap<RunId, Plan>,
) -> PlanStepSummary {
    let is_done = step.state() == StepState::Done;
    let (state, reason) = if is_done {
        match judgements.get(&step.id()) {
            Some(j) => {
                let step_at_run = j
                    .run
                    .as_ref()
                    .and_then(|r| earlier_by_run.get(r))
                    .and_then(|p| p.step(&step.id()));
                if step_at_run == Some(step) {
                    (Some(j.state), j.reason.clone())
                } else {
                    (None, None)
                }
            }
            None => (None, None),
        }
    } else {
        (None, None)
    };
    PlanStepSummary {
        id: step.id(),
        mark: step.state(),
        title: step.title().to_owned(),
        state,
        reason,
    }
}

/// Parses a retained plan, logging why an invalid version cannot supply a snapshot.
fn retained_plan(record: &PlanRecord) -> Option<Plan> {
    match &record.read {
        HistoryPlanRead::Missing | HistoryPlanRead::Unreadable { .. } => None,
        HistoryPlanRead::Version { text } => match parse_plan(text) {
            Ok(plan) => Some(plan),
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::HistoryRecordIncomplete.as_str(),
                    task = %record.task,
                    error = %error_chain(&error),
                    "cannot parse retained plan for snapshot"
                );
                None
            }
        },
    }
}

/// Expands deduplicated plan versions across chronological runs, carrying forward the active plan.
fn expand_plans_by_run(task_runs: &[RunId], records: &[(&PlanRecord, Option<Plan>)]) -> HashMap<RunId, Plan> {
    let mut by_run = HashMap::new();
    let mut active_plan: Option<Plan> = None;
    let mut record_iter = records.iter().peekable();

    for run_id in task_runs {
        while let Some((record, plan)) = record_iter.peek() {
            match &record.run {
                None => active_plan.clone_from(plan),
                Some(r) if r == run_id => active_plan.clone_from(plan),
                Some(_) => break,
            }
            record_iter.next();
        }
        if let Some(plan) = &active_plan {
            by_run.insert(run_id.clone(), plan.clone());
        }
    }
    for (record, plan) in records {
        if let (Some(run_id), Some(plan)) = (&record.run, plan) {
            by_run.entry(run_id.clone()).or_insert_with(|| plan.clone());
        }
    }
    by_run
}

/// Whether the member `agent` of `team` still works on its item: it does unless it waits after a run that ended
/// `DONE`, which hands the item to the coordinator. A name the crew does not have works, which keeps the turn open.
fn member_works(team: &Team, agent: &AgentName) -> bool {
    let Some((_, member)) = team
        .crew()
        .roster()
        .into_iter()
        .find(|(_, member)| member.agent().name() == agent.as_str())
    else {
        return true;
    };
    let finished = member.status() == TaskStatus::Waiting
        && member.runs().last().is_some_and(|run| {
            matches!(
                run.state(),
                RunState::Ended {
                    end: RunEnd::Reported {
                        outcome: RunOutcome::Done
                    },
                    ..
                }
            )
        });
    !finished
}

/// What the file `path` of a plan holds: missing, unreadable with why, or the plan with its text.
async fn read_plan(path: &Path) -> Found {
    let read = match read_text_file(path, MAX_TASK_LIST_BYTES).await {
        Ok(None) => PlanRead::Missing,
        Ok(Some(file)) => match parse_plan(&file.text) {
            Ok(plan) => {
                return Found {
                    read: PlanRead::Parsed(plan),
                    text: Some(file.text),
                };
            }
            Err(error) => PlanRead::unreadable(&error),
        },
        Err(TextFileError::TooLarge { limit }) => PlanRead::unreadable(&PlanError::TooLarge {
            len: limit.saturating_add(1),
            max: limit,
        }),
        Err(error) => PlanRead::Unreadable {
            line: None,
            detail: error_chain(&error),
        },
    };
    Found { read, text: None }
}

/// What the human's answer `file` to the question about a plan says: the recommendation (`ok`) or an answer that starts
/// with `a` is "go on as it is"; any other answer, the human's own words too, sends the run back once more, so that the
/// agent reads it with the answers.
fn plan_answer(file: &QuestionsFile) -> PlanAnswer {
    match file.entries().first().and_then(Entry::verdict) {
        Some(Verdict::Accept) => PlanAnswer::GoOn,
        Some(Verdict::Own(text)) if starts_as_option_a(text.as_str()) => PlanAnswer::GoOn,
        Some(Verdict::Own(_)) | None => PlanAnswer::OnceMore,
    }
}

/// Whether `answer` names the option `a`: `a` alone, or `a` followed by what is not a letter or a digit (`a)`, `a —`).
fn starts_as_option_a(answer: &str) -> bool {
    let mut chars = answer.trim_start().chars();
    matches!(chars.next(), Some('a' | 'A')) && chars.next().is_none_or(|next| !next.is_alphanumeric())
}

/// The permission bits of a questions file the harness writes: the human's to read and edit.
const QUESTIONS_MODE: u32 = 0o600;

/// Why the question about a plan cannot be written.
#[derive(Debug, thiserror::Error)]
enum PlanQuestionError {
    /// The texts do not take their values.
    #[error("the question cannot be made")]
    Compose(#[from] ComposeError),
    /// A variable of the question is not valid.
    #[error("a variable of the question is not valid")]
    Variable(#[from] VariablesError),
    /// The question cannot be rendered.
    #[error("the question cannot be rendered")]
    Render(#[from] RenderTextError),
    /// The directory of the task's files cannot be made.
    #[error("cannot make the directory {path:?}")]
    Directory {
        /// The directory.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
    /// The file cannot be written.
    #[error("the questions file cannot be written")]
    Write(#[from] ReplaceError),
}

/// The correction a run sent back for `reason` is told: a text of the catalog for each reason.
///
/// # Errors
/// [`ComposeError`] when the text does not take its values: a mistake of the prompt files, which the daemon checks
/// when it starts.
pub(super) fn correction(texts: &HarnessPrompts, reason: &PlanReason) -> Result<String, ComposeError> {
    let steps =
        |steps: &[agent_core::ids::ItemId]| steps.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
    let text = match reason {
        PlanReason::Missing => texts.render(HarnessPrompt::PlanMissing, &Variables::new())?,
        PlanReason::Unreadable { detail, .. } => texts.render(
            HarnessPrompt::PlanUnreadable,
            &Variables::new().with("error", detail.as_str())?,
        )?,
        PlanReason::Unfinished { steps: ids } => texts.render(
            HarnessPrompt::PlanUnfinished,
            &Variables::new().with("steps", steps(ids))?,
        )?,
        PlanReason::LostStep { steps: ids } => texts.render(
            HarnessPrompt::PlanLostStep,
            &Variables::new().with("steps", steps(ids))?,
        )?,
        PlanReason::Mismatch {
            step,
            title,
            check,
            reason,
            tail,
        } => texts.render(
            HarnessPrompt::PlanMismatch,
            &Variables::new()
                .with("step", step.to_string())?
                .with("title", title.as_str())?
                .with("check", check.as_deref().unwrap_or(""))?
                .with("reason", reason.as_deref().unwrap_or(""))?
                .with("output", tail.as_str())?,
        )?,
        PlanReason::FaultAndMismatch { fault, mismatch } => {
            let fault_text = match fault {
                PlanFault::Unfinished { steps: ids } => texts.render(
                    HarnessPrompt::PlanUnfinished,
                    &Variables::new().with("steps", steps(ids))?,
                )?,
                PlanFault::LostStep { steps: ids } => texts.render(
                    HarnessPrompt::PlanLostStep,
                    &Variables::new().with("steps", steps(ids))?,
                )?,
            };
            let mismatch_text = texts.render(
                HarnessPrompt::PlanMismatch,
                &Variables::new()
                    .with("step", mismatch.step.to_string())?
                    .with("title", mismatch.title.as_str())?
                    .with("check", mismatch.check.as_deref().unwrap_or(""))?
                    .with("reason", mismatch.reason.as_deref().unwrap_or(""))?
                    .with("output", mismatch.tail.as_str())?,
            )?;
            format!("{fault_text}\n\n{mismatch_text}")
        }
    };
    Ok(text)
}

/// Scans `tasks_dir` on a blocking thread and ends any command check process groups left running by a dead daemon.
fn end_leftover_checks_in(tasks_dir: &Path) -> Result<(), PlanCheckCleanupError> {
    let entries = match std::fs::read_dir(tasks_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            let error = PlanCheckCleanupError::TasksDirRead {
                path: tasks_dir.to_path_buf(),
                source,
            };
            tracing::error!(
                event = LogEvent::DaemonTaskFailed.as_str(),
                error = %error_chain(&error),
                "cannot read the tasks directory to end leftover command checks"
            );
            return Err(error);
        }
    };
    let mut first_error: Option<PlanCheckCleanupError> = None;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(source) => {
                let error = PlanCheckCleanupError::TasksDirEntry {
                    path: tasks_dir.to_path_buf(),
                    source,
                };
                tracing::error!(
                    event = LogEvent::DaemonTaskFailed.as_str(),
                    error = %error_chain(&error),
                    "cannot read an entry of the tasks directory for leftover command checks"
                );
                if first_error.is_none() {
                    first_error = Some(error);
                }
                continue;
            }
        };
        let path = entry.path();
        let Ok(name) = entry.file_name().into_string() else {
            let error = PlanCheckCleanupError::InvalidTaskEntry { path };
            tracing::error!(
                event = LogEvent::DaemonTaskFailed.as_str(),
                error = %error_chain(&error),
                "a task directory name is not valid UTF-8"
            );
            if first_error.is_none() {
                first_error = Some(error);
            }
            continue;
        };
        let Ok(task) = TaskId::from_str(&name) else {
            let error = PlanCheckCleanupError::InvalidTaskEntry { path };
            tracing::error!(
                event = LogEvent::DaemonTaskFailed.as_str(),
                error = %error_chain(&error),
                "a directory in tasks/ does not have a valid task ID"
            );
            if first_error.is_none() {
                first_error = Some(error);
            }
            continue;
        };
        if let Err(source) = recover_checks(&tasks_dir.join(task.as_str())) {
            let error = PlanCheckCleanupError::Recover {
                task: task.clone(),
                source,
            };
            tracing::error!(
                event = LogEvent::DaemonTaskFailed.as_str(),
                task = %task,
                error = %error_chain(&error),
                "a command check left by a dead daemon could not be ended"
            );
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
