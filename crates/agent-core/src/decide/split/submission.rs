//! Which `SPLIT: <item>` line of a coordinator's final message the harness takes, and which it refuses.
//!
//! A line that names an item of the team that is not split yet is a proposal (read and judged by [`approval`]). A line the
//! harness cannot take — the item is split already, or no item of the team has that name — is a refusal that its author is
//! told, so that it is never left waiting for a correction nobody asked it for.
//!
//! [`approval`]: super::approval

use std::collections::BTreeSet;

use crate::ids::ItemId;

/// What a line of the final message starts with when it submits a split; the item follows.
const SPLIT_PREFIX: &str = "SPLIT:";

/// What the `SPLIT:` lines of a message ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submission {
    /// The message has no `SPLIT:` line.
    NotSubmitted,
    /// The message proposes a split of this item.
    Taken(ItemId),
    /// The harness cannot take the submission.
    Refused(Refusal),
}

/// Why a `SPLIT:` line cannot be taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The item has a plan already: an item is split once, and what does not fit its parts is a task of its own.
    AlreadySplit {
        /// The item.
        item: ItemId,
    },
    /// No item of the team has the name that follows `SPLIT:`.
    UnknownItem {
        /// What follows `SPLIT:`, trimmed.
        text: String,
    },
}

/// What the `SPLIT:` lines of `message` ask of a team with `items`, of which those in `split` are split already.
///
/// Only a line that has the form of a proposal counts: `SPLIT:` and an item number. One that names an item that is not split
/// wins, in the order of `items`; otherwise the first line that names a split item is refused as such, and otherwise the
/// first line is refused as naming no item of the team.
#[must_use]
pub fn judge_submission(items: &[ItemId], split: &BTreeSet<ItemId>, message: Option<&str>) -> Submission {
    let named: Vec<&str> = message
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.trim().strip_prefix(SPLIT_PREFIX))
        .map(str::trim)
        // Only `SPLIT:` and an item number is a submission: `SPLIT: none` in a report asks nothing.
        .filter(|text| text.parse::<ItemId>().is_ok_and(|item| item.to_string() == *text))
        .collect();
    let Some(first) = named.first() else {
        return Submission::NotSubmitted;
    };
    let is_named = |item: &ItemId| named.iter().any(|text| *text == item.to_string());
    if let Some(item) = items.iter().find(|item| !split.contains(item) && is_named(item)) {
        return Submission::Taken(*item);
    }
    if let Some(item) = named.iter().find_map(|text| {
        items
            .iter()
            .find(|item| split.contains(item) && item.to_string() == *text)
    }) {
        return Submission::Refused(Refusal::AlreadySplit { item: *item });
    }
    Submission::Refused(Refusal::UnknownItem {
        text: (*first).to_owned(),
    })
}

#[cfg(test)]
mod tests;
