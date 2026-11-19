//! The answers a verifier may give for each open finding: one list, from which the brief names them and by which the
//! report is read, so that the two cannot disagree.

use super::{ExplanationStatus, FindingStatus, Findings};
use crate::review_findings::Severity;
use std::num::NonZeroU32;

/// What a verifier decides about one open finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The lead's explanation waits for a decision: `ACCEPTED — why` or `REJECTED — why`.
    Explanation,
    /// The finding itself: `RESOLVED` or `OPEN — why`.
    Outcome,
}

/// Whether, and how far, a verifier may raise the severity of one open finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Raise {
    /// Not at all: a blocker, an accepted explanation whose file did not change, or a pull request of the protocol
    /// before explanations.
    Never,
    /// `RAISED [BLOCKER] — why`.
    ToBlocker,
    /// `RAISED [MAJOR] — why` or `RAISED [BLOCKER] — why`.
    ToMajorOrBlocker,
}

/// The answers a verifier may give for one open finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifierAnswers {
    /// The permanent number of the finding.
    pub number: NonZeroU32,
    /// What is decided.
    pub decision: Decision,
    /// Whether its severity may be raised.
    pub raise: Raise,
}

impl Findings {
    /// The answers a verifier may give, for every open finding in number order.
    #[must_use]
    pub fn verifier_answers(&self) -> Vec<VerifierAnswers> {
        self.entries
            .iter()
            .filter(|entry| entry.status == FindingStatus::Open)
            .map(|entry| {
                let explanation = self
                    .disputes
                    .as_ref()
                    .and_then(|disputes| disputes.explanations.get(&entry.number))
                    .map(|explanation| explanation.status);
                let decision = if explanation == Some(ExplanationStatus::Pending) {
                    Decision::Explanation
                } else {
                    Decision::Outcome
                };
                // An accepted explanation is protected while its code is unchanged.
                let protected = explanation == Some(ExplanationStatus::Accepted)
                    && !self
                        .disputes
                        .as_ref()
                        .is_some_and(|disputes| disputes.changed_findings.contains(&entry.number));
                let raise = match (&self.disputes, entry.finding.severity) {
                    (None, _) | (Some(_), Severity::Blocker) => Raise::Never,
                    (Some(_), _) if protected => Raise::Never,
                    (Some(_), Severity::Major) => Raise::ToBlocker,
                    (Some(_), Severity::Minor) => Raise::ToMajorOrBlocker,
                };
                VerifierAnswers {
                    number: entry.number,
                    decision,
                    raise,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
