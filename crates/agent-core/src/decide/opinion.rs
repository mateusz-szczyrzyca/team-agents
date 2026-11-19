//! Pure continuation of a fix pass after all its second opinions.
use crate::review_response::opinion::Outcome;
use crate::state::{Findings, OpinionStage};
use std::collections::BTreeSet;
use std::num::NonZeroU32;

/// Whether the lead handed over a commit different from the reviewed head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tip {
    /// No new code was handed over.
    Unchanged,
    /// A new commit is ready for verification.
    Changed,
}

/// What the daemon must do next, without consuming another fix pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// At least one requested opinion has not been accepted yet.
    AwaitOpinions,
    /// Return confirmed findings to the lead for code fixes in this same pass.
    FixConfirmed(Vec<NonZeroU32>),
    /// Wait for the lead's new tip before requesting verification.
    AwaitTip,
    /// Every opinion is settled and the handed-over tip may be verified.
    Verify,
    /// All blocking findings were not confirmed and no code changed: decide the pass without new reviews.
    DecideUnchanged,
}

/// Decides continuation for one fix pass, considering only opinions requested in that pass.
/// `fixed` contains the confirmed findings the lead has subsequently answered `FIXED`.
#[must_use]
pub fn after_opinions(findings: &Findings, pass: NonZeroU32, fixed: &BTreeSet<NonZeroU32>, tip: Tip) -> Next {
    let opinions: Vec<_> = findings
        .opinions()
        .iter()
        .filter(|(_, opinion)| opinion.request.pass == pass)
        .collect();
    if opinions
        .iter()
        .any(|(_, opinion)| matches!(opinion.stage, OpinionStage::Waiting | OpinionStage::Assigned { .. }))
    {
        return Next::AwaitOpinions;
    }
    let confirmed: Vec<_> = opinions
        .iter()
        .filter_map(|(number, opinion)| match &opinion.stage {
            OpinionStage::Reported { report, .. }
                if report.outcome == Outcome::Confirmed && !fixed.contains(number) =>
            {
                Some(**number)
            }
            OpinionStage::Waiting | OpinionStage::Assigned { .. } | OpinionStage::Reported { .. } => None,
        })
        .collect();
    if !confirmed.is_empty() {
        return Next::FixConfirmed(confirmed);
    }
    match tip {
        Tip::Changed => Next::Verify,
        Tip::Unchanged if !opinions.is_empty() && !findings.open().iter().any(|(_, severity)| severity.blocks()) => {
            Next::DecideUnchanged
        }
        Tip::Unchanged => Next::AwaitTip,
    }
}

/// Whether an unsuccessful assignment is replaced or escalated to the human.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unanswered {
    /// A free independent investigator may take the request.
    Replace,
    /// Three investigators were asked; keep the assignment for the one-shot notice.
    Hold,
}

/// Bounds automatic replacement to two, regardless of how each run ended.
#[must_use]
pub fn unanswered(_failure: crate::state::OpinionFailure, _attempts: usize) -> Unanswered {
    Unanswered::Hold
}

#[cfg(test)]
mod tests;
