//! What the daemon keeps of the checks before a review, in `tasks/<lead>/checks.json`: how often each item was sent back,
//! what the human allowed for it, the question the round waits on and what the lead is still to be told.
//!
//! It is a file of the lead's task and not a part of the state: the state's items stay as the plan of the state made them,
//! and the checks are the autopilot's own business. Like the gate record, it is the daemon's record and nobody else's.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use super::values::Timestamp;
use crate::decide::checks::Subject;
use crate::ids::ItemId;

/// What the checks hold for one item.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemChecks {
    /// How many times the checks sent the item's round back to the team since the human last answered.
    pub passes: u32,
    /// What the human allowed for the item: the review starts with these as they are.
    pub allowed: BTreeSet<Subject>,
}

/// A question to the human that the round waits on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asked {
    /// The number of the lead's questions the question is: the number in the name of its file. A record written with the
    /// key `round`, as it was called before, is read all the same.
    #[serde(alias = "round")]
    pub number: NonZeroU32,
    /// When it was written.
    pub at: Timestamp,
    /// The items of the round, which share the pull request.
    pub items: Vec<ItemId>,
    /// What each question of the file is about, in the order of the questions.
    pub subjects: Vec<Subject>,
    /// How many times the lead was woken to put the question and ended without doing it.
    pub relays: u32,
}

/// What the lead of a team is to be told on its next run: the round goes back to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sent {
    /// When the team was sent back.
    pub at: Timestamp,
    /// Which pass of the checks this is, counted from 1.
    pub pass: u32,
    /// How many passes the checks have, as configured when it was sent.
    pub of: u32,
    /// The reasons, as the lead is told them.
    pub text: String,
}

/// The record of the checks of one team's lead.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChecksRecord {
    /// What the checks hold for each item that went through them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub items: BTreeMap<ItemId, ItemChecks>,
    /// The question the round waits on, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked: Option<Asked>,
    /// What the lead is to be told when it runs next, until it has been told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent: Option<Sent>,
}

impl ChecksRecord {
    /// How many times the checks sent back the round of `items`, which move together: the most any of them has.
    #[must_use]
    pub fn passes_of(&self, items: &[ItemId]) -> u32 {
        items
            .iter()
            .filter_map(|item| self.items.get(item))
            .map(|held| held.passes)
            .max()
            .unwrap_or(0)
    }

    /// What the human allowed for any of `items`, without repeats.
    #[must_use]
    pub fn allowed_of(&self, items: &[ItemId]) -> Vec<Subject> {
        let allowed: BTreeSet<&Subject> = items
            .iter()
            .filter_map(|item| self.items.get(item))
            .flat_map(|held| held.allowed.iter())
            .collect();
        allowed.into_iter().cloned().collect()
    }

    /// Records that the checks sent back the round of `items` for the `passes`-th time.
    pub fn set_passes(&mut self, items: &[ItemId], passes: u32) {
        for item in items {
            self.items.entry(*item).or_default().passes = passes;
        }
    }

    /// Records that the human allowed `subjects` for every one of `items`.
    pub fn allow(&mut self, items: &[ItemId], subjects: &[Subject]) {
        for item in items {
            self.items
                .entry(*item)
                .or_default()
                .allowed
                .extend(subjects.iter().cloned());
        }
    }

    /// Starts the count of passes again for `items`: the human decided.
    pub fn restart_passes(&mut self, items: &[ItemId]) {
        self.set_passes(items, 0);
    }
}
