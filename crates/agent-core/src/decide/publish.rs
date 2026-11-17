//! Whether the branch of a lead that waits is published, gated first, or sent back.
//!
//! A branch is published only when the repository's gate passed on a clean tree at its tip. The agent may have run the
//! gate itself; when it ended with `STATUS: DONE` and the record does not say so, the harness runs the gate instead of
//! waiting for one that nobody runs, and a gate that does not pass sends the lead back with the reason.
//!
//! [`gate_step`] decides in this order:
//!
//! 1. A lead that did not end with `STATUS: DONE` (it sleeps, waiting for its team) is not done: its branch is
//!    published when a record passes at the tip — the gate it ran itself — and otherwise waits.
//! 2. A branch without a commit of its own has nothing to publish: the lead is sent back.
//! 3. The record is of the tip: a pass publishes, anything else sends the lead back with what the gate said.
//! 4. The record is of another commit, or there is none: the harness runs the gate — once for a tip. When the gate it
//!    ran for this tip left no record of the tip, the worktree has another commit out: the lead is sent back.

use std::fmt;

use crate::config::AbsolutePath;
use crate::state::{CommitId, GateRecord};

/// What is known of the branch of a lead that waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Waiting<'a> {
    /// The tip of the branch.
    pub tip: &'a CommitId,
    /// Whether the lead's last run ended with `STATUS: DONE`.
    pub lead_done: bool,
    /// Whether the branch has a commit that its base branch does not have.
    pub own_work: bool,
    /// The last gate record of the lead's task.
    pub record: Option<&'a GateRecord>,
    /// Whether the harness already ran the gate for this tip.
    pub gated_for_tip: bool,
    /// Whether the harness already ran the gate for this tip once more, after a gate had not passed at it.
    pub retried_for_tip: bool,
}

/// What the harness does with the branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateStep {
    /// The gate passed at the tip on a clean tree: the branch is published.
    Publish,
    /// Nothing is done: the lead is not done and no gate passed at the tip.
    Wait,
    /// The harness runs the gate at the tip.
    Run,
    /// A gate failed or timed out at the tip: the harness runs it once more before it sends the lead back, because a
    /// check that fails for a reason outside the change (a test that depends on the load of the machine) is no reason
    /// to wake the lead, and a lead that changed nothing can do no more than run it again.
    Retry,
    /// The lead is sent back.
    SendBack(GateHold),
}

/// Why the branch of a lead that is done is not published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateHold {
    /// The branch has no commit of its own.
    NoCommit,
    /// The gate ran at the tip and did not pass, or passed on a tree that was not clean.
    NotPassed {
        /// What the gate said, in a few words ([`GateRecord::summary`]).
        summary: String,
        /// The file with the gate's output.
        log: AbsolutePath,
    },
    /// The harness ran the gate for the tip, and the gate ran on another commit: the worktree does not have the tip out.
    NotAtTip {
        /// The tip of the branch.
        tip: CommitId,
        /// The commit the gate ran on, if it left a record.
        gated: Option<CommitId>,
    },
}

impl fmt::Display for GateHold {
    /// The reason as the lead and the human are told it: a sentence that stands after "the gate has not passed:".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCommit => f.write_str("the branch has no commit of its own, so there is nothing to publish."),
            Self::NotPassed { summary, log } => {
                write!(
                    f,
                    "the gate {summary}; its whole output is in {}.",
                    log.as_path().display()
                )
            }
            Self::NotAtTip {
                tip,
                gated: Some(gated),
            } => write!(
                f,
                "the harness ran the gate and it ran on commit {gated}, not on the tip {tip} of the branch: your \
                 worktree has another commit out than the branch that is published."
            ),
            Self::NotAtTip { tip, gated: None } => write!(
                f,
                "the harness ran the gate and it left no record of the tip {tip} of the branch: your worktree does \
                 not have the branch that is published out."
            ),
        }
    }
}

/// What the harness does with the branch `waiting` describes.
#[must_use]
pub fn gate_step(waiting: &Waiting<'_>) -> GateStep {
    let at_tip = waiting.record.filter(|record| record.result.head == *waiting.tip);
    if !waiting.lead_done {
        return match at_tip {
            Some(record) if record.counts_as_pass() => GateStep::Publish,
            Some(_) | None => GateStep::Wait,
        };
    }
    if !waiting.own_work {
        return GateStep::SendBack(GateHold::NoCommit);
    }
    match at_tip {
        Some(record) if record.counts_as_pass() => GateStep::Publish,
        Some(record) => GateStep::SendBack(GateHold::NotPassed {
            summary: record.summary(),
            log: record.result.log.clone(),
        }),
        None if waiting.gated_for_tip => GateStep::SendBack(GateHold::NotAtTip {
            tip: waiting.tip.clone(),
            gated: waiting.record.map(|record| record.result.head.clone()),
        }),
        None => GateStep::Run,
    }
}

#[cfg(test)]
mod tests;
