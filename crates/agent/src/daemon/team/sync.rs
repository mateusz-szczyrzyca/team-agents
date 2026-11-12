//! The state each member should have on the bus, and what differs from the state it has.

use agent_core::ids::{AgentName, InvalidName, TaskId};
use agent_core::kinds::Role;
use agent_core::state::{Member, RunEnd, RunState, Team};
use agent_core::status::{RunOutcome, TaskStatus};
use agent_runtime::agentbus::{BusTeam, MemberState};

use super::assign::agent_name;
use super::close::is_held;

/// What to change on the bus so that it agrees with the tasks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::daemon) struct Reconciliation {
    /// The members whose state on the bus is to be set, with the state.
    pub(in crate::daemon) set: Vec<(AgentName, MemberState)>,
    /// The tasks of members the bus has woken: they start now.
    pub(in crate::daemon) wake: Vec<TaskId>,
}

/// The state on the bus that matches the status of `member`, of role `role` in `team`.
pub(in crate::daemon) fn desired_bus_state(team: &Team, role: Role, member: &Member) -> MemberState {
    match member.status() {
        TaskStatus::Queued if is_held(team, role, member) => MemberState::Sleeping,
        TaskStatus::Queued | TaskStatus::Running => MemberState::Active,
        TaskStatus::Waiting => MemberState::Sleeping,
        // A member that waits for the human, or for its provider, is not woken by a message: it could not act on it.
        TaskStatus::Questions | TaskStatus::Paused => MemberState::Paused,
        TaskStatus::Stopped => MemberState::Stopped,
        TaskStatus::Failed => MemberState::Done,
        TaskStatus::Done if was_excluded(member) => MemberState::Excluded,
        TaskStatus::Done => MemberState::Done,
    }
}

/// Whether the member's last run ended by acknowledging that the coordinator excluded it.
fn was_excluded(member: &Member) -> bool {
    member.runs().last().is_some_and(|run| {
        matches!(
            run.state(),
            RunState::Ended {
                end: RunEnd::Reported {
                    outcome: RunOutcome::Excluded
                },
                ..
            }
        )
    })
}

/// What differs between `team` and `bus`: the states to set and the members to wake.
///
/// # Errors
/// [`InvalidName`] for a member whose name is not an agent name.
pub(in crate::daemon) fn reconcile(team: &Team, bus: &BusTeam) -> Result<Reconciliation, InvalidName> {
    let mut plan = Reconciliation::default();
    for (role, member) in team.crew().roster() {
        let name = agent_name(member.agent())?;
        let Some(on_bus) = bus.members.get(&name) else {
            continue;
        };
        // Exclusion is final on the bus; whatever the task does, it stays.
        if on_bus.state == MemberState::Excluded {
            continue;
        }
        let mut wanted = desired_bus_state(team, role, member);
        let asleep = member.status() == TaskStatus::Waiting || is_held(team, role, member);
        if on_bus.state == MemberState::Waking && asleep {
            plan.wake.push(member.task().clone());
            wanted = MemberState::Active;
        }
        if on_bus.state != wanted {
            plan.set.push((name, wanted));
        }
    }
    Ok(plan)
}

#[cfg(test)]
mod tests;
