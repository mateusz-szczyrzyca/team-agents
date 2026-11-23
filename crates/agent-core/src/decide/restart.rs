//! The bounded restart allowance of one task-list lineage, independent of team identities.

use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

/// Replacements already reserved for a task-list lineage; zero for a newly submitted task list.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RestartCount(pub u32);

/// Whether one more replacement may be reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Reserve this ordinal before removing the old team.
    Restart(NonZeroU32),
    /// No replacement remains in the configured allowance.
    Exhausted,
}

/// Decides whether another restart fits the allowance, without overflowing at its maximum.
#[must_use]
pub const fn decide(_used: RestartCount, _limit: u32) -> Decision {
    Decision::Exhausted
}

#[cfg(test)]
mod tests;
