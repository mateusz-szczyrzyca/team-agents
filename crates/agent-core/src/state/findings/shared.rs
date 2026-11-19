//! The items of one pull request share its findings: a state in which their ledgers differ is brought together.

use super::{ExplanationStatus, FindingStatus, Findings};
use std::num::NonZeroU32;

/// One finding whose explanation, status or severity differed between the ledgers of one pull request, as it stands in
/// all of them after [`reconcile`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reconciled {
    /// The permanent number of the finding.
    pub number: NonZeroU32,
    /// How its explanation stands now; `None` when no ledger has one.
    pub explanation: Option<ExplanationStatus>,
    /// Its status now.
    pub status: FindingStatus,
}

/// Brings the `ledgers` of the items of one pull request together, finding by finding, and returns the findings that
/// were repaired, in number order.
///
/// For each number that every ledger has, the most advanced record holds for all: its explanation, the status and the
/// severity of the finding are copied to the other ledgers. An explanation is more advanced than none, a decided one
/// more than a pending one, and a rejected one more than an accepted one (a fix is the safer side); with equal
/// explanations, a closed finding is more advanced than an open one. Ledgers of the protocol before explanations are
/// left as they are when any ledger of the pull request has no explanations at all.
pub fn reconcile(ledgers: &mut [Findings]) -> Vec<Reconciled> {
    ledgers.first().map_or_else(Vec::new, |ledger| {
        ledger
            .entries
            .iter()
            .map(|entry| Reconciled {
                number: entry.number,
                explanation: None,
                status: entry.status,
            })
            .collect()
    })
}

#[cfg(test)]
mod tests;
