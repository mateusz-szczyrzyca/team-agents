//! The owner's side of the plan of a task (`.agent/plan.md`, see `docs/plan.md`): what the daemon does with it at the
//! end of a run.

use std::str::FromStr;

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use agent_core::config::{AbsolutePath, InvalidValue};
use agent_core::ids::{AgentName, InvalidName, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::plan::PLAN_FILE;
use agent_core::prompt::{HarnessPrompt, Variables, VariablesError};
use agent_core::questions::{Entry, QuestionsFile, Verdict, quote};
use agent_core::repo_path::{InvalidRepoPath, RepoPath};
use agent_core::state::{PlanKeeping, PlanReason, PlanStage};
use agent_runtime::git::{CommitIdentity, Git, GitError, Untracked};
use agent_runtime::harness_prompts::{HarnessPrompts, RenderTextError};
use agent_runtime::history::log_decision;
use agent_runtime::process::error_chain;
use agent_runtime::run::compose::ComposeError;
use agent_runtime::text_file::{ReplaceError, replace_text_file};

use super::start::next_number;
use super::{Located, Owner, OwnerError};

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
    /// Git failed.
    #[error("git cannot take the plan out of the branch")]
    Git(#[source] GitError),
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
            Ok(untracked) => log_decision("plan_out_of_delivery", &task.as_str(), &format!("{untracked:?}")),
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
    async fn untrack_plan(&self, task: &TaskId, located: &Located) -> Result<Untracked, OutOfDeliveryError> {
        let worktree = self.worktree_of(task, located)?;
        let file = worktree.as_path().join(PLAN_FILE);
        match tokio::fs::try_exists(&file).await {
            Ok(true) => {}
            Ok(false) => return Ok(Untracked::NotTracked),
            Err(source) => return Err(OutOfDeliveryError::Look { path: file, source }),
        }
        let plan = RepoPath::from_str(PLAN_FILE).map_err(OutOfDeliveryError::PlanPath)?;
        let agent = AgentName::from_str(&located.agent).map_err(|source| OutOfDeliveryError::Agent {
            name: located.agent.clone(),
            source,
        })?;
        Git::new(
            worktree,
            CommitIdentity::for_agent(self.parts.settings.config.git(), &agent),
        )
        .untrack(&plan, OUT_OF_DELIVERY)
        .await
        .map_err(OutOfDeliveryError::Git)
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
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "called at the end of a run, wired by step 6 of this item")
)]
pub(super) enum PlanQuestion {
    /// The task's next questions file holds it, and the task's plan waits for its answer: the run's end is settled as
    /// a run that asked questions.
    Written,
    /// It could not be made or written (logged): the run's end is settled as the done it reported.
    NotWritten,
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
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "called at the end of a run, wired by step 6 of this item")
    )]
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
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "called at the end of a run, wired by step 6 of this item")
    )]
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
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "called at the end of a run, wired by step 6 of this item")
)]
const QUESTIONS_MODE: u32 = 0o600;

/// Why the question about a plan cannot be written.
#[derive(Debug, thiserror::Error)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "called at the end of a run, wired by step 6 of this item")
)]
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
