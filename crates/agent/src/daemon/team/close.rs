//! When a team is finished, and which of its tasks wait to be ended with it.

use agent_core::ids::TaskId;
use agent_core::kinds::Role;
use agent_core::state::{Member, Team};
use agent_core::status::TaskStatus;

/// Whether `member`, of role `role` in `team`, is queued but has never run and has nothing to do: it starts when it is
/// given an item, woken by the bus or asked to serve a request, not before.
pub(in crate::daemon) fn is_held(team: &Team, role: Role, member: &Member) -> bool {
    let _ = (team, role, member);
    false
}

/// Whether the work of `team` is over: its items are done, or for a list without items its lead said `DONE`, and no
/// task is going, asking, paused or about to start.
pub(in crate::daemon) fn is_finished(team: &Team) -> bool {
    let _ = team;
    false
}

/// The tasks of a finished `team` that are still open, with the status that ends each: waiting ones are done, held ones
/// never start.
pub(in crate::daemon) fn closing_transitions(team: &Team) -> Vec<(TaskId, TaskStatus)> {
    let _ = team;
    Vec::new()
}

#[cfg(test)]
mod tests;
