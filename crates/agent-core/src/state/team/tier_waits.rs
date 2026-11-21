//! Durable capability waits, refreshed atomically from a team snapshot.

use super::{CheckedMut, Team};
use crate::ids::ItemId;
use crate::kinds::Tier;
use std::collections::BTreeMap;

impl Team {
    /// Capability waits that still need a human question.
    #[must_use]
    pub fn unasked_tier_waits(&self) -> Vec<(ItemId, Tier)> {
        Vec::new()
    }

    /// Records that the current capability waits were asked of the human.
    pub fn mark_tier_questions(&mut self) {}

    /// Items waiting for a capability absent from the crew; empty in older states.
    #[must_use]
    pub fn tier_waits(&self) -> &BTreeMap<ItemId, Tier> {
        &self.tier_waits
    }

    /// Refreshes capability waits and returns newly entered waits for notification.
    pub fn refresh_tier_waits(&mut self) -> Vec<(ItemId, Tier)> {
        use crate::decide::assign::dispatch::{WaitReason, dispatch};
        let mut next: BTreeMap<_, _> = dispatch(self)
            .waiting
            .into_iter()
            .filter(|wait| wait.reason == WaitReason::Absent)
            .map(|wait| (wait.item, wait.required))
            .collect();
        for item in self.items() {
            if let crate::state::ItemPhase::InReview { reviews, .. } = item.phase()
                && !reviews.is_empty()
                && reviews
                    .iter()
                    .all(|review| matches!(review.outcome, crate::state::ReviewOutcome::Submitted { .. }))
                && reviews.iter().any(|review| {
                    matches!(
                        review.outcome,
                        crate::state::ReviewOutcome::Submitted {
                            verdict: crate::status::ReviewVerdict::RequestChanges,
                            ..
                        }
                    )
                })
                && crate::decide::assign::dispatch::fixer(self, item.id()).is_none()
            {
                next.insert(
                    item.id(),
                    crate::decide::assign::dispatch::required_tier(self, item.id()),
                );
            }
        }
        let entered = next
            .iter()
            .filter(|(item, tier)| self.tier_waits.get(item) != Some(tier))
            .map(|(item, tier)| (*item, *tier))
            .collect();
        self.tier_waits = next;
        entered
    }
}

impl CheckedMut<'_, Team> {
    /// See [`Team::refresh_tier_waits`].
    pub fn refresh_tier_waits(&mut self) -> Vec<(ItemId, Tier)> {
        self.value.refresh_tier_waits()
    }
}

#[cfg(test)]
mod tests;
