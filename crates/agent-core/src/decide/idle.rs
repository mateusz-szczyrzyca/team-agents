//! Whether a team has done no work for the time `[limits] idle_team_minutes` gives, so that the daemon may remove it
//! as by `agent remove <team>`.
//!
//! The decision is a pure function of what a look at a team found: the moments of its last activity, its running runs,
//! its pauses and the waits that are not its own work. `idle_team_minutes = 0` turns it off.

#[cfg(test)]
mod tests;

use std::time::Duration;

use crate::state::Timestamp;

use super::idle_look::{Holds, Look, Why};

/// What the daemon does about a team that has done no work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Nothing: the team has not been idle for the whole bound, or is held by something that ends by itself.
    Keep(Why),
    /// Remove the team as by `agent remove <team>`.
    Remove,
}

/// The seconds between `then` and `now`; a moment in the future counts as no time at all.
#[must_use]
pub fn since(then: Timestamp, now: Timestamp) -> Duration {
    let elapsed = now.as_utc() - then.as_utc();
    if elapsed.is_negative() {
        return Duration::ZERO;
    }
    elapsed.unsigned_abs()
}

/// Whether a team has been idle for the whole bound.
///
/// A run, a quota wait, the human's pause or another team's merge keeps the team: those are waits that end by
/// themselves, or that the human decides. A question to the human is not among them — an agent that asks and sleeps
/// leaves nobody to answer, which is the stall this bound exists for.
///
/// `bound` is `[limits] idle_team_minutes`; zero never makes a team idle, and the caller does not ask then. A look
/// whose last activity is ahead of the look — a clock that went back — is no idle time at all until it passes.
#[must_use]
pub fn decide(look: &Look, bound: Duration) -> Decision {
    if bound.is_zero() {
        return Decision::Keep(Why::RecentlyActive);
    }
    match look.holds {
        Holds::Running => return Decision::Keep(Why::Running),
        Holds::QuotaWait => return Decision::Keep(Why::QuotaWait),
        Holds::HumanHold => return Decision::Keep(Why::HumanHold),
        Holds::Nothing => {}
    }
    if look.waits_for_another_team {
        return Decision::Keep(Why::AnotherTeam);
    }
    if since(look.last, look.at) < bound {
        return Decision::Keep(Why::RecentlyActive);
    }
    Decision::Remove
}
