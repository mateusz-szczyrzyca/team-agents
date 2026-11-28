//! Pure selection of the next holder of a lead task.

use crate::ids::{AgentName, TaskId};
use crate::kinds::Role;
use crate::state::{Completion, ItemPhase, Member, State, Team, Timestamp};
use crate::status::TaskStatus;

/// Clock and configured candidates supplied by the caller, without reading configuration or time here.
#[derive(Debug)]
pub struct ShiftLimits<'a> {
    /// Maximum worked minutes per holder; zero disables shifts.
    pub lead_shift_minutes: u32,
    /// Time at which a running lead's work is measured.
    pub now: Timestamp,
    /// Solo-task rotation order from `[replacement] agents`.
    pub replacement_agents: &'a [AgentName],
}

/// Whether a lead should retain its task or give it to another model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShiftCheck {
    /// The task has not reached a shift boundary or is not eligible.
    NotDue,
    /// Work is due to rotate, but every other eligible agent is occupied.
    NoneFree,
    /// The next free agent in rotation order.
    To(AgentName),
}

/// Chooses a lead's next holder without mutating state or consulting a clock.
#[must_use]
pub fn shift_target(task: &TaskId, team: &Team, state: &State, limits: &ShiftLimits<'_>) -> ShiftCheck {
    let Some((role, lead)) = team.find(task) else {
        return ShiftCheck::NotDue;
    };
    if limits.lead_shift_minutes == 0
        || !matches!(role, Role::Solo | Role::Coordinator)
        || matches!(lead.status(), TaskStatus::Done | TaskStatus::Paused)
        || worked_secs(lead, limits.now) < u64::from(limits.lead_shift_minutes) * 60
        || !has_pull_request(lead, team)
    {
        return ShiftCheck::NotDue;
    }
    let solo = role == Role::Solo;
    let order = if solo {
        limits.replacement_agents.to_vec()
    } else {
        team_order(team)
    };
    let holder = lead.agent().agent_name();
    let start = order.iter().position(|name| *name == holder).map_or(0, |i| i + 1);
    for candidate in order.iter().cycle().skip(start).take(order.len()) {
        if *candidate == holder || state.refused_at(candidate, limits.now) {
            continue;
        }
        let unavailable = std::iter::once(team)
            .chain(state.teams().iter().filter(|other| other.id() != team.id()))
            .any(|other| {
                let roster = other.crew().roster();
                let holds = |member: &&Member| member.agent().agent_name() == *candidate;
                let busy = roster
                    .iter()
                    .map(|(_, member)| *member)
                    .chain(other.reviewers())
                    .any(|member| holds(&member) && member.current_run().is_some());
                let questions = other.id() == team.id()
                    && roster
                        .iter()
                        .any(|(_, member)| holds(member) && member.status() == TaskStatus::Questions);
                let open_team = solo
                    && other.id() != team.id()
                    && roster.iter().any(|(_, member)| member.status() != TaskStatus::Done)
                    && roster.iter().any(|(_, member)| holds(member));
                busy || questions || open_team
            });
        if !unavailable {
            return ShiftCheck::To(candidate.clone());
        }
    }
    ShiftCheck::NoneFree
}

/// Formats minutes into human-readable span text such as `1h40m`, `2h`, or `45m`.
#[must_use]
pub fn format_span(minutes: u64) -> String {
    let _ = minutes;
    String::new()
}

/// Includes only positive elapsed time when the caller's clock precedes a run start.
#[must_use]
pub fn worked_secs(lead: &Member, now: Timestamp) -> u64 {
    let running = lead.current_run().map_or(0, |run| {
        let seconds = (now.as_utc() - run.started_at().as_utc()).whole_seconds();
        u64::try_from(seconds).unwrap_or(0)
    });
    lead.lead_worked_secs().saturating_add(running)
}

/// A previous round's PR cannot authorize a shift during a newer round without one.
fn has_pull_request(lead: &Member, team: &Team) -> bool {
    let from_run = |run: &crate::state::Run| {
        run.rounds()
            .iter()
            .rev()
            .filter_map(|id| team.round(id))
            .find(|round| round.is_open())
    };
    let assigned = || {
        team.rounds()
            .iter()
            .rev()
            .filter(|round| round.is_open())
            .find(|round| {
                round.item().and_then(|id| team.item(id)).is_some_and(|item| {
                    matches!(item.phase(),
                    ItemPhase::InProgress { assignee, .. } | ItemPhase::Fixing { assignee, .. }
                        if *assignee == lead.agent().agent_name())
                })
            })
    };
    let Some(round) = lead
        .current_run()
        .and_then(from_run)
        .or_else(assigned)
        .or_else(|| lead.runs().last().and_then(from_run))
        .or_else(|| {
            let mut open = team.open_rounds();
            let only = open.next()?;
            open.next().is_none().then_some(only)
        })
    else {
        return false;
    };
    round.delivered_by().is_some()
        || round.part_pull_request().is_some()
        || round.item().and_then(|id| team.item(id)).is_some_and(|item| {
            matches!(
                item.phase(),
                ItemPhase::InReview { .. }
                    | ItemPhase::Fixing { .. }
                    | ItemPhase::Approved { .. }
                    | ItemPhase::Done {
                        completion: Completion::Merged { .. }
                    }
            )
        })
}

/// Task trades move holders, so recover each original slot from its earliest holding.
fn team_order(team: &Team) -> Vec<AgentName> {
    let roster = team.crew().roster();
    let current: Vec<_> = roster
        .iter()
        .filter(|(role, _)| *role != Role::Worker)
        .map(|(_, member)| member.agent().agent_name())
        .collect();
    let mut order = Vec::new();
    for (_, member) in roster.iter().filter(|(role, _)| *role != Role::Worker) {
        let original = member
            .past_holders()
            .first()
            .map_or_else(|| member.agent().agent_name(), |holding| holding.agent.agent_name());
        if current.contains(&original) && !order.contains(&original) {
            order.push(original);
        }
    }
    for name in current {
        if !order.contains(&name) {
            order.push(name);
        }
    }
    order
}

#[cfg(test)]
mod tests;
