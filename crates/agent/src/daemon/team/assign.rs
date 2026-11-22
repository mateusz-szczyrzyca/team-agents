//! Which item goes to which member next, and the branch names that go with it.

use agent_core::decide::assign::dispatch::{dispatch_with_candidates, required_tier};
use agent_core::decide::replacement::ReplacementMode;
use agent_core::ids::{AgentName, BranchName, InvalidBranchName, InvalidName, ItemId, TaskId};
use agent_core::state::{AdvanceError, ItemMut, ItemPhase, Participant, Team};

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
    assignable_in_mode(team, ReplacementMode::Auto)
}

/// The items that start now and the members that take them under `mode`, as decided from `team`.
///
/// # Errors
/// [`AssignmentError`].
pub(in crate::daemon) fn assignable_in_mode(
    team: &Team,
    mode: ReplacementMode,
) -> Result<Vec<Assigned>, AssignmentError> {
    assignable_in_policy(team, mode, &[])
}

/// The items that start now and the members that take them under `mode` and `configured_replacements`, as decided from `team`.
///
/// # Errors
/// [`AssignmentError`].
pub(in crate::daemon) fn assignable_in_policy(
    team: &Team,
    mode: ReplacementMode,
    configured_replacements: &[AgentName],
) -> Result<Vec<Assigned>, AssignmentError> {
    assignable_with_candidates(team, mode, configured_replacements, &[])
}

/// Assigns split parts using real outside participants and their availability.
///
/// # Errors
/// [`AssignmentError`] if the selected participant has no reusable task or branch.
pub(in crate::daemon) fn assignable_with_candidates(
    team: &Team,
    mode: ReplacementMode,
    configured_replacements: &[AgentName],
    outside: &[agent_core::decide::part::Candidate<'_>],
) -> Result<Vec<Assigned>, AssignmentError> {
    let takers = team
        .crew()
        .roster()
        .into_iter()
        .map(|(_, member)| Ok((agent_name(member.agent())?, member.task())))
        .collect::<Result<Vec<_>, InvalidName>>()?;
    let available: Vec<AgentName> = takers.iter().map(|(name, _)| name.clone()).collect();
    let decided = dispatch_with_candidates(team, mode, configured_replacements, outside).assigned;
    if !decided.is_empty() {
        let span = tracing::error_span!("team", team = %team.id());
        let _entered = span.enter();
        let formatted: Vec<String> = decided
            .iter()
            .map(|(item, agent)| format!("{item} -> {}", agent.name()))
            .collect();
        agent_runtime::history::log_decision(
            "assignable",
            &format!("team={}, available={available:?}", team.id()),
            &formatted,
        );
        log_split_assignments(team, &decided, mode);
    }
    decided
        .into_iter()
        .map(|(item, agent)| {
            let member = agent_name(&agent)?;
            let task = takers
                .iter()
                .find(|(name, _)| *name == member)
                .map(|(_, task)| *task)
                .or_else(|| {
                    // Outside replacements keep the preceding member's task and its durable history.
                    team.rounds()
                        .iter()
                        .rev()
                        .find(|round| round.item() == Some(item))
                        .and_then(|round| {
                            team.members_in(round.id())
                                .into_iter()
                                .find(|(role, _)| {
                                    matches!(role, agent_core::kinds::Role::Solo | agent_core::kinds::Role::Member)
                                })
                                .map(|(_, member)| member.task())
                        })
                })
                .ok_or_else(|| AssignmentError::UnknownMember { member: member.clone() })?;
            let branch = match team.next_part(item) {
                Some((part, _)) => format!("agents/{member}/{task}-{part}").parse(),
                None => item_branch(&member, task, item),
            }
            .map_err(|source| AssignmentError::Branch {
                item,
                task: task.clone(),
                source,
            })?;
            Ok(Assigned {
                item,
                task: task.clone(),
                agent: member,
                branch,
            })
        })
        .collect()
}

/// Records the selected holder and reason for every assigned split part.
fn log_split_assignments(team: &Team, decided: &[(ItemId, Participant)], mode: ReplacementMode) {
    for (item, agent) in decided {
        if let Some((part, _)) = team.next_part(*item) {
            let required = required_tier(team, *item);
            let previous = team
                .rounds()
                .iter()
                .rev()
                .find(|round| round.item() == Some(*item))
                .and_then(|round| {
                    team.members_in(round.id())
                        .into_iter()
                        .find(|(role, _)| {
                            matches!(role, agent_core::kinds::Role::Solo | agent_core::kinds::Role::Member)
                        })
                        .map(|(_, member)| member)
                });
            let choice = if previous.is_some_and(|member| member.agent() == agent) {
                agent_core::decide::part::PartChoice::PreviousMember(agent)
            } else if team
                .crew()
                .coordinator()
                .is_some_and(|c| c.agent().name() == agent.name())
            {
                agent_core::decide::part::PartChoice::Lead(agent)
            } else {
                agent_core::decide::part::PartChoice::Replacement(agent)
            };
            let previous = previous.map(|member| agent_core::decide::part::Candidate {
                agent: member.agent(),
                status: member.status(),
                handed_over: member.owes_lead(),
                busy: false,
            });
            let reason = choice.explanation(previous.as_ref());
            agent_runtime::history::log_decision(
                "who_gets_part",
                &serde_json::json!({
                    "team": team.id().as_str(),
                    "item": item.to_string(),
                    "part": part.to_string(),
                    "required": format!("{required:?}"),
                    "mode": format!("{mode:?}"),
                }),
                &serde_json::json!({
                    "agent": agent.to_string(),
                    "choice": choice.choice_name(),
                    "reason": reason,
                }),
            );
        }
    }
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

/// Records that `assigned` works on its item.
///
/// # Errors
/// [`AdvanceError`] when the item is not pending.
pub(in crate::daemon) fn start_item(item: &mut ItemMut<'_>, assigned: &Assigned) -> Result<(), AdvanceError> {
    item.advance(ItemPhase::InProgress {
        assignee: assigned.agent.clone(),
        branch: assigned.branch.clone(),
    })
}

#[cfg(test)]
mod tests;
