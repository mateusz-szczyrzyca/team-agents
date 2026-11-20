//! What the daemon keeps of the checks before a review, in `tasks/<lead>/checks.json`: how often each item was sent back,
//! what the human allowed for it, the question the round waits on and what the lead is still to be told.
//!
//! It is a file of the lead's task and not a part of the state: the state's items stay as the plan of the state made them,
//! and the checks are the autopilot's own business. Like the gate record, it is the daemon's record and nobody else's.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use super::values::{CommitId, Timestamp};
use crate::decide::checks::Subject;
use crate::ids::ItemId;
use crate::repo_path::RepoPath;

/// What the checks hold for one item.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemChecks {
    /// How many times the checks sent the item's round back to the team since the human last answered.
    pub passes: u32,
    /// What the human allowed for the item: the review starts with these as they are.
    pub allowed: BTreeSet<Subject>,
    /// What of `allowed` the human allowed in answer to a question that named only what they alone may allow: it covers
    /// the issues of that kind about the subject and not an issue of the team's about the same file. A file written
    /// before this key existed has none.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub human_only: BTreeSet<Subject>,
    /// The head the checks looked at when the human allowed a file: an allowance for a file does not outlast a change of
    /// that file after it. A file with no head here was allowed before this key existed and holds for any.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub allowed_at: BTreeMap<RepoPath, CommitId>,
}

/// What a question of the checks covers of the issues the checks found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Covers {
    /// Every issue the checks found: an allowance covers all that is found about the subject.
    #[default]
    Everything,
    /// Only the issues that the human alone may allow, while the team still has issues of its own to settle.
    HumanPart,
}

/// What the question in [`ChecksRecord::asked`] was put about, besides its subjects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Basis {
    /// The head the checks looked at when they wrote the question.
    pub head: CommitId,
    /// What the question covers.
    pub covers: Covers,
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
    /// What the question in `asked` was put about; none for a question an earlier daemon wrote, which covers everything
    /// and holds for any head.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<Basis>,
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

    /// What the human allowed for any of `items` in answer to a question that named only their part, without repeats.
    #[must_use]
    pub fn human_only_of(&self, items: &[ItemId]) -> Vec<Subject> {
        let allowed: BTreeSet<&Subject> = items
            .iter()
            .filter_map(|item| self.items.get(item))
            .flat_map(|held| held.human_only.iter())
            .collect();
        allowed.into_iter().cloned().collect()
    }

    /// The heads at which the files of `items` were allowed, for those that have one.
    #[must_use]
    pub fn allowed_at_of(&self, items: &[ItemId]) -> BTreeMap<RepoPath, CommitId> {
        items
            .iter()
            .filter_map(|item| self.items.get(item))
            .flat_map(|held| held.allowed_at.iter())
            .map(|(path, head)| (path.clone(), head.clone()))
            .collect()
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

    /// Records that the human allowed `subjects` for every one of `items` in answer to a question of `basis`.
    pub fn allow_on(&mut self, items: &[ItemId], subjects: &[Subject], basis: &Basis) {
        for item in items {
            let held = self.items.entry(*item).or_default();
            match basis.covers {
                Covers::Everything => {
                    for subject in subjects {
                        held.human_only.remove(subject);
                    }
                }
                Covers::HumanPart => {
                    let fresh: Vec<Subject> = subjects
                        .iter()
                        .filter(|subject| !held.allowed.contains(subject))
                        .cloned()
                        .collect();
                    held.human_only.extend(fresh);
                }
            }
            held.allowed.extend(subjects.iter().cloned());
            held.allowed_at
                .extend(subjects.iter().filter_map(|subject| match subject {
                    Subject::Path(path) => Some((path.clone(), basis.head.clone())),
                    Subject::NoTests => None,
                }));
        }
    }

    /// Records that the checks sent back the round of `items` for the `passes`-th time: no question is waited on.
    pub fn send_back(&mut self, items: &[ItemId], passes: u32) {
        self.set_passes(items, passes);
        self.asked = None;
        self.basis = None;
    }

    /// Takes the human's answer to the question that is waited on: `allowed` holds for `items` as the question covered
    /// them, and the count of passes starts again unless the question was about the human's part while the team still
    /// had passes — what the team used of them stays used. A question an earlier daemon wrote has no basis: it covered
    /// everything and holds for any head.
    pub fn answer(&mut self, items: &[ItemId], allowed: &[Subject]) {
        if let Some(basis) = self.basis.take() {
            self.allow_on(items, allowed, &basis);
            if basis.covers == Covers::Everything {
                self.restart_passes(items);
            }
        } else {
            self.allow(items, allowed);
            self.restart_passes(items);
        }
        self.asked = None;
    }

    /// Takes back what the human allowed for `subjects`: they are asked about again.
    pub fn revoke(&mut self, items: &[ItemId], subjects: &[Subject]) {
        for item in items {
            let Some(held) = self.items.get_mut(item) else {
                continue;
            };
            for subject in subjects {
                held.allowed.remove(subject);
                held.human_only.remove(subject);
                if let Subject::Path(path) = subject {
                    held.allowed_at.remove(path);
                }
            }
        }
    }

    /// Starts the count of passes again for `items`: the human decided.
    pub fn restart_passes(&mut self, items: &[ItemId]) {
        self.set_passes(items, 0);
    }
}
