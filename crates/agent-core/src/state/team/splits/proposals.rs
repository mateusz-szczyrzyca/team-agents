//! Team ownership of durable proposals, independent of their future daemon processing.

use super::{CheckedMut, RecordSplitError, Team};
use crate::ids::ItemId;
use crate::state::split::keeping::ProposalKeeping;

impl Team {
    /// Pending proposal and correction progress for an original item, if any.
    pub fn split_proposal(&self, item: ItemId) -> Option<&ProposalKeeping> {
        self.split_proposals.get(&item)
    }
    /// Stores proposal progress without changing approved plans, rounds or item progress.
    ///
    /// # Errors
    /// Returns [`RecordSplitError`] if the parent is absent or already split.
    pub fn keep_split_proposal(&mut self, item: ItemId, keeping: ProposalKeeping) -> Result<(), RecordSplitError> {
        super::check_parent(self, item)?;
        if self.splits.iter().any(|plan| plan.parent() == item) {
            return Err(RecordSplitError::AlreadySplit { item });
        }
        match self.split_proposals.entry(item) {
            std::collections::btree_map::Entry::Occupied(mut entry) => *entry.get_mut() = keeping,
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(keeping);
            }
        }
        Ok(())
    }
}
impl CheckedMut<'_, Team> {
    /// See [`Team::keep_split_proposal`].
    ///
    /// # Errors
    /// Returns [`RecordSplitError`] for an absent or already split parent.
    pub fn keep_split_proposal(&mut self, item: ItemId, keeping: ProposalKeeping) -> Result<(), RecordSplitError> {
        self.value.keep_split_proposal(item, keeping)
    }
}
#[cfg(test)]
mod tests;
