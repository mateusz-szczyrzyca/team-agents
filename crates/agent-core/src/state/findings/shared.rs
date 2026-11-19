//! The items of one pull request share its findings: a state in which their ledgers differ is brought together.

use super::{Explanation, ExplanationStatus, FindingStatus, Findings};
use crate::review_findings::Severity;
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
/// explanations, a closed finding is more advanced than an open one, and then a higher severity than a lower one.
/// Ledgers of the protocol before explanations are left as they are when any ledger of the pull request has no
/// explanations at all.
pub fn reconcile(ledgers: &mut [Findings]) -> Vec<Reconciled> {
    let Some((first, others)) = ledgers.split_first() else {
        return Vec::new();
    };
    if others.is_empty() || ledgers.iter().any(|ledger| ledger.disputes.is_none()) {
        return Vec::new();
    }
    let numbers: Vec<NonZeroU32> = first.entries.iter().map(|entry| entry.number).collect();
    let mut repaired = Vec::new();
    for number in numbers {
        let Some(records) = ledgers
            .iter()
            .map(|ledger| Record::of(ledger, number))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let Some(best) = records.iter().max_by_key(|record| record.rank()).cloned() else {
            continue;
        };
        if records.iter().all(|record| *record == best) {
            continue;
        }
        for ledger in ledgers.iter_mut() {
            best.apply(ledger, number);
        }
        repaired.push(Reconciled {
            number,
            explanation: best.explanation.as_ref().map(|explanation| explanation.status),
            status: best.status,
        });
    }
    repaired
}

/// What one ledger holds of one finding, as far as the items of a pull request must agree on it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Record {
    /// The lead's explanation and its decision; `None` when there is none.
    explanation: Option<Explanation>,
    /// Whether the finding is open.
    status: FindingStatus,
    /// Its severity, lowered by an accepted explanation.
    severity: Severity,
}

impl Record {
    /// The record of finding `number` in `ledger`; `None` when the ledger has no such finding.
    fn of(ledger: &Findings, number: NonZeroU32) -> Option<Self> {
        let entry = ledger.entries.iter().find(|entry| entry.number == number)?;
        Some(Self {
            explanation: ledger
                .disputes
                .as_ref()
                .and_then(|disputes| disputes.explanations.get(&number))
                .cloned(),
            status: entry.status,
            severity: entry.finding.severity,
        })
    }

    /// How far the record has come, by the order [`reconcile`] states: the highest rank holds for all ledgers.
    fn rank(&self) -> (u8, u8, u8) {
        let explanation = match self.explanation.as_ref().map(|explanation| explanation.status) {
            None => 0,
            Some(ExplanationStatus::Pending) => 1,
            Some(ExplanationStatus::Accepted) => 2,
            Some(ExplanationStatus::Rejected) => 3,
        };
        let status = match self.status {
            FindingStatus::Open => 0,
            FindingStatus::NotConfirmed => 1,
            FindingStatus::Resolved => 2,
        };
        let severity = match self.severity {
            Severity::Minor => 0,
            Severity::Major => 1,
            Severity::Blocker => 2,
        };
        (explanation, status, severity)
    }

    /// Writes the record into `ledger` as finding `number`'s.
    fn apply(&self, ledger: &mut Findings, number: NonZeroU32) {
        if let Some(entry) = ledger.entries.iter_mut().find(|entry| entry.number == number) {
            entry.status = self.status;
            entry.finding.severity = self.severity;
        }
        if let Some(disputes) = ledger.disputes.as_mut() {
            match &self.explanation {
                Some(explanation) => {
                    disputes.explanations.insert(number, explanation.clone());
                }
                None => {
                    disputes.explanations.remove(&number);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
