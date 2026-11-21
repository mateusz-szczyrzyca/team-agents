//! Durable capability waits, refreshed atomically from a team snapshot.

use super::{CheckedMut, Team};
use crate::ids::ItemId;
use crate::kinds::Tier;
use std::collections::BTreeMap;

impl Team {
    /// Items waiting for a capability absent from the crew; empty in older states.
    #[must_use]
    pub fn tier_waits(&self) -> &BTreeMap<ItemId, Tier> {
        &self.tier_waits
    }

    /// Refreshes capability waits and returns newly entered waits for notification.
    pub fn refresh_tier_waits(&mut self) -> Vec<(ItemId, Tier)> {
        use crate::decide::assign::dispatch::{WaitReason, dispatch};
        let next: BTreeMap<_, _> = dispatch(self)
            .waiting
            .into_iter()
            .filter(|wait| wait.reason == WaitReason::Absent)
            .map(|wait| (wait.item, wait.required))
            .collect();
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
