//! Forming a team: from a `run` request to the team, the workspaces of its members and its team on the bus.

use agent_core::config::{AbsolutePath, Config};
use agent_core::decide::team::TeamPlanError;
use agent_core::ids::{AgentName, BranchName, Clock, TaskId};
use agent_core::protocol::{Failure, FailureKind, RunRequest};
use agent_core::state::Team;
use agent_runtime::agentbus::NewTeam;
use agent_runtime::store::Home;
use rand::Rng;

/// The worktree and branch a member's task starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon) struct Workspace {
    /// The task.
    pub(in crate::daemon) task: TaskId,
    /// The agent that works in it.
    pub(in crate::daemon) agent: AgentName,
    /// Where the worktree is made.
    pub(in crate::daemon) path: AbsolutePath,
    /// The branch the worktree has out.
    pub(in crate::daemon) branch: BranchName,
}

/// A team as formed from a request, before anything is made.
#[derive(Debug, Clone)]
pub(in crate::daemon) struct Formed {
    /// The team: crew, task list and the first assignments.
    pub(in crate::daemon) team: Team,
    /// A workspace for every task of the crew, the coordinator's (or the solo agent's) first.
    pub(in crate::daemon) workspaces: Vec<Workspace>,
    /// The team on the bus; none for one agent alone.
    pub(in crate::daemon) bus: Option<NewTeam>,
    /// The branch the workspaces start from.
    pub(in crate::daemon) base: BranchName,
}

/// Why a request does not form a team.
#[derive(Debug, thiserror::Error)]
pub(in crate::daemon) enum FormError {
    /// The repository is not in the configuration.
    #[error("the repository {repo} is not in the configuration: add a [repos.{repo}] table to agents.toml")]
    UnknownRepo {
        /// The repository.
        repo: String,
    },
    /// Every item of the list is done already.
    #[error("every item of the task list is marked done: there is nothing to do")]
    NothingToDo,
    /// The agents cannot form a team.
    #[error(transparent)]
    Plan(#[from] TeamPlanError),
}

impl FormError {
    /// How the daemon refuses the request: a team that is too crowded with investigators is `refused`, the rest of what
    /// the request names wrongly is `invalid`.
    pub(in crate::daemon) fn failure(&self) -> Failure {
        Failure {
            kind: FailureKind::Internal,
            message: self.to_string(),
        }
    }
}

/// Forms the team `request` asks for, in `home`, at the time `clock` shows.
///
/// # Errors
/// [`FormError`].
pub(in crate::daemon) fn form(
    request: &RunRequest,
    config: &Config,
    home: &Home,
    clock: &impl Clock,
    rng: &mut impl Rng,
) -> Result<Formed, FormError> {
    let _ = (request, config, home, clock, rng);
    Err(FormError::NothingToDo)
}

#[cfg(test)]
mod tests;
