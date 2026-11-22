//! A rejection: what the lead of a team says in its final message when it sends an item back to the member that handed
//! it over, instead of taking the work. The count of the rejections of an item is kept in the item's state and is
//! compared with `[replacement] rejections` (`docs/architecture.md`).
//!
//! The line is `RETURNED: <item>`, one per rejected item, beside the `STATUS:` line. [`returned_items`] reads them out
//! of a final message the way the `STATUS:` line is read — through the markup of a markdown answer — and the machine
//! decides which of them are rejections from what it saw itself: see [`super::super::state::ItemProgress`].

use std::num::NonZeroU32;

use crate::ids::ItemId;
use crate::state::{RunEnd, RunState};

/// The word that starts the line of a lead's final message that sends an item back, before its `:`.
const RETURNED: &str = "RETURNED";

/// The items a lead's final message `message` sends back, in the order of its lines.
///
/// A line is read as a return when it starts with `RETURNED` and a `:` once the markup characters `*` and `` ` `` are
/// dropped, like the `STATUS:` line, and what follows is an item number such as `1.1` or `1.1a`. A line whose answer
/// is no item number — a title, a sentence, an item of another task list that starts differently — is skipped: the
/// machine reads no item out of it and never guesses. An item named twice is returned once, where it is named first.
#[must_use]
pub fn returned_items(message: &str) -> Vec<ItemId> {
    let mut returned: Vec<ItemId> = Vec::new();
    for word in return_words(message) {
        let Ok(item) = word.parse() else {
            continue;
        };
        if !returned.contains(&item) {
            returned.push(item);
        }
    }
    returned
}

/// The first word after the `:` of every `RETURNED:` line of `message`, in order.
fn return_words(message: &str) -> Vec<String> {
    message
        .lines()
        .filter_map(|line| {
            let plain: String = line.chars().filter(|c| !matches!(*c, '*' | '`')).collect();
            let value = plain.trim_start().strip_prefix(RETURNED)?.strip_prefix(':')?;
            value.split_whitespace().next().map(str::to_owned)
        })
        .collect()
}

/// What a `RETURNED:` line names: an item, or one part of a split item (`1.1a`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnTarget {
    /// The item the line names; for a part, its parent item.
    item: ItemId,
    /// The part of the item, when the line names one.
    part: Option<crate::state::split::PartKey>,
}

impl ReturnTarget {
    /// The item, or the parent item of the part.
    #[must_use]
    pub const fn item(&self) -> ItemId {
        self.item
    }

    /// The part of the item the line names; `None` when it names the item itself.
    #[must_use]
    pub const fn part(&self) -> Option<crate::state::split::PartKey> {
        self.part
    }
}

/// What a lead's final message `message` sends back, in the order of its lines: items, and parts of split items.
///
/// A line is read as [`returned_items`] reads it, and its answer is an item number or an item number and one lowercase
/// letter, the key of a part (`1.1a`). Anything else is skipped. A target named twice is returned once.
#[must_use]
pub fn returned_targets(message: &str) -> Vec<ReturnTarget> {
    let mut targets: Vec<ReturnTarget> = Vec::new();
    for word in return_words(message) {
        let Some(target) = target_of(&word) else {
            continue;
        };
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    targets
}

/// The item or the part `word` names: `1.1`, or `1.1` and a lowercase letter.
fn target_of(word: &str) -> Option<ReturnTarget> {
    if let Ok(item) = word.parse() {
        return Some(ReturnTarget { item, part: None });
    }
    let letter = word.chars().next_back().filter(char::is_ascii_lowercase)?;
    let item = word.strip_suffix(letter)?.parse().ok()?;
    let part = crate::state::split::PartKey::try_from(letter.to_string()).ok()?;
    Some(ReturnTarget { item, part: Some(part) })
}

/// The runs of `runs` that worked for the round `round`, in order: the runs of one item, not of the items the member held
/// before it.
#[must_use]
pub fn runs_of_round(runs: &[crate::state::Run], _round: &crate::ids::RoundId) -> Vec<crate::state::Run> {
    runs.to_vec()
}

/// The run of `runs` that the member's last hand-over was made after: the last one that ended `DONE`.
#[must_use]
pub fn handed_over_run(runs: &[crate::state::Run]) -> Option<&crate::ids::RunId> {
    runs.iter()
        .rev()
        .find(|run| {
            matches!(
                run.state(),
                RunState::Ended {
                    end: RunEnd::Reported {
                        outcome: crate::status::RunOutcome::Done
                    },
                    ..
                }
            )
        })
        .map(crate::state::Run::id)
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
    /// A run of it is going: it has not handed anything over after it.
    Running,
}

/// Whether the item moved on after the run it was handed over after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovedOn {
    /// It did not: it is still in progress, and the agent that ran is its assignee.
    No,
    /// It left its phase, or was given another agent.
    Yes,
}

