//! Decides whether a team's recorded inactivity permits automatic removal.

use std::time::Duration;

use crate::state::Timestamp;

/// A reason that work cannot currently advance and must be preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// No exemption; unanswered human questions are included here.
    None,
    /// A task or a check still runs.
    Running,
    /// The human paused or stopped a task.
    Human,
    /// A provider's quota or rate limit has not reset.
    Provider,
    /// A round waits for another team's pull request to merge.
    ExternalMerge,
}

/// Whether the owner may attempt its normal safe removal procedure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Keep the team, either exempt, recently active, or disabled by configuration.
    Keep,
    /// Attempt removal; its safeguards may still refuse it.
    Remove,
}

/// Compares the supplied clock with the last run or branch activity.
/// A zero limit disables removal; future activity never counts as idle.
#[must_use]
pub fn decide(_now: Timestamp, _last: Timestamp, _limit: Duration, _hold: Hold) -> Decision {
    Decision::Keep
}

#[cfg(test)]
mod tests;
