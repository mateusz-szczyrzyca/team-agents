//! A rejection: what the lead of a team says in its final message when it sends an item back to the member that handed
//! it over, instead of taking the work. The count of the rejections of an item is kept in the item's state and is
//! compared with `[replacement] rejections` (`docs/architecture.md`).
//!
//! The line is `RETURNED: <item>`, one per rejected item, beside the `STATUS:` line. [`returned_items`] reads them out
//! of a final message the way the `STATUS:` line is read — through the markup of a markdown answer — and the machine
//! decides which of them are rejections from what it saw itself: see [`super::super::state::ItemProgress`].

use std::num::NonZeroU32;

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

/// The line of a lead's final message that sends an item back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnLine<'a> {
    /// The item the line names.
    item: &'a ItemId,
}

impl<'a> ReturnLine<'a> {
    /// The line that names `item`.
    #[must_use]
    pub const fn returned(item: &'a ItemId) -> Self {
        Self { item }
    }
}

/// How the lead's run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeadEnd {
    /// It ended with a status; `Questions`, `Failed` and `Excluded` are no answer to a hand-over.
    Settled(crate::status::RunOutcome),
    /// It ended without a status, was stopped, hung, or hit a limit: it did not take the work.
    Unsettled,
}

/// Who holds the item now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// A member works on it.
    ByTheMember,
    /// The lead took it, or the item is pending, under review, approved or done.
    Nobody,
}

/// How the member's last run of the item ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastRun {
    /// The member never ran on the item.
    Never,
    /// It ended with this status.
    Reported(crate::status::RunOutcome),
    /// It ended without a status.
    NoStatus,
    /// The human stopped it.
    Stopped,
}

/// Whether the item moved on after the run it was handed over after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovedOn {
    /// It did not: it is still in progress, and the agent that ran is its assignee.
    No,
    /// It left its phase, or was given another agent.
    Yes,
}

/// What the machine saw of an item when its lead ended with the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    /// How the lead's run ended.
    lead: LeadEnd,
    /// Who holds the item now.
    held: Held,
    /// How the member's last run of the item ended.
    last_run: LastRun,
    /// Whether the item moved on after the run it was handed over after.
    moved_on: MovedOn,
    /// The number of the run of the item the hand-over was made after.
    handed_over_after: NonZeroU32,
    /// The number of the member's last run.
    last_run_number: NonZeroU32,
}

impl Facts {
    /// The facts of an item whose lead ended `SLEEPING`: who holds it, how the member's last run ended, whether it moved
    /// on, and the numbers of the run it was handed over after and of the member's last run.
    #[must_use]
    pub const fn new(
        held: Held,
        last_run: LastRun,
        moved_on: MovedOn,
        handed_over_after: NonZeroU32,
        last_run_number: NonZeroU32,
    ) -> Self {
        Self {
            lead: LeadEnd::Settled(crate::status::RunOutcome::Sleeping),
            held,
            last_run,
            moved_on,
            handed_over_after,
            last_run_number,
        }
    }

    /// The same facts with the lead's end `lead`.
    #[must_use]
    pub const fn with_lead(self, lead: LeadEnd) -> Self {
        Self { lead, ..self }
    }

    /// The same facts with `held` holding the item.
    #[must_use]
    pub const fn with_held(self, held: Held) -> Self {
        Self { held, ..self }
    }

    /// The same facts with the member's last run ending `last_run`.
    #[must_use]
    pub const fn with_last_run(self, last_run: LastRun) -> Self {
        Self { last_run, ..self }
    }

    /// The same facts with the member's last run numbered `run`.
    #[must_use]
    pub const fn with_last_run_of(self, run: NonZeroU32) -> Self {
        Self {
            last_run_number: run,
            ..self
        }
    }

    /// The same facts after the item moved on.
    #[must_use]
    pub const fn moved_on(self) -> Self {
        Self {
            moved_on: MovedOn::Yes,
            ..self
        }
    }
}

/// What a return of `line` counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// The lead read the member's work and sent it back: a rejection.
    Counted,
    /// No work that was handed over stands behind the return: not a rejection.
    NotHandedOver,
}

/// What `line` of a lead's final message counts, from the facts the machine saw of the item.
#[must_use]
pub fn classify(line: &ReturnLine<'_>, facts: Facts) -> Option<Rejection> {
    let _ = line;
    let _ = facts;
    None
}

#[cfg(test)]
mod facts_tests;
#[cfg(test)]
mod tests;
