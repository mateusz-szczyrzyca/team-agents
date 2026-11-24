//! Whether a team waits behind a failed member whose work its lead's branch already holds.
//!
//! A lead that took the commits of a member into its own branch and ended `STATUS: DONE` with a passed gate has
//! delivered the member's item; the member that failed afterwards, or never ended, must not hold the round's
//! publication. [`delivered_step`] tells the cases apart:
//!
//! 1. Only a member whose task *failed* is looked at. A member that still runs, waits, asks or is paused is never
//!    bypassed, and one the human stopped is published when its lead is done, as before.
//! 2. The lead is done and its gate passed at the tip of its branch; before that nothing is decided.
//! 3. The member's branch holds no commit the lead's branch lacks, and every uncommitted file of its worktree is one
//!    the lead was told of: the member's task is closed as taken over and the round is published.
//! 4. Something is lacking that the lead was not told of: the lead is told, exactly what, once.
//! 5. The lead was told of every commit that is lacking, ended again, and the commits are still lacking: the human is
//!    asked, once, with the list. A lead that has not ended again since is waited for.

use crate::repo_path::RepoPath;
use crate::state::CommitId;
use crate::status::TaskStatus;

/// What the lead was told of a failed member's work, and whether it answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Told<'a> {
    /// The commits of the member's branch the lead was told it lacks.
    pub commits: &'a [CommitId],
    /// The uncommitted files of the member's worktree the lead was told of.
    pub files: &'a [RepoPath],
    /// Whether the lead ended a run with `STATUS: DONE` after it was told.
    pub answered: bool,
}

/// What is known of a failed member's work against the lead's branch at one look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delivery<'a> {
    /// The status of the member's task.
    pub member: TaskStatus,
    /// Whether the lead's last run ended with `STATUS: DONE`.
    pub lead_done: bool,
    /// Whether the lead's gate passed on a clean tree at the tip of its branch.
    pub gate_passed: bool,
    /// The commits of the member's branch that the lead's branch lacks.
    pub commits: &'a [CommitId],
    /// The files the member's worktree holds that no commit does.
    pub files: &'a [RepoPath],
    /// What the lead was told of this member's work; `None` when it was told nothing.
    pub told: Option<Told<'a>>,
    /// Whether the human was asked about this member's work.
    pub asked: bool,
}

/// What the harness does about a failed member whose item waits for its lead's delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivered {
    /// Nothing: the member is not failed, the lead is not done with a passed gate, or something waits for the lead or
    /// the human.
    Nothing,
    /// The lead's branch holds the member's work: the member's task is closed as taken over and the round goes on.
    Release,
    /// The member has work the lead's branch lacks that the lead was not told of: it is told what, once.
    TellLead,
    /// The lead ended again without the commits it was told of: the human is asked, once, with the list.
    AskHuman,
}

/// What the harness does about the failed member described by `delivery`.
#[must_use]
pub fn delivered_step(delivery: &Delivery<'_>) -> Delivered {
    if !delivery.commits.is_empty() {
        Delivered::Release
    } else if !delivery.files.is_empty() {
        Delivered::AskHuman
    } else {
        Delivered::TellLead
    }
}

#[cfg(test)]
mod tests;
