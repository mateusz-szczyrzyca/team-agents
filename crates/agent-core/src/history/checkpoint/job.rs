//! What a checkpoint says of a job that is not an item of a round: a worker's request, a review and an investigation.
//!
//! The three parts are made here from plain facts the daemon gathers, as [`super::state`] does for an item: what is
//! meant is the request, the pull request under review or the question; what is done is what the worktree holds; what is
//! left is what the job still has to do, and `next:` says what that is.

use super::{CheckpointContent, CommitLine};
use crate::ids::ItemId;
use crate::state::PrNumber;
use crate::task_list::requirements::Requirement;

/// The result of the check of a worker's request, as it last ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckResult<'a> {
    /// Whether the check passed.
    pub passed: bool,
    /// The line of the check's output that says it.
    pub line: &'a str,
}

/// The item a review is against.
#[derive(Debug, Clone, Copy)]
pub struct ReviewedItem<'a> {
    /// The item.
    pub id: ItemId,
    /// Its title.
    pub title: &'a str,
    /// Its requirements, numbered.
    pub requirements: &'a [Requirement],
}

/// What a job has done so far, which its worktree and its files say.
#[derive(Debug, Clone, Copy)]
pub enum JobWork<'a> {
    /// A worker's request.
    Request {
        /// How its check last ran; `None` when it did not run yet or the request names none.
        last_check: Option<CheckResult<'a>>,
        /// The commits of the request's worktree since it was made.
        commits: &'a [CommitLine],
        /// The files that differ from where the worktree started.
        changed: &'a [String],
    },
    /// A review.
    Review {
        /// The review pass.
        pass: u32,
        /// The findings written so far, as the reviewer's findings file has them; empty when it has none.
        findings: &'a str,
    },
    /// An investigation.
    Investigation {
        /// The commits of the investigator's worktree since it was made.
        commits: &'a [CommitLine],
        /// The files that differ from where the worktree started: the investigator's notes and changes.
        changed: &'a [String],
    },
}

/// What is meant by a worker's request: its text, and its check when it names one.
#[must_use]
pub fn request_meant(request: &str, check: Option<&str>) -> String {
    let _ = (request, check);
    String::new()
}

/// What is meant by a review: the pull request at its head and the pass, against the requirements of its item.
#[must_use]
pub fn review_meant(pull_request: PrNumber, head: &str, pass: u32, item: &ReviewedItem<'_>) -> String {
    let _ = (pull_request, head, pass, item);
    String::new()
}

/// What is meant by an investigation: the question of the request.
#[must_use]
pub fn investigation_meant(question: &str) -> String {
    let _ = question;
    String::new()
}

/// The three parts of the checkpoint of a job whose `meant` part is `meant`, with `work` done so far.
#[must_use]
pub fn job_content(meant: String, work: &JobWork<'_>) -> CheckpointContent {
    let _ = work;
    CheckpointContent {
        meant,
        done: String::new(),
        left: String::new(),
    }
}

#[cfg(test)]
mod tests;
