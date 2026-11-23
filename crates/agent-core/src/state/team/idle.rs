//! Persisted observations survive daemon restarts without granting a new idle window.
use super::Team;
use crate::state::checked::CheckedMut;
use crate::state::{CommitId, Timestamp};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Last work observed in a team and the refusal already reported to the human.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdleWatch {
    /// Most recent run activity or newly observed commit.
    pub last_activity: Timestamp,
    /// Local branch and worktree reference names with their last observed tips.
    pub heads: BTreeMap<String, CommitId>,
    /// A refusal already notified, cleared when new work is observed.
    pub refusal: Option<String>,
}
impl IdleWatch {
    /// Reconstructs first activity; subsequent new or changed tips count at observation time.
    #[must_use]
    pub fn observe(
        previous: Option<&Self>,
        recorded: Timestamp,
        _now: Timestamp,
        heads: BTreeMap<String, CommitId>,
    ) -> Self {
        Self {
            last_activity: recorded,
            heads,
            refusal: previous.and_then(|p| p.refusal.clone()),
        }
    }
}
impl Team {
    /// The last durable idle observation, absent on an older state.
    #[must_use]
    pub const fn idle_watch(&self) -> Option<&IdleWatch> {
        self.idle_watch.as_ref()
    }
    /// Keeps idle observations with the team across daemon restarts.
    pub fn record_idle_watch(&mut self, _watch: IdleWatch) {}
}
impl CheckedMut<'_, Team> {
    /// See [`Team::record_idle_watch`].
    pub fn record_idle_watch(&mut self, watch: IdleWatch) {
        self.value.record_idle_watch(watch);
    }
}
#[cfg(test)]
mod tests;
