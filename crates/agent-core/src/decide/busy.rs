//! What a queued task whose agent is busy in another task does once it has waited for its agent long enough: it keeps
//! waiting, or a free agent of `[replacement] agents` takes it over. The offers are the agents of that list in its
//! order; the one whose CLI carries the fewest runs goes, and among equals the first of the list.
//!
//! [`decide`] is the whole decision. It touches nothing: the daemon asks who is free, records the hand-over and tells
//! the team.

use time::Duration;

use crate::ids::AgentName;
use crate::kinds::{Role, Tier};

#[cfg(test)]
mod tests;

/// Whether an agent of `[replacement] agents` can take a task over now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// It runs nothing and has no queued task of its own: it may take a task over.
    Free,
    /// It runs a task, or has a queued one of its own that could start now.
    Busy,
    /// A task of it is paused for a quota or a rate limit.
    Paused,
}

/// An agent of `[replacement] agents` that could take the task over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offer<'a> {
    /// The agent.
    pub agent: &'a AgentName,
    /// Its tier in the configuration.
    pub tier: Tier,
    /// Whether one of its tasks is in the team of the task that waits: it is never given that task.
    pub in_team: bool,
    /// Whether it can take a task over now.
    pub availability: Availability,
    /// How many runs go on the CLI of the agent, over all teams, at this moment.
    pub cli_runs: u32,
}

/// Why a task is waiting or active: only a queued task waiting for its busy agent ([`TaskWait::BusyAgent`]) is handed
/// over after `busy_after`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TaskWait {
    /// The task is queued and waits for its busy agent.
    #[default]
    BusyAgent,
    /// The task is currently running.
    Running,
    /// The task waits for an answer of the human.
    HumanAnswer,
    /// The task's item waits for a review.
    Review,
}

/// What the daemon knows about a queued task that waits for its agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Waiting<'a> {
    /// The role of the task.
    pub role: Role,
    /// What the task is currently doing or waiting for.
    pub wait: TaskWait,
    /// Whether a member of the task's team runs now; a coordinator is not taken from its team while one does.
    pub member_runs: bool,
    /// The tier of the agent that holds the task.
    pub held: Tier,
    /// How long the task has waited for its agent.
    pub waited: Duration,
    /// `[replacement] busy_after` in seconds; `0` never hands a task over.
    pub busy_after: u32,
    /// The agents of `[replacement] agents` that could take the task over, in the order of the configuration.
    pub offers: &'a [Offer<'a>],
}

/// What happens to a waiting task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision<'a> {
    /// The task keeps waiting for its agent.
    Keep,
    /// This agent takes the task over, in the same worktree and on the same branch.
    HandOver(&'a AgentName),
}

/// The place of a tier as a number, as `decide::replacement::rank` gives it.
const fn rank(tier: Tier) -> u8 {
    crate::decide::replacement::rank(tier)
}

/// Whether the task of `waiting` may be handed over at all: a worker's and a reviewer's are not, nor a coordinator's while
/// one of its members runs.
const fn may_be_handed_over(waiting: &Waiting<'_>) -> bool {
    match waiting.role {
        Role::Worker | Role::Reviewer => false,
        Role::Coordinator => !waiting.member_runs,
        Role::Solo | Role::Member => true,
    }
}

/// The waiting task's decision: [`Decision::HandOver`] to the free agent of the same or a higher place than the holder,
/// outside its team, whose CLI carries the fewest runs (the first of the list among equals), once the task has waited at
/// least `busy_after` seconds; otherwise [`Decision::Keep`].
#[must_use]
pub fn decide<'a>(waiting: &Waiting<'a>) -> Decision<'a> {
    if !may_be_handed_over(waiting) || waiting.busy_after == 0 {
        return Decision::Keep;
    }
    if waiting.waited < Duration::seconds(i64::from(waiting.busy_after)) {
        return Decision::Keep;
    }
    // `min_by_key` returns the first of several equal minima: the first of the list among the least loaded CLIs.
    waiting
        .offers
        .iter()
        .filter(|offer| {
            !offer.in_team && offer.availability == Availability::Free && rank(offer.tier) >= rank(waiting.held)
        })
        .min_by_key(|offer| offer.cli_runs)
        .map_or(Decision::Keep, |offer| Decision::HandOver(offer.agent))
}
