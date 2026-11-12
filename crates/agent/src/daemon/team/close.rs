//! When a team is finished, and which of its tasks wait to be ended with it.

use agent_core::ids::TaskId;
use agent_core::kinds::Role;
use agent_core::state::{ItemPhase, Member, RunEnd, RunState, Team};
use agent_core::status::{RunOutcome, TaskStatus};

/// Whether `member`, of role `role` in `team`, is queued but has never run and has nothing to do: it starts when it is
/// given an item, woken by the bus or asked to serve a request, not before.
pub(in crate::daemon) fn is_held(team: &Team, role: Role, member: &Member) -> bool {
    member.status() == TaskStatus::Queued
        && member.runs().is_empty()
        && matches!(role, Role::Member | Role::Worker)
        && !has_item(team, member)
}

/// Whether `member` is the assignee of an item that is in progress or being fixed.
fn has_item(team: &Team, member: &Member) -> bool {
    team.items().iter().any(|progress| match progress.phase() {
        ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. } => {
            assignee.as_str() == member.agent().name()
        }
        ItemPhase::Pending | ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. } => false,
    })
}

/// Whether the work of `team` is over: its items are done, or for a list without items its lead said `DONE`, and no
/// task is going, asking, paused or about to start.
pub(in crate::daemon) fn is_finished(team: &Team) -> bool {
    let items_done = team
        .items()
        .iter()
        .all(|progress| matches!(progress.phase(), ItemPhase::Done { .. }));
    // A list without items is one piece of work for the lead: it is over when the lead says so.
    let lead_done = !team.items().is_empty() || lead_said_done(team);
    items_done
        && lead_done
        && team
            .crew()
            .roster()
            .into_iter()
            .all(|(role, member)| at_rest(team, role, member))
}

/// Whether the lead's last run ended with `DONE`.
fn lead_said_done(team: &Team) -> bool {
    let roster = team.crew().roster();
    let Some((_, lead)) = roster.first() else {
        return false;
    };
    lead.runs().last().is_some_and(|run| {
        matches!(
            run.state(),
            RunState::Ended {
                end: RunEnd::Reported {
                    outcome: RunOutcome::Done
                },
                ..
            }
        )
    })
}

/// Whether `member` will do nothing more without being asked to: it waits, is over, or has not started and is held.
fn at_rest(team: &Team, role: Role, member: &Member) -> bool {
    match member.status() {
        TaskStatus::Waiting | TaskStatus::Done | TaskStatus::Stopped | TaskStatus::Failed => true,
        TaskStatus::Queued => is_held(team, role, member),
        TaskStatus::Running | TaskStatus::Questions | TaskStatus::Paused => false,
    }
}

/// The tasks of a finished `team` that are still open, with the status that ends each: waiting ones are done, held ones
/// never start.
pub(in crate::daemon) fn closing_transitions(team: &Team) -> Vec<(TaskId, TaskStatus)> {
    if !is_finished(team) {
        return Vec::new();
    }
    team.crew()
        .roster()
        .into_iter()
        .filter_map(|(role, member)| match member.status() {
            TaskStatus::Waiting => Some((member.task().clone(), TaskStatus::Done)),
            TaskStatus::Queued if is_held(team, role, member) => Some((member.task().clone(), TaskStatus::Stopped)),
            TaskStatus::Queued
            | TaskStatus::Running
            | TaskStatus::Questions
            | TaskStatus::Paused
            | TaskStatus::Stopped
            | TaskStatus::Failed
            | TaskStatus::Done => None,
        })
        .collect()
}

#[cfg(test)]
mod tests;
