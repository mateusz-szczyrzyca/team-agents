//! A rejection: what the lead of a team says in its final message when it sends an item back to the member that handed
//! it over, instead of taking the work. The count of the rejections of an item is kept in the item's state and is
//! compared with `[replacement] rejections` (`docs/architecture.md`).
//!
//! The line is `RETURNED: <item>`, one per rejected item, beside the `STATUS:` line. [`returned_items`] reads them out
//! of a final message the way the `STATUS:` line is read — through the markup of a markdown answer — and the machine
//! decides which of them are rejections from what it saw itself: see [`super::super::state::ItemProgress`].

use crate::ids::ItemId;

/// The word that starts the line of a lead's final message that sends an item back, before its `:`.
const RETURNED: &str = "RETURNED";

/// The items a lead's final message `message` sends back, in the order of its lines.
///
/// A line is read as a return when it starts with [`RETURNED`] and a `:` once the markup characters `*` and `` ` `` are
/// dropped, like the `STATUS:` line, and what follows is an item number such as `1.1` or `1.1a`. A line whose answer
/// is no item number — a title, a sentence, an item of another task list that starts differently — is skipped: the
/// machine reads no item out of it and never guesses. An item named twice is returned once, where it is named first.
#[must_use]
pub fn returned_items(message: &str) -> Vec<ItemId> {
    let mut returned: Vec<ItemId> = Vec::new();
    for line in message.lines() {
        let plain: String = line.chars().filter(|c| !matches!(*c, '*' | '`')).collect();
        let Some(value) = plain
            .trim_start()
            .strip_prefix(RETURNED)
            .and_then(|rest| rest.strip_prefix(':'))
        else {
            continue;
        };
        let Some(item) = value.split_whitespace().next().and_then(|text| text.parse().ok()) else {
            continue;
        };
        if !returned.contains(&item) {
            returned.push(item);
        }
    }
    returned
}

/// How many times a lead returned an item, and the limit `[replacement] rejections` it is compared with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rejections {
    /// How many rejections the machine counted for the item.
    counted: u32,
    /// The limit `[replacement] rejections`: `0` never replaces the agent.
    limit: u32,
}

impl Rejections {
    /// `counted` rejections against `limit`.
    #[must_use]
    pub const fn new(counted: u32, limit: u32) -> Self {
        Self { counted, limit }
    }

    /// How many rejections the machine counted.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.counted
    }

    /// The limit it is compared with.
    #[must_use]
    pub const fn limit(self) -> u32 {
        self.limit
    }

    /// No rejection counted, against `limit`.
    #[must_use]
    pub const fn none(limit: u32) -> Self {
        Self { counted: 0, limit }
    }
}

#[cfg(test)]
mod tests;
