//! Which item goes to which member next, and the branch names that go with it.

use agent_core::decide::assign::{AssignError, AssignmentRequest, next_assignments};
use agent_core::ids::{AgentName, BranchName, InvalidBranchName, InvalidName, ItemId, TaskId};
use agent_core::kinds::Role;
use agent_core::state::{AdvanceItemError, ItemPhase, Participant, Team, TeamMut};
use agent_core::status::TaskStatus;

/// An item given to a member, with the branch the member works on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon) struct Assigned {
    /// The item that starts.
    pub(in crate::daemon) item: ItemId,
    /// The task of the member that works on it.
    pub(in crate::daemon) task: TaskId,
    /// The member.
    pub(in crate::daemon) agent: AgentName,
    /// The branch of the item: the member's worktree has it out while the member works on the item.
    pub(in crate::daemon) branch: BranchName,
}

/// Why the assignments of a team cannot be decided.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AssignmentError {
    /// The progress does not fit the task list.
    #[error("cannot decide which item goes to which member")]
    Decide(#[from] AssignError),
    /// A member's name is not an agent name.
    #[error("the name of a member of the team is not an agent name")]
    Name(#[from] InvalidName),
    /// The decision names a member that was not offered an item: a mistake of the decision.
    #[error("{member} was given an item but was not offered one")]
    UnknownMember {
        /// The member.
        member: AgentName,
    },
    /// The branch of an item is not a valid branch name.
    #[error("the branch of item {item} of task {task} is not a valid branch name")]
    Branch {
        /// The item.
        item: ItemId,
        /// The member's task.
        task: TaskId,
        /// What is wrong with the name.
        source: InvalidBranchName,
    },
}

/// The name of `participant` as an agent name: the three tiers share one namespace.
///
/// # Errors
/// [`InvalidName`] for a name that is not an agent name: a state built through its checks has none.
pub(in crate::daemon) fn agent_name(participant: &Participant) -> Result<AgentName, InvalidName> {
    participant.name().parse()
}

/// The items that start now and the members that take them, as [`assignable`] decides from `team`.
///
/// # Errors
/// [`AssignmentError`].
pub(in crate::daemon) fn assignable(team: &Team) -> Result<Vec<Assigned>, AssignmentError> {
    // Whoever takes items and is not at work: a member that waits for its next item, or has not started yet. A member
    // that is running would not see an item given now; it is looked at again when its run ends.
    let takers = team
        .crew()
        .roster()
        .into_iter()
        .filter(|(role, member)| {
            matches!(role, Role::Solo | Role::Member)
                && matches!(member.status(), TaskStatus::Queued | TaskStatus::Waiting)
        })
        .map(|(_, member)| Ok((agent_name(member.agent())?, member.task())))
        .collect::<Result<Vec<_>, InvalidName>>()?;
    let available: Vec<AgentName> = takers.iter().map(|(name, _)| name.clone()).collect();
    let decided = next_assignments(&AssignmentRequest {
        list: team.task_list().list(),
        progress: team.items(),
        merge: team.merge(),
        available: &available,
    })?;
    decided
        .into_iter()
        .map(|given| {
            let task = takers
                .iter()
                .find(|(name, _)| *name == given.member)
                .map(|(_, task)| *task)
                .ok_or_else(|| AssignmentError::UnknownMember {
                    member: given.member.clone(),
                })?;
            let branch = item_branch(&given.member, task, given.item).map_err(|source| AssignmentError::Branch {
                item: given.item,
                task: task.clone(),
                source,
            })?;
            Ok(Assigned {
                item: given.item,
                task: task.clone(),
                agent: given.member,
                branch,
            })
        })
        .collect()
}

/// The branch a member works `item` on.
///
/// # Errors
/// [`InvalidBranchName`] when the names make no branch.
pub(in crate::daemon) fn item_branch(
    agent: &AgentName,
    task: &TaskId,
    item: ItemId,
) -> Result<BranchName, InvalidBranchName> {
    format!("agents/{agent}/{task}-{item}").parse()
}

/// The branch a task's worktree starts on, before it works on an item.
///
/// # Errors
/// [`InvalidBranchName`] when the names make no branch.
pub(in crate::daemon) fn task_branch(agent: &AgentName, task: &TaskId) -> Result<BranchName, InvalidBranchName> {
    format!("agents/{agent}/{task}").parse()
}

/// A whole-team handle through which an assigned item can change phase.
pub(in crate::daemon) trait AssignedTeam {
    /// Moves `item` to `next`, keeping the team's item and round invariants.
    fn advance_assigned(&mut self, item: ItemId, next: ItemPhase) -> Result<(), AdvanceItemError>;
}

impl AssignedTeam for Team {
    fn advance_assigned(&mut self, item: ItemId, next: ItemPhase) -> Result<(), AdvanceItemError> {
        self.advance_item(item, next)
    }
}

impl AssignedTeam for TeamMut<'_> {
    fn advance_assigned(&mut self, item: ItemId, next: ItemPhase) -> Result<(), AdvanceItemError> {
        self.advance_item(item, next)
    }
}

/// Records that `assigned` works on its item.
///
/// # Errors
/// [`AdvanceItemError`] when the item is not pending, is not in the team, or has no round.
pub(in crate::daemon) fn start_item<T: AssignedTeam>(
    team: &mut T,
    assigned: &Assigned,
) -> Result<(), AdvanceItemError> {
    team.advance_assigned(
        assigned.item,
        ItemPhase::InProgress {
            assignee: assigned.agent.clone(),
            branch: assigned.branch.clone(),
        },
    )
}

#[cfg(test)]
mod tests;
