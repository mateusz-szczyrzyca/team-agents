//! The owner's side of the plan of a task (`.agent/plan.md`, see `docs/plan.md`): what the daemon does with it at the
//! end of a run.

use std::str::FromStr;

use std::path::PathBuf;

use agent_core::config::{AbsolutePath, InvalidValue};
use agent_core::ids::{AgentName, InvalidName, TaskId, TeamId};
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::plan::PLAN_FILE;
use agent_core::prompt::{HarnessPrompt, Variables};
use agent_core::repo_path::{InvalidRepoPath, RepoPath};
use agent_core::state::PlanReason;
use agent_runtime::git::{CommitIdentity, Git, GitError, Untracked};
use agent_runtime::harness_prompts::HarnessPrompts;
use agent_runtime::history::log_decision;
use agent_runtime::process::error_chain;
use agent_runtime::run::compose::ComposeError;

use super::{Located, Owner};

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
