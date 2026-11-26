//! The choice of who takes over a task after its agent was refused by its provider (rate limit, overload or quota).
//!
//! [`pick`] is a pure function: given the task's role, its team, the refused agent's provider, the candidates from the
//! configuration and the facts of who is busy and refused, it returns the agent that takes the task over, or that none
//! is free with the earliest return time of a refused agent. The daemon carries the answer out.

use crate::ids::AgentName;
use crate::kinds::{Role, Tier};
use crate::state::Timestamp;

/// An agent the pick considers as a replacement for a refused task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate<'a> {
    /// Its name.
    pub name: &'a AgentName,
    /// Its provider, as [`super::super::pool::provider_of`] would compute it.
    pub provider: &'a str,
    /// The tier of the configuration it is in.
    pub tier: Tier,
    /// Whether it is at its run limit now and cannot take a task.
    pub busy: bool,
    /// Until when its provider refuses it; `None` when it is not refused.
    pub refused_until: Option<Timestamp>,
}

/// The result of picking a replacement for a refused task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick<'a> {
    /// The agent that takes the task over.
    Agent(&'a AgentName),
    /// No agent is free now: every candidate is in the team, busy, refused or of the same provider.
    /// `earliest_return` is the earliest time a refused candidate returns, so the caller knows when to try again;
    /// `None` when no candidate is refused at all.
    NoneFree {
        /// The earliest time a refused candidate returns; `None` when none is refused.
        earliest_return: Option<Timestamp>,
    },
}

/// The facts a [`pick`] needs.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    /// The task's role.
    pub role: Role,
    /// The agents in the task's team.
    pub team: &'a [AgentName],
    /// The provider that refused the task's agent.
    pub refused_provider: &'a str,
    /// The agents of `[replacement] agents`, in the configured order.
    pub replacement_agents: &'a [Candidate<'a>],
    /// The agents of the tier `normal`, in the configured order.
    pub normal_agents: &'a [Candidate<'a>],
    /// The investigators, in name order.
    pub investigators: &'a [Candidate<'a>],
    /// The current time, for checking which agents are still refused.
    pub now: Timestamp,
    /// Whether `[replacement] replacements` is used up: no more takeovers are left.
    pub replacements_exhausted: bool,
}

/// Picks the agent that takes over a task whose agent was refused by its provider.
///
/// The pick walks the candidates in two groups:
/// 1. `replacement_agents` — the agents of `[replacement] agents`, in the configured order.
/// 2. The fallback: `normal_agents` for most roles; `investigators` for a coordinator (the team's lead).
///
/// A candidate is skipped when it:
/// - is in `team`,
/// - is refused at `now` (its [`Candidate::refused_until`] is later),
/// - has the same provider as `refused_provider`,
/// - is busy,
/// - is a worker for a member's task,
/// - is not an investigator for a coordinator's task.
///
/// When `replacements_exhausted` is true, no more takeovers are left: the pick returns [`Pick::NoneFree`] at once.
///
/// [`Pick::NoneFree`] carries the earliest return time of every refused candidate, so the caller knows when to try
/// again; `None` when no candidate is refused at all.
///
/// # Panics
/// Never.
#[must_use]
pub fn pick<'a>(request: &Request<'a>) -> Pick<'a> {
    let all_candidates: &[&[Candidate<'_>]] =
        &[request.replacement_agents, request.normal_agents, request.investigators];

    if request.replacements_exhausted {
        return Pick::NoneFree {
            earliest_return: earliest_refused_return(all_candidates, request.now),
        };
    }

    // First group: [replacement] agents.
    if let Some(found) = find_fitting(
        request.role,
        request.team,
        request.refused_provider,
        request.replacement_agents,
        request.now,
    ) {
        return Pick::Agent(found);
    }

    // Second group: the fallback.
    let fallback = fallback_candidates(request.role, request.normal_agents, request.investigators);
    if let Some(found) = find_fitting(
        request.role,
        request.team,
        request.refused_provider,
        fallback,
        request.now,
    ) {
        return Pick::Agent(found);
    }

    Pick::NoneFree {
        earliest_return: earliest_refused_return(all_candidates, request.now),
    }
}

/// The first candidate of `candidates` that fits `role` and is not skipped; `None` when none fits.
fn find_fitting<'a>(
    role: Role,
    team: &[AgentName],
    refused_provider: &str,
    candidates: &'a [Candidate<'a>],
    now: Timestamp,
) -> Option<&'a AgentName> {
    candidates.iter().find_map(|candidate| {
        if fits(role, team, refused_provider, candidate, now) {
            Some(candidate.name)
        } else {
            None
        }
    })
}

/// Whether `candidate` may take the task over: not in the team, not refused, not of the same provider, not busy,
/// and the right tier for `role`.
fn fits(role: Role, team: &[AgentName], refused_provider: &str, candidate: &Candidate<'_>, now: Timestamp) -> bool {
    if team.contains(candidate.name) {
        return false;
    }
    if candidate.refused_until.is_some_and(|until| now < until) {
        return false;
    }
    if candidate.provider == refused_provider {
        return false;
    }
    if candidate.busy {
        return false;
    }
    match role {
        Role::Coordinator => candidate.tier == Tier::Investigator,
        Role::Member => candidate.tier != Tier::Worker,
        Role::Solo | Role::Worker | Role::Reviewer | Role::Investigator => true,
    }
}

/// The fallback candidates for `role`: `investigators` for a coordinator, `normal_agents` for the others.
fn fallback_candidates<'a>(
    role: Role,
    normal_agents: &'a [Candidate<'a>],
    investigators: &'a [Candidate<'a>],
) -> &'a [Candidate<'a>] {
    match role {
        Role::Coordinator => investigators,
        Role::Solo | Role::Member | Role::Worker | Role::Reviewer | Role::Investigator => normal_agents,
    }
}

/// The earliest return time of every refused candidate across all groups; `None` when none is refused.
fn earliest_refused_return(all: &[&[Candidate<'_>]], now: Timestamp) -> Option<Timestamp> {
    all.iter()
        .flat_map(|group| group.iter())
        .filter_map(|candidate| candidate.refused_until)
        .filter(|until| now < *until)
        .min()
}

#[cfg(test)]
mod tests;
