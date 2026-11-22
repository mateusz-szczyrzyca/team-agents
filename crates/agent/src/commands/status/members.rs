//! The line of a task under its team, and which tasks the short view lists: the team's coordinator, members and workers
//! always, a reviewer only while its review is not over.

use agent_core::ids::TaskId;
use agent_core::kinds::Role;
use agent_core::local_time::Zone;
use agent_core::protocol::{MemberSummary, TeamSummary};
use agent_core::state::{Activity, ActivityEnd, ActivityOutcome};
use agent_core::status::TaskStatus;

use super::reviewer;

/// The widths the lines of the tasks pad their columns to, so that the statuses of a list start in one column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Columns {
    /// The width of the longest role of the list.
    role: usize,
    /// The width of the longest agent name of the list.
    agent: usize,
}

impl Columns {
    /// The widths that fit every task of `teams`, listed or not.
    pub(super) fn of(teams: &[TeamSummary]) -> Self {
        let members = || teams.iter().flat_map(|team| team.members.iter());
        Self {
            role: members().map(|member| member.role.to_string().len()).max().unwrap_or(0),
            agent: members().map(|member| member.agent.name().len()).max().unwrap_or(0),
        }
    }
}

/// The line of the task of `member` under its `team`: its ID, its role, its agent and what it does — for a reviewer
/// what the record of the team says of its review —, then that it runs in a new session, whom it was taken over from,
/// what `activity` says of it while it runs, and the questions that wait for answers.
pub(super) fn line(
    member: &MemberSummary,
    team: &TeamSummary,
    columns: Columns,
    activity: &dyn Fn(&TaskId) -> Option<String>,
    zone: Zone,
) -> String {
    let waiting = member.open_question.map_or_else(String::new, |number| {
        format!(", questions number {number} wait for answers")
    });
    let renewed = member
        .new_sessions
        .map_or_else(String::new, |_| ", in a new session".to_owned());
    let taken_over = member
        .held_by
        .as_ref()
        .map_or_else(String::new, |held| format!(", for {held}"));
    // Only a task that runs is active now: what a task that waits last did is in its log.
    let active = (member.status == TaskStatus::Running)
        .then(|| activity(&member.task))
        .flatten()
        .map_or_else(String::new, |said| format!(", {said}"));
    let status = match member.role {
        Role::Reviewer => reviewer::status(member, &team.activities, zone).unwrap_or_else(|| member.status.to_string()),
        Role::Solo | Role::Coordinator | Role::Member | Role::Worker => member.status.to_string(),
    };
    let (role_width, agent_width) = (columns.role, columns.agent);
    format!(
        "  {}  {:<role_width$}  {:<agent_width$}  {status}{renewed}{taken_over}{active}{waiting}",
        member.task,
        member.role.to_string(),
        member.agent.name(),
    )
}

/// Whether the short view lists the task of `member`, whose team recorded `activities`.
pub(super) fn listed(member: &MemberSummary, activities: &[Activity]) -> bool {
    alive(member.role, member.status, reviewer::outcome(member, activities))
}

/// Whether the short view lists a task of `role` in `status`; `review` is the outcome of the newest activity the team
/// recorded of it, `None` when it recorded none.
///
/// The team's own tasks — a solo task, the coordinator, a member, a worker — are always listed: the human wants to see
/// the team, also the one that stopped, failed or is done. A reviewer is listed while its review is not over: always
/// while its task runs, never once its task is done or stopped, and otherwise unless the record says the review ended
/// for good — with its report, stopped by the human, or as a second opinion that was abandoned. A review that failed
/// is still listed: its task can be resumed.
pub(super) const fn alive(role: Role, status: TaskStatus, review: Option<&ActivityOutcome>) -> bool {
    match role {
        Role::Solo | Role::Coordinator | Role::Member | Role::Worker => true,
        Role::Reviewer => match status {
            TaskStatus::Running => true,
            TaskStatus::Done | TaskStatus::Stopped => false,
            TaskStatus::Waiting
            | TaskStatus::Questions
            | TaskStatus::Queued
            | TaskStatus::Paused
            | TaskStatus::Failed => match review {
                None
                | Some(
                    ActivityOutcome::Working
                    | ActivityOutcome::Finished {
                        end: ActivityEnd::Failed { .. },
                        ..
                    },
                ) => true,
                Some(ActivityOutcome::Finished {
                    end: ActivityEnd::Reported | ActivityEnd::Stopped | ActivityEnd::OpinionFailed { .. },
                    ..
                }) => false,
            },
        },
    }
}

#[cfg(test)]
mod tests;
