//! Activation of approved plans and durable progress through their rounds.

use super::{CheckedMut, RecordSplitError, SplitPlan, Team};
use crate::ids::ItemId;
use crate::state::{
    ItemPhase,
    split::{PartDefinition, PartId},
};

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
    /// The next dependency-ready part of an activated split, in declaration order.
    #[must_use]
    pub fn next_part(&self, item: ItemId) -> Option<(PartId, &PartDefinition)> {
        let rounds: Vec<_> = self
            .rounds()
            .iter()
            .filter(|round| round.item() == Some(item))
            .collect();
        if !rounds.iter().any(|round| round.part().is_some()) {
            return None;
        }
        let plan = self.splits().iter().find(|plan| plan.parent() == item)?;
        let merged: std::collections::BTreeSet<_> = rounds
            .iter()
            .filter(|round| round.part_pull_request().is_some())
            .filter_map(|round| round.part())
            .collect();
        crate::decide::split::ready_entries(plan, &merged).next()
    }

    /// Records a whole plan and binds the working round to its first part.
    ///
    /// # Errors
    /// Returns [`ActivateSplitError`] without mutation if the plan or current round cannot be used.
    pub fn activate_split(&mut self, plan: SplitPlan) -> Result<(), ActivateSplitError> {
        let item = plan.parent();
        super::check_parent(self, item)?;
        if self.splits().iter().any(|old| old.parent() == item) {
            return Err(RecordSplitError::AlreadySplit { item }.into());
        }
        let round = self
            .round_of(item)
            .filter(|round| round.is_open())
            .filter(|_| {
                matches!(
                    self.item(item).map(crate::state::ItemProgress::phase),
                    Some(ItemPhase::InProgress { .. })
                )
            })
            .ok_or(ActivateSplitError::NotWorking { item })?
            .id()
            .clone();
        let (first, definition) = plan.entries().next().ok_or(ActivateSplitError::FirstBlocked)?;
        if !definition.depends_on.is_empty() {
            return Err(ActivateSplitError::FirstBlocked);
        }
        let target = self
            .rounds
            .iter_mut()
            .find(|candidate| candidate.id() == &round)
            .ok_or(ActivateSplitError::NotWorking { item })?;
        target.bind_part(first.key());
        self.splits.push(plan);
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

/// Checks each part against its plan and only the rounds preceding it.
pub(in crate::state::team) fn check_execution(team: &Team) -> Result<(), crate::state::RoundsError> {
    let active: std::collections::BTreeSet<_> = team
        .rounds()
        .iter()
        .filter(|round| round.part().is_some())
        .filter_map(crate::state::Round::item)
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    let mut merged = std::collections::BTreeSet::new();
    for round in team.rounds() {
        let Some(key) = round.part() else {
            if round.item().is_some_and(|item| active.contains(&item)) {
                return Err(crate::state::RoundsError::SplitExecution {
                    round: round.id().clone(),
                });
            }
            continue;
        };
        let parent = round.item();
        let definition = team
            .splits()
            .iter()
            .find(|plan| Some(plan.parent()) == parent)
            .and_then(|plan| plan.entries().find(|(id, _)| id.key() == key));
        let valid = definition.is_some_and(|(_, part)| {
            part.depends_on
                .iter()
                .all(|dependency| merged.contains(&(parent, *dependency)))
        });
        if !valid
            || !seen.insert((parent, key))
            || (round.part_pull_request().is_some() != (round.ended() == Some(crate::state::RoundEnd::Merged)))
        {
            return Err(crate::state::RoundsError::SplitExecution {
                round: round.id().clone(),
            });
        }
        if round.part_pull_request().is_some() {
            merged.insert((parent, key));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
