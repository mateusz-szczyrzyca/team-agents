//! The owner's side of the plan of a task (`.agent/plan.md`, see `docs/plan.md`): what the daemon does with it at the
//! end of a run.

use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use agent_core::config::{AbsolutePath, InvalidValue, RepoName};
use agent_core::decide::plan::{DoneEnds, PlanFacts, PlanRead, PlanVerdict, decide_plan, done_ends_with};
use agent_core::ids::RunId;
use agent_core::ids::{AgentName, BranchName, InvalidName, ItemId, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::plan::judge::{JudgementState, steps_to_judge};
use agent_core::plan::{PLAN_FILE, Plan, PlanError, parse_plan};
use agent_core::prompt::{HarnessPrompt, Variables, VariablesError};
use agent_core::questions::{Entry, QuestionsFile, Verdict, quote};
use agent_core::repo_path::{InvalidRepoPath, RepoPath};
use agent_core::state::{ItemProgress, PlanKeeping, PlanReason, PlanStage};
use agent_core::state::{RunEnd, RunState, Team, Timestamp};
use agent_core::status::{RunOutcome, TaskStatus};
use agent_core::task_list::MAX_TASK_LIST_BYTES;
use agent_runtime::git::{CommitIdentity, Git, GitError, Revision, Untracked};
use agent_runtime::harness_prompts::{HarnessPrompts, RenderTextError};
use agent_runtime::history::{JudgementRecord, PlanRead as HistoryPlanRead, PlanRecord, Reader, log_decision};
use agent_runtime::plan::check::{CheckContext, JudgementResult, execute_step_check};
use agent_runtime::process::error_chain;
use agent_runtime::run::compose::ComposeError;
use agent_runtime::text_file::{ReplaceError, TextFileError, read_text_file, replace_text_file};

use super::start::next_number;
use super::{Located, Owner, OwnerError};
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

/// What the end of a run found of the plan: what the decision reads, and the text as the task wrote it.
struct Found {
    /// What the decision reads.
    read: PlanRead,
    /// The text of a plan that parses, as written; `None` otherwise.
    text: Option<String>,
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
    /// What the end `end` of a run of `task` does with its plan: what the worktree has is kept in the history, and a
    /// run that reported `DONE` against its plan is sent back or the human asked, by [`decide_plan`].
    /// `settled` is what the run's end comes to otherwise; the result is what it comes to now.
    ///
    /// # Errors
    /// [`OwnerError`] when the task is gone or has no questions number left.
    pub(super) async fn plan_at_end(
        &mut self,
        task: &TaskId,
        located: &Located,
        end: &RunEnd,
        settled: Settled,
    ) -> Result<(Settled, PlanHeld), OwnerError> {
        let found = match self.worktree_of(task, located) {
            Ok(worktree) => read_plan(&worktree.as_path().join(PLAN_FILE)).await,
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::GitCallFailed.as_str(),
                    task = %task,
                    error = %error_chain(&error),
                    "the plan of the task cannot be looked for"
                );
                return Ok((settled, PlanHeld::No));
            }
        };
        let run = self
            .parts
            .state
            .find_task(task)
            .and_then(|found| found.member.current_run().map(|run| run.id().clone()));
        let earlier = if matches!(found.read, PlanRead::Parsed(_)) {
            self.earlier_plans(task, run.as_ref()).await
        } else {
            Vec::new()
        };
        self.keep_plan(task, located, run.clone(), &found)?;
        let _judged_steps = if let PlanRead::Parsed(plan) = &found.read {
            let (res, latest) = self
                .judge_steps_at_end(task, located, run.as_ref(), plan, earlier.last())
                .await?;
            self.refresh_task_plan(task, plan, &latest);
            res
        } else {
            self.invalidate_task_plan(task);
            Vec::new()
        };
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
            decide_plan(PlanFacts {
                role: located.role,
                end,
                read: &found.read,
                earlier: &earlier,
                keeping: member.plan(),
                policy: self.parts.settings.config.plan(),
            })
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

    /// The versions of the plan of `task` the history kept from runs other than `run`, oldest first; none when the
    /// history cannot be read (logged): a step lost since then is then not found. A kept version that does not parse is
    /// left out and logged.
    async fn earlier_plans(&self, task: &TaskId, run: Option<&RunId>) -> Vec<Plan> {
        let path = self.parts.store.home().history_file();
        let task_id = task.clone();
        let looked =
            tokio::task::spawn_blocking(move || Reader::open(&path).and_then(|reader| reader.plan_versions(&task_id)))
                .await;
        let records = match looked {
            Ok(Ok(records)) => records,
            Ok(Err(error)) => {
                // A home without a history (or one disabled) has kept no plan: nothing can be found lost.
                tracing::debug!(task = %task, error = %error_chain(&error), "no plan kept before can be read");
                return Vec::new();
            }
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::HistoryRecordIncomplete.as_str(),
                    task = %task,
                    %error,
                    "the plans kept before cannot be read: no step is found lost"
                );
                return Vec::new();
            }
        };
        let mut plans = Vec::new();
        for record in records.into_iter().filter(|record| record.run.as_ref() != run) {
            let HistoryPlanRead::Version { text } = record.read else {
                continue;
            };
            match parse_plan(&text) {
                Ok(plan) => plans.push(plan),
                Err(error) => tracing::warn!(
                    event = LogEvent::HistoryRecordIncomplete.as_str(),
                    task = %task,
                    %error,
                    "a plan kept before does not parse: its steps are not looked for"
                ),
            }
        }
        plans
    }

    /// Executes and records the step checks of `plan` that are to be judged at the end of `run`.
    async fn judge_steps_at_end<'a>(
        &mut self,
        task: &TaskId,
        located: &Located,
        run: Option<&RunId>,
        plan: &'a Plan,
        earlier_plan: Option<&Plan>,
    ) -> Result<
        (
            Vec<(&'a agent_core::plan::Step, JudgementResult)>,
            HashMap<ItemId, JudgementRecord>,
        ),
        OwnerError,
    > {
        let Ok(worktree) = self.worktree_of(task, located) else {
            return Ok((Vec::new(), HashMap::new()));
        };
        let mut previous = self.latest_judgements(task).await;
        let to_judge = steps_to_judge(plan, earlier_plan, |id| previous.get(id).map(|j| j.state));
        let task_dir = self.parts.store.home().task_dir(task);
        let context = CheckContext {
            worktree: worktree.as_path(),
            task_dir: &task_dir,
            run,
            check_seconds: 0,
            check_refuse: &[],
            envs: &[],
        };
        let mut records = Vec::new();
        let mut results = Vec::new();
        for step in to_judge {
            let result = match execute_step_check(&context, step).await {
                Ok(res) => res,
                Err(error) => JudgementResult {
                    state: JudgementState::Mismatch,
                    check_line: step.check().map(|c| match c {
                        agent_core::plan::Check::Command(cmd) => cmd.clone(),
                        agent_core::plan::Check::Exists(p) => format!("exists {}", p.as_str()),
                        agent_core::plan::Check::Reviewer => "reviewer".to_owned(),
                    }),
                    reason: Some(format!("cannot run check: {}", error_chain(&error))),
                    duration_ms: 0,
                    output_path: None,
                    tail: String::new(),
                    gate_refused: false,
                },
            };
            let at = Timestamp::now(&self.parts.clock)?;
            records.push(JudgementRecord {
                task: task.clone(),
                team: located.team.clone(),
                run: run.cloned(),
                at,
                step: step.id(),
                state: result.state,
                check: result.check_line.clone(),
                reason: result.reason.clone(),
                duration_ms: result.duration_ms,
                output_path: result.output_path.clone(),
            });
            results.push((step, result));
        }
        for record in &records {
            previous.insert(record.step, record.clone());
        }
        if !records.is_empty() {
            self.history.persist_judgements(records).await?;
        }
        Ok((results, previous))
    }

    /// The latest judgements of steps of `task` kept in history, keyed by step ID.
    async fn latest_judgements(&self, task: &TaskId) -> HashMap<ItemId, JudgementRecord> {
        let path = self.parts.store.home().history_file();
        let task_id = task.clone();
        let looked = tokio::task::spawn_blocking(move || {
            Reader::open(&path).and_then(|reader| reader.latest_judgements(&task_id))
        })
        .await;
        match looked {
            Ok(Ok(map)) => map,
            Ok(Err(error)) => {
                tracing::debug!(task = %task, error = %error_chain(&error), "no previous judgements could be read");
                HashMap::new()
            }
            Err(error) => {
                tracing::warn!(
                    event = LogEvent::HistoryRecordIncomplete.as_str(),
                    task = %task,
                    %error,
                    "the previous judgements could not be read"
                );
                HashMap::new()
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
    };
    Ok(text)
}

#[cfg(test)]
mod tests;
