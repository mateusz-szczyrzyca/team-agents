//! The line of a task under its team, and which tasks the short view lists: every task of the team — its coordinator,
//! members, workers and reviewers — whatever its status.

use agent_core::ids::TaskId;
use agent_core::kinds::Role;
use agent_core::local_time::Zone;
use agent_core::protocol::{MemberSummary, TeamSummary};
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
    /// Widths of `role` and `agent` characters, which the tests of the lines pad to.
    #[cfg(test)]
    pub(super) const fn new(role: usize, agent: usize) -> Self {
        Self { role, agent }
    }

    /// These widths, widened to fit a role of `role` and an agent of `agent` characters.
    pub(super) fn fit(self, role: usize, agent: usize) -> Self {
        Self {
            role: self.role.max(role),
            agent: self.agent.max(agent),
        }
    }

    /// The width of the column of the roles.
    pub(super) const fn role(self) -> usize {
        self.role
    }

    /// The width of the column of the agents.
    pub(super) const fn agent(self) -> usize {
        self.agent
    }

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
/// what the record of the team says of its review —, then that it runs in a new session, why it failed when the reason
/// is named, whom it was taken over from, what `activity` says of it while it runs, and the questions that wait for
/// answers.
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
    let failed = member
        .failed_reason
        .as_ref()
        .map_or_else(String::new, |reason| format!(", {reason}"));
    let returned = super::returned::task_note(member);
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
    // A queued task whose agent holds its limit of runs says which run it waits for, and in which team.
    let blocked = member.waits_for.as_ref().map_or_else(String::new, |wait| {
        format!(", waits for {}, running in {}", member.agent.name(), wait.team)
    });
    let (role_width, agent_width) = (columns.role, columns.agent);
    format!(
        "  {}  {:<role_width$}  {:<agent_width$}  {status}{failed}{blocked}{renewed}{taken_over}{returned}{active}{waiting}",
        member.task,
        member.role.to_string(),
        member.agent.name(),
    )
}

/// Whether the short view lists the task of `member`.
pub(super) const fn listed(member: &MemberSummary) -> bool {
    alive(member.role)
}

/// Whether the short view lists a task of `role`.
///
/// Every task of a team is listed, whatever its status and, for a reviewer, whatever its review's outcome: the human
/// wants to see who is in the team and who reviewed, also once the work is done, stopped or failed.
pub(super) const fn alive(role: Role) -> bool {
    match role {
        Role::Solo | Role::Coordinator | Role::Member | Role::Worker | Role::Reviewer => true,
    }
}

#[cfg(test)]
mod tests;
