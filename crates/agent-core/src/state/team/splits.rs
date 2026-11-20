//! Atomic storage of approved split definitions with their owning team.

use crate::ids::ItemId;
use crate::state::split::SplitPlan;

use super::{CheckedMut, Team};

pub(super) mod execution;
mod proposals;
pub use execution::ActivateSplitError;

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
        &self.splits
    }

    /// Attaches a whole validated plan without changing the item or its round.
    ///
    /// # Errors
    /// Returns [`RecordSplitError`] if the item is absent or already has a split.
    pub fn record_split(&mut self, plan: SplitPlan) -> Result<(), RecordSplitError> {
        check_parent(self, plan.parent())?;
        if self.splits.iter().any(|old| old.parent() == plan.parent()) {
            return Err(RecordSplitError::AlreadySplit { item: plan.parent() });
        }
        self.split_proposals.retain(|parent, _| *parent != plan.parent());
        self.splits.push(plan);
        Ok(())
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

/// Validates membership and uniqueness without changing the persisted order.
pub(super) fn check_splits(team: &Team) -> Result<(), super::TeamError> {
    let mut parents = std::collections::BTreeSet::new();
    for plan in &team.splits {
        check_parent(team, plan.parent())
            .and_then(|()| {
                if parents.insert(plan.parent()) {
                    Ok(())
                } else {
                    Err(RecordSplitError::AlreadySplit { item: plan.parent() })
                }
            })
            .map_err(|source| super::TeamError::Split {
                team: team.id.clone(),
                source,
            })?;
    }
    for parent in team.split_proposals.keys() {
        check_parent(team, *parent)
            .and_then(|()| {
                if parents.contains(parent) {
                    Err(RecordSplitError::AlreadySplit { item: *parent })
                } else {
                    Ok(())
                }
            })
            .map_err(|source| super::TeamError::Split {
                team: team.id.clone(),
                source,
            })?;
    }
    Ok(())
}

/// A stored plan must name one of the team's original task-list items.
fn check_parent(team: &Team, item: ItemId) -> Result<(), RecordSplitError> {
    if team.item(item).is_none() {
        return Err(RecordSplitError::UnknownItem { item });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
