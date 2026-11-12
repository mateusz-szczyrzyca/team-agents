//! The state each member should have on the bus, and what differs from the state it has.

use agent_core::ids::{AgentName, InvalidName, TaskId};
use agent_core::kinds::Role;
use agent_core::state::{Member, Team};
use agent_runtime::agentbus::{BusTeam, MemberState};

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
    let _ = (team, role, member);
    MemberState::Active
}

/// What differs between `team` and `bus`: the states to set and the members to wake.
///
/// # Errors
/// [`InvalidName`] for a member whose name is not an agent name.
pub(in crate::daemon) fn reconcile(team: &Team, bus: &BusTeam) -> Result<Reconciliation, InvalidName> {
    let _ = (team, bus);
    Ok(Reconciliation::default())
}

#[cfg(test)]
mod tests;
