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
pub fn pick<'a>(
    _role: Role,
    _team: &[AgentName],
    _refused_provider: &str,
    _replacement_agents: &'a [Candidate<'a>],
    _normal_agents: &'a [Candidate<'a>],
    _investigators: &'a [Candidate<'a>],
    _now: Timestamp,
    _replacements_exhausted: bool,
) -> Pick<'a> {
    // Stub: always says none free with no return time.
    Pick::NoneFree {
        earliest_return: None,
    }
}

#[cfg(test)]
mod tests;
