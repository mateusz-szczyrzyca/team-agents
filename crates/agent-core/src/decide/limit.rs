//! What a provider's refusal of a run does to the agent that met it: the pause and a retry after it, or a change of
//! agent. A refusal is a rate limit, an overload or an exhausted quota; the retries after the pause are counted in a
//! row, and a run that ends in any other way clears the count.
//!
//! [`decide`] is the whole rule and touches nothing: the daemon feeds it what the run showed and carries out the answer.

use time::Duration;

use crate::state::RunEnd;

#[cfg(test)]
mod tests;

/// What the provider's refusal showed about its limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// A rate limit or an overload: the provider names no reset.
    RateLimit,
    /// An exhausted quota. `reset_in` is how far away its named reset is; `None` when the CLI names none.
    Quota {
        /// The time until the reset the provider named, strictly positive when present.
        reset_in: Option<Duration>,
    },
}

/// The facts of one refused run, as the daemon has them after the run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refusal {
    /// What the refusal showed.
    pub limit: Limit,
    /// How many refusals in a row the task has had, this one included; `1` for the first.
    pub refusals: u32,
    /// `[replacement] limit_retries`: how many retries after the pause the task gets.
    pub retries: u32,
    /// `[replacement] limit_wait_minutes` as a duration: the longest wait for a named reset; zero never waits for one.
    pub wait_limit: Duration,
}

/// What the rule says to do with a refused task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The task waits for the pause and is tried again in the same session.
    Pause,
    /// The task changes to an agent of another provider: the next refusal is not waited out.
    ChangeAgent,
}

/// The count of consecutive provider refusals after this run; other endings clear it.
#[must_use]
pub const fn next_refusals(previous: u32, end: &RunEnd) -> u32 {
    match end {
        RunEnd::RateLimited | RunEnd::QuotaExhausted { .. } => previous.saturating_add(1),
        RunEnd::Reported { .. } | RunEnd::NoStatus { .. } | RunEnd::Hung | RunEnd::TurnLimit | RunEnd::Stopped => 0,
    }
}

/// What the rule says about a refused task: the change of agent when its retries are used up, or when the provider's
/// named reset is further away than the configured wait; the pause otherwise.
#[must_use]
pub fn decide(refusal: &Refusal) -> Decision {
    if let Limit::Quota {
        reset_in: Some(reset_in),
    } = refusal.limit
        && reset_in > refusal.wait_limit
    {
        return Decision::ChangeAgent;
    }
    if refusal.refusals > refusal.retries {
        Decision::ChangeAgent
    } else {
        Decision::Pause
    }
}
