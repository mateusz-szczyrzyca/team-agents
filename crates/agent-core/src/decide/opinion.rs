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
    let _inputs = (findings, pass, fixed, tip);
    Next::AwaitOpinions
}

#[cfg(test)]
mod tests;
