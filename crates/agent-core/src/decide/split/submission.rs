//! Which `SPLIT: <item>` line of a coordinator's final message the harness takes, and which it refuses.
//!
//! A line that names an item of the team that is not split yet is a proposal (read and judged by [`approval`]). A line the
//! harness cannot take — the item is split already, or no item of the team has that name — is a refusal that its author is
//! told, so that it is never left waiting for a correction nobody asked it for.
//!
//! [`approval`]: super::approval

use std::collections::BTreeSet;

use crate::ids::ItemId;

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
/// A line that names an item that is not split wins, in the order of `items`; otherwise the first line that names a split
/// item is refused as such, and otherwise the first line is refused as naming no item.
#[must_use]
pub fn judge_submission(_items: &[ItemId], _split: &BTreeSet<ItemId>, _message: Option<&str>) -> Submission {
    Submission::NotSubmitted
}

#[cfg(test)]
mod tests;