/// Whether the work handed over was sent back already, by a return the member has not run after yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answered {
    /// No earlier return of this hand-over is waiting for the member to run again.
    No,
    /// An earlier return of it is recorded and the member has not been started since: the lead repeats itself.
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
    /// Whether the hand-over was sent back already.
    answered: Answered,
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
            answered: Answered::No,
        }
    }

    /// The same facts with `answered` saying whether the hand-over was sent back already.
    #[must_use]
    pub const fn with_answered(self, answered: Answered) -> Self {
        Self { answered, ..self }
    }

    /// The facts of the item as its member's `runs` show them, with `held` holding it and `answered` saying whether the
    /// hand-over was sent back already: the end of the last run, the number of the last run, and the number of the last
    /// run that ended `DONE`, which is the one the hand-over was made after.
    #[must_use]
    pub fn from_runs(held: Held, answered: Answered, runs: &[crate::state::Run]) -> Self {
        let last_run = runs.last().map_or(LastRun::Never, |run| match run.state() {
            RunState::Running => LastRun::Running,
            RunState::Ended { end, .. } => match end {
                RunEnd::Reported { outcome } => LastRun::Reported(*outcome),
                RunEnd::Stopped => LastRun::Stopped,
                RunEnd::NoStatus { .. }
                | RunEnd::QuotaExhausted { .. }
                | RunEnd::RateLimited
                | RunEnd::Hung
                | RunEnd::TurnLimit => LastRun::NoStatus,
            },
        });
        let handed_over_after = runs
            .iter()
            .rposition(|run| {
                matches!(
                    run.state(),
                    RunState::Ended {
                        end: RunEnd::Reported {
                            outcome: crate::status::RunOutcome::Done
                        },
                        ..
                    }
                )
            })
            .map_or(NonZeroU32::MIN, |index| run_number(index.saturating_add(1)));
        let last_run_number = run_number(runs.len().max(1));
        Self::new(held, last_run, MovedOn::No, handed_over_after, last_run_number).with_answered(answered)
    }

    /// The facts of the item as `from_runs` reads them from the runs of its member that started before `lead_started_at`,
    /// the start of the lead's run that ended with the line: a run the member was started for after the lead began is the
    /// consequence of the return, not the work the lead read.
    #[must_use]
    pub fn from_runs_before(
        held: Held,
        answered: Answered,
        runs: &[crate::state::Run],
        lead_started_at: crate::state::Timestamp,
    ) -> Self {
        let before = runs.partition_point(|run| run.started_at() < lead_started_at);
        // `partition_point` never passes the end, so the prefix is always there; the whole list is the harmless fallback.
        Self::from_runs(held, answered, runs.get(..before).unwrap_or(runs))
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

/// The run number `position`, counted from one: a list of runs is never as long as `u32` can count, so the largest
/// number stands for a position beyond it.
#[must_use]
pub fn run_number(position: usize) -> NonZeroU32 {
    u32::try_from(position)
        .ok()
        .and_then(NonZeroU32::new)
        .unwrap_or(NonZeroU32::MAX)
}

/// The number of the member's last run when it started at `at` or later: the member was started again since the lead's
/// run began; `None` when no run of it did.
#[must_use]
pub fn run_started_since(runs: &[crate::state::Run], at: crate::state::Timestamp) -> Option<NonZeroU32> {
    let before = runs.partition_point(|run| run.started_at() < at);
    (before < runs.len()).then(|| run_number(runs.len()))
}

/// What a return of `line` counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// The lead read the member's work and sent it back: a rejection.
    Counted,
    /// No work that was handed over stands behind the return: not a rejection.
    NotHandedOver,
    /// The hand-over was sent back already and the member has not run since: the same rejection, counted once.
    AlreadyCounted,
}

/// What `line` of a lead's final message counts, from the facts the machine saw of the item.
#[must_use]
pub fn classify(_line: &ReturnLine<'_>, facts: Facts) -> Option<Rejection> {
    // The line names the item, and the daemon gathers `facts` for exactly that item; the facts say whether work was
    // handed over behind it.
    // A lead that asked, failed or ended without a status did not answer the hand-over, and the item it names is not
    // the item it read: nothing it wrote counts.
    match facts.lead {
        LeadEnd::Settled(crate::status::RunOutcome::Sleeping | crate::status::RunOutcome::Done) => {}
        LeadEnd::Settled(
            crate::status::RunOutcome::Questions
            | crate::status::RunOutcome::Failed
            | crate::status::RunOutcome::Excluded,
        )
        | LeadEnd::Unsettled => return None,
    }
    let handed_over = facts.held == Held::ByTheMember
        && facts.last_run == LastRun::Reported(crate::status::RunOutcome::Done)
        && facts.moved_on == MovedOn::No
        && facts.last_run_number == facts.handed_over_after;
    Some(match (handed_over, facts.answered) {
        (false, Answered::No | Answered::Yes) => Rejection::NotHandedOver,
        (true, Answered::Yes) => Rejection::AlreadyCounted,
        (true, Answered::No) => Rejection::Counted,
    })
}

#[cfg(test)]
mod answered_tests;
#[cfg(test)]
mod before_tests;
#[cfg(test)]
mod facts_tests;
#[cfg(test)]
mod round_runs_tests;
#[cfg(test)]
mod runs_tests;
#[cfg(test)]
mod targets_tests;
#[cfg(test)]
mod tests;
