//! Activation of approved plans and durable progress through their rounds.

use super::{CheckedMut, RecordSplitError, SplitPlan, Team};
use crate::ids::ItemId;

/// Why a plan could not take over its parent's current round.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActivateSplitError {
    /// The plan cannot be attached to this team.
    #[error(transparent)]
    Record(#[from] RecordSplitError),
    /// The parent has no open working round.
    #[error("item {item} has no open working round to split")]
    NotWorking {
        /// The item requiring a working round.
        item: ItemId,
    },
    /// The current round must deliver the first part immediately.
    #[error("the first part must have no dependencies")]
    FirstBlocked,
}

impl Team {
    /// Records a whole plan and binds the working round to its first part.
    ///
    /// # Errors
    /// Returns [`ActivateSplitError`] without mutation if the plan or current round cannot be used.
    pub fn activate_split(&mut self, _plan: SplitPlan) -> Result<(), ActivateSplitError> {
        Ok(())
    }
}

impl CheckedMut<'_, Team> {
    /// See [`Team::activate_split`].
    ///
    /// # Errors
    /// Returns [`ActivateSplitError`] if activation cannot be performed atomically.
    pub fn activate_split(&mut self, plan: SplitPlan) -> Result<(), ActivateSplitError> {
        self.value.activate_split(plan)
    }
}

#[cfg(test)]
mod tests;
