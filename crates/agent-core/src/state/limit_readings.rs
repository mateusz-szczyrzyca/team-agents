//! The newest reading of each rate-limit window of an account, as the state keeps it: per account, and per agent for an
//! agent that has none. A reading is dropped when its window resets, because the window it told of is over.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use super::Timestamp;
use crate::config::AccountName;
use crate::ids::AgentName;
use crate::limit_reading::{LimitReading, LimitWindow};

#[cfg(test)]
mod tests;

/// Whose limits a reading tells of: an account's, or the agent's own when the agent has no account.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReadingOwner {
    /// An account of `agents.toml`.
    Account(AccountName),
    /// An agent without an account.
    Agent(AgentName),
}

impl fmt::Display for ReadingOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Account(account) => account.fmt(formatter),
            Self::Agent(agent) => agent.fmt(formatter),
        }
    }
}

/// A reading with the moment it was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeptReading {
    /// What the CLI told of the window.
    reading: LimitReading,
    /// When the daemon read it: the end of the run that showed it.
    read_at: Timestamp,
}

impl KeptReading {
    /// `reading`, read at `read_at`.
    #[must_use]
    pub const fn new(reading: LimitReading, read_at: Timestamp) -> Self {
        Self { reading, read_at }
    }

    /// What the CLI told of the window.
    #[must_use]
    pub const fn reading(&self) -> LimitReading {
        self.reading
    }

    /// When the reading was read.
    #[must_use]
    pub const fn read_at(&self) -> Timestamp {
        self.read_at
    }

    /// Whether the window the reading tells of has reset by `now`: the reading no longer says anything.
    #[must_use]
    pub fn is_over(&self, now: Timestamp) -> bool {
        self.reading.resets_at() <= now
    }
}

/// What is kept of one window after `new`, a reading of it, is read at `now`, when `kept` is what was kept before.
///
/// A reading whose window has reset is dropped, kept or new; of two readings the one read later is kept, and the new
/// one when both were read at the same moment, as the run that is settled last read last.
#[must_use]
pub fn keep_reading(kept: Option<KeptReading>, new: KeptReading, now: Timestamp) -> Option<KeptReading> {
    let kept = kept.filter(|kept| !kept.is_over(now));
    if new.is_over(now) {
        return kept;
    }
    Some(newer(kept, new))
}

/// The one of `kept` and `new` that was read later; `new` when they were read together.
fn newer(kept: Option<KeptReading>, new: KeptReading) -> KeptReading {
    match kept {
        Some(kept) if kept.read_at > new.read_at => kept,
        Some(_) | None => new,
    }
}

/// The kept readings of every owner, at most one per window each.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "ReadingsWire", into = "ReadingsWire")]
pub struct LimitReadings {
    /// The windows of each owner that have a reading; an owner without one has no entry.
    owners: BTreeMap<ReadingOwner, BTreeMap<LimitWindow, KeptReading>>,
}

impl LimitReadings {
    /// Keeps `readings` of `owner`, read at `read_at`, and forgets every reading that has reset by `now`.
    pub fn keep(&mut self, owner: &ReadingOwner, readings: &[LimitReading], read_at: Timestamp, now: Timestamp) {
        for reading in readings {
            let windows = self.owners.entry(owner.clone()).or_default();
            let window = reading.window();
            match keep_reading(windows.get(&window).copied(), KeptReading::new(*reading, read_at), now) {
                Some(kept) => {
                    windows.insert(window, kept);
                }
                None => {
                    windows.remove(&window);
                }
            }
        }
        self.forget_over(now);
    }

    /// Forgets the readings that have reset by `now`, and the owners left with none.
    fn forget_over(&mut self, now: Timestamp) {
        for windows in self.owners.values_mut() {
            windows.retain(|_, kept| !kept.is_over(now));
        }
        self.owners.retain(|_, windows| !windows.is_empty());
    }

    /// The readings that have not reset at `now`, by owner — accounts first, in the order of their names, then agents —
    /// each owner's readings in the order of [`LimitWindow`]; an owner without one is not listed.
    #[must_use]
    pub fn live(&self, now: Timestamp) -> Vec<(&ReadingOwner, Vec<KeptReading>)> {
        self.owners
            .iter()
            .map(|(owner, windows)| {
                let live: Vec<KeptReading> = windows.values().copied().filter(|kept| !kept.is_over(now)).collect();
                (owner, live)
            })
            .filter(|(_, live)| !live.is_empty())
            .collect()
    }

    /// Whether no reading is kept.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.owners.is_empty()
    }
}

/// How [`LimitReadings`] is written: one list of readings per account and per agent, so that the keys are plain names.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadingsWire {
    /// The readings of each account.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    accounts: BTreeMap<AccountName, Vec<KeptReading>>,
    /// The readings of each agent without an account.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    agents: BTreeMap<AgentName, Vec<KeptReading>>,
}

impl From<LimitReadings> for ReadingsWire {
    fn from(readings: LimitReadings) -> Self {
        let mut wire = Self::default();
        for (owner, windows) in readings.owners {
            let list = windows.into_values().collect();
            match owner {
                ReadingOwner::Account(account) => wire.accounts.insert(account, list),
                ReadingOwner::Agent(agent) => wire.agents.insert(agent, list),
            };
        }
        wire
    }
}

impl From<ReadingsWire> for LimitReadings {
    fn from(wire: ReadingsWire) -> Self {
        let accounts = wire
            .accounts
            .into_iter()
            .map(|(account, list)| (ReadingOwner::Account(account), list));
        let agents = wire
            .agents
            .into_iter()
            .map(|(agent, list)| (ReadingOwner::Agent(agent), list));
        let owners = accounts
            .chain(agents)
            .filter_map(|(owner, list)| {
                let mut windows: BTreeMap<LimitWindow, KeptReading> = BTreeMap::new();
                for kept in list {
                    let window = kept.reading.window();
                    windows.insert(window, newer(windows.get(&window).copied(), kept));
                }
                (!windows.is_empty()).then_some((owner, windows))
            })
            .collect();
        Self { owners }
    }
}
