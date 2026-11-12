//! When a team is finished, and which of its tasks wait to be ended with it.

use agent_core::ids::TaskId;
use agent_core::state::{ItemPhase, Member, RunEnd, RunState, Team};
use agent_core::status::{RunOutcome, TaskStatus};

/// Whether the work of `team` is over: its items are done, or for a list without items its lead said `DONE`, and no
/// task is going, asking, paused or about to start.
pub(in crate::daemon) fn is_finished(team: &Team) -> bool {
    let items_done = team
        .items()
        .iter()
        .all(|progress| matches!(progress.phase(), ItemPhase::Done { .. }));
    // A list without items is one piece of work for the lead: it is over when the lead says so.
    let lead_done = !team.items().is_empty() || lead_said_done(team);
    items_done && lead_done && team.crew().roster().into_iter().all(|(_, member)| at_rest(member))
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

/// Whether `member` will do nothing more without being asked to: it waits, or is over.
fn at_rest(member: &Member) -> bool {
    match member.status() {
        TaskStatus::Waiting | TaskStatus::Done | TaskStatus::Stopped | TaskStatus::Failed => true,
        TaskStatus::Queued | TaskStatus::Running | TaskStatus::Questions | TaskStatus::Paused => false,
    }
}

/// Whether `team` has an item that has started and is not done: its work goes on until that item is merged, so a task
/// that took the item must wait for it instead of ending.
pub(in crate::daemon) fn item_in_flight(team: &Team) -> bool {
    team.items().iter().any(|progress| {
        matches!(
            progress.phase(),
            ItemPhase::InProgress { .. }
                | ItemPhase::InReview { .. }
                | ItemPhase::Fixing { .. }
                | ItemPhase::Approved { .. }
        )
    })
}

/// The tasks of a finished `team` that are still open, with the status that ends each: waiting ones are done.
pub(in crate::daemon) fn closing_transitions(team: &Team) -> Vec<(TaskId, TaskStatus)> {
    if !is_finished(team) {
        return Vec::new();
    }
    team.crew()
        .roster()
        .into_iter()
        .filter_map(|(_, member)| match member.status() {
            TaskStatus::Waiting => Some((member.task().clone(), TaskStatus::Done)),
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
