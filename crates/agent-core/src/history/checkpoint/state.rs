//! What a checkpoint says of the work, told against the requirements of the item: what is meant, what is done of it and
//! what is left of it.
//!
//! The text is made here from plain facts the daemon gathers (the plan, the judgements of its steps, the commits with
//! their `Step:` trailers, the findings), so that the same facts always give the same text.

use std::collections::BTreeMap;
use std::num::NonZeroU32;

use super::CheckpointContent;
use crate::ids::ItemId;
use crate::plan::Plan;
use crate::plan::judge::JudgementState;
use crate::review_findings::Severity;
use crate::state::{FindingStatus, PrNumber};
use crate::task_list::requirements::Requirement;

#[cfg(test)]
mod tests;

/// The most lines of one list of a checkpoint that are kept; the rest is counted.
pub const LIST_LIMIT: usize = 40;

/// A commit of the holder's branch, as the checkpoint tells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitLine {
    /// The abbreviated hash of the commit.
    pub id: String,
    /// The step its `Step:` trailer names; `None` when it has none or it does not read as a step.
    pub step: Option<ItemId>,
}

/// A finding of the reviews of the round's pull request, as the checkpoint tells it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingLine {
    /// Its permanent number.
    pub number: NonZeroU32,
    /// How serious the reviewer found it.
    pub severity: Severity,
    /// Where the reviewer found it.
    pub place: String,
    /// What the reviewer said.
    pub text: String,
    /// Whether it is open, resolved or not confirmed.
    pub status: FindingStatus,
}

/// What the holder does next, which the phase of the round decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// There is no pull request yet.
    HandOver,
    /// The reviews of the pass are awaited.
    WaitForReviews {
        /// The review pass.
        pass: u32,
    },
    /// The open findings are fixed in the pull request.
    Fix {
        /// The pull request.
        pull_request: PrNumber,
    },
    /// The round waits for the human.
    WaitForHuman,
    /// The pull request is approved and waits for its merge.
    WaitForMerge,
}

/// The facts a checkpoint tells.
#[derive(Debug, Clone)]
pub struct StateInput<'a> {
    /// The task's short description.
    pub description: &'a str,
    /// The item's number.
    pub item: ItemId,
    /// The item's title.
    pub title: &'a str,
    /// The requirements of the item, numbered.
    pub requirements: &'a [Requirement],
    /// The plan of the holder; `None` when it has none that reads.
    pub plan: Option<&'a Plan>,
    /// What the harness judged of the steps that are marked done.
    pub judgements: &'a BTreeMap<ItemId, JudgementState>,
    /// The commits of the holder's branch, oldest first.
    pub commits: &'a [CommitLine],
    /// The findings of the round's pull request.
    pub findings: &'a [FindingLine],
    /// How many review passes passed.
    pub passes_passed: u32,
    /// The tip of the holder's branch, abbreviated.
    pub head: Option<&'a str>,
    /// What comes next.
    pub next: Next,
}

/// The numbers that the first line of the `done` and of the `left` part of a checkpoint carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    /// How many commits the branch has.
    pub commits: u32,
    /// How many steps are done.
    pub steps_done: u32,
    /// How many steps are left.
    pub steps_left: u32,
    /// How many findings are open.
    pub findings_open: u32,
}

/// The three parts of the checkpoint that tells `input`.
#[must_use]
pub fn checkpoint_content(input: &StateInput<'_>) -> CheckpointContent {
    let _ = input;
    CheckpointContent {
        meant: String::new(),
        done: String::new(),
        left: String::new(),
    }
}

impl CheckpointContent {
    /// The numbers in the first lines of the `done` and `left` parts; `None` for a content that has none.
    #[must_use]
    pub fn counts(&self) -> Option<Counts> {
        None
    }
}
