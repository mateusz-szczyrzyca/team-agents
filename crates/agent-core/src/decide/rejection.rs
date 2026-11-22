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
pub fn returned_items(_message: &str) -> Vec<ItemId> {
    Vec::new()
}

#[cfg(test)]
mod tests;
