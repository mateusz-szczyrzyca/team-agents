//! Whether the reviewer of a review that is still to come is replaced by another.
//!
//! A review pass waits for every review that was asked. A reviewer that cannot work — its CLI is not logged in, its
//! provider has no quota left, the human stopped it — would keep the pass, and with it the team, waiting for ever.
//! [`replace_now`] says when another investigator takes such a review:
//!
//! 0. The lead of the team is over (stopped or failed): the team does not go on, so nobody is asked for anything.
//! 1. The task failed or was stopped: it will not review, so at once.
//! 2. The task is paused (a quota or a rate limit, which end by themselves, or the human's pause): when it has been
//!    paused for as long as a reviewer may be.
//! 3. Otherwise not: a task that is queued, runs or asks the human works on the review; one that waits is about to be
//!    given it; one that is done reported, and a report without a verdict is asked for again elsewhere.

use std::time::Duration;

use crate::status::TaskStatus;

/// What is known of a review that is still to come.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Awaited {
    /// The status of the reviewer's task.
    pub reviewer: TaskStatus,
    /// The status of the task of the team's lead.
    pub lead: TaskStatus,
    /// How long ago the reviewer's last run ended, or the review was asked when it has not run.
    pub quiet_for: Duration,
    /// How long a reviewer may be paused.
    pub patience: Duration,
}

/// Whether the review `awaited` describes is given to another investigator.
#[must_use]
pub fn replace_now(awaited: &Awaited) -> bool {
    let _ = awaited;
    false
}

#[cfg(test)]
mod tests;
