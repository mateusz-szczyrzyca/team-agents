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
        Vec::new()
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
