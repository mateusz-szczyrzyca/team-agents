//! Atomic storage of approved split definitions with their owning team.

use crate::ids::ItemId;
use crate::state::split::SplitPlan;

use super::{CheckedMut, Team};

/// Why an approved split cannot be attached to a team.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecordSplitError {
    /// The original item is absent from this team's task list.
    #[error("cannot split unknown item {item}")]
    UnknownItem {
        /// The item absent from the team.
        item: ItemId,
    },
    /// The original item already has a plan, whose keys must remain stable.
    #[error("item {item} already has an approved split")]
    AlreadySplit {
        /// The item whose existing plan cannot be replaced.
        item: ItemId,
    },
}

impl Team {
    /// Whole approved plans, in the order recorded; empty for older teams.
    #[must_use]
    pub fn splits(&self) -> &[SplitPlan] {
        &[]
    }

    /// Attaches a whole validated plan without changing the item or its round.
    ///
    /// # Errors
    /// Returns [`RecordSplitError`] if the item is absent or already has a split.
    pub fn record_split(&mut self, plan: SplitPlan) -> Result<(), RecordSplitError> {
        Err(RecordSplitError::UnknownItem { item: plan.parent() })
    }
}

impl CheckedMut<'_, Team> {
    /// See [`Team::record_split`].
    ///
    /// # Errors
    /// Returns [`RecordSplitError`] for an absent or already split item.
    pub fn record_split(&mut self, plan: SplitPlan) -> Result<(), RecordSplitError> {
        self.value.record_split(plan)
    }
}

#[cfg(test)]
mod tests;
