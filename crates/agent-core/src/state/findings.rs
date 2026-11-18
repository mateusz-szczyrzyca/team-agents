//! The permanent findings ledger of one pull request, including reports waiting for their peers.

use super::Review;
use crate::ids::{InvestigatorName, TaskId};
use crate::review_findings::{Answer, Finding, Report, Severity};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::num::NonZeroU32;

/// Whether all verifiers have accepted a finding's fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    /// At least one verifier has not accepted the fix.
    Open,
    /// Every verifier of a pass accepted the fix.
    Resolved,
}

/// A finding with its permanent identity and provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedFinding {
    /// Permanent number, counting from one over the pull request.
    pub number: NonZeroU32,
    /// The finding as it was reported.
    pub finding: Finding,
    /// Who first reported it.
    pub reporter: InvestigatorName,
    /// Review pass, counting from one.
    pub pass: NonZeroU32,
    /// Whether it is still open.
    pub status: FindingStatus,
}

/// A pull request's findings; absent on rounds started under the old review protocol.
///
/// The permanent numbers of `entries` are exactly 1, 2, … in order: [`Findings::finish`] gives the next one by counting,
/// so a ledger read back with a duplicate, a gap or another order is refused.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "FindingsWire")]
pub struct Findings {
    /// The independent protocol's pass size; absent on pull requests opened before its introduction.
    #[serde(skip_serializing_if = "Option::is_none")]
    review_count: Option<crate::config::ReviewCount>,
    /// Verification context, absent on PRs opened before rotating verifications.
    #[serde(skip_serializing_if = "Option::is_none")]
    verification: Option<Verification>,
    /// Findings, in permanent-number order; private so that only [`Findings::new`], the loader and
    /// [`Findings::finish`] can set the numbers.
    entries: Vec<RecordedFinding>,
    /// Reports received during the current pass; numbering waits for request order to be known in full.
    pending: BTreeMap<TaskId, Report>,
    /// The harness comment, updated instead of posting another table.
    pub comment: Option<u64>,
}

/// How a [`Findings`] ledger is read, before its numbering is checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FindingsWire {
    /// See [`Findings::review_count`]; absence retains the previous protocol.
    #[serde(default)]
    review_count: Option<crate::config::ReviewCount>,
    /// See [`Findings::entries`].
    entries: Vec<RecordedFinding>,
    /// See [`Findings::verification`]; missing retains the previous protocol.
    #[serde(default)]
    verification: Option<Verification>,
    /// See [`Findings`]; absent when no report waits.
    #[serde(default)]
    pending: BTreeMap<TaskId, Report>,
    /// See [`Findings::comment`].
    #[serde(default)]
    comment: Option<u64>,
}

/// A ledger whose permanent numbers are not 1, 2, … in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("finding number {found} stands where {expected} must: numbers count from 1 in order")]
pub struct InvalidFindingNumbers {
    /// The number the position requires.
    pub expected: u32,
    /// The number the entry has.
    pub found: u32,
}

impl TryFrom<FindingsWire> for Findings {
    type Error = InvalidFindingNumbers;

    fn try_from(wire: FindingsWire) -> Result<Self, Self::Error> {
        let mut ledger = Self::new(wire.entries)?;
        ledger.pending = wire.pending;
        ledger.comment = wire.comment;
        ledger.review_count = wire.review_count;
        ledger.verification = wire.verification;
        Ok(ledger)
    }
}

/// The pull request exhausted the representable finding numbers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FindingNumbersExhausted {
    /// The total cannot be represented by a finding number.
    #[error("the pull request has too many findings")]
    Capacity(#[from] std::num::TryFromIntError),
    /// Adding a number would overflow.
    #[error("the pull request has exhausted its finding numbers")]
    Exhausted,
}

/// The context retained across fix passes for rotating verification reviews.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verification {
    /// The head of the last completed review pass, before the lead's fixes.
    pub reviewed_head: Option<super::CommitId>,
    /// The lead's final message of the latest fix pass.
    pub lead_message: Option<String>,
    /// Additional fix passes granted by the human beyond the configured initial limit.
    pub extra_fix_passes: u32,
}

impl Findings {
    /// Starts independent reviews followed by rotating verification passes.
    #[must_use]
    pub fn verifying(count: crate::config::ReviewCount) -> Self {
        Self::independent(count)
    }

    /// The context of the rotating verification protocol, absent for legacy PRs.
    #[must_use]
    pub const fn verification(&self) -> Option<&Verification> {
        self.verification.as_ref()
    }

    /// The mutable verification context of a PR using the rotating protocol.
    pub const fn verification_mut(&mut self) -> Option<&mut Verification> {
        self.verification.as_mut()
    }

    /// Starts an independent review protocol with the configured pass size.
    #[must_use]
    pub fn independent(count: crate::config::ReviewCount) -> Self {
        Self {
            review_count: Some(count),
            ..Self::default()
        }
    }

    /// The pass size frozen at publication; absent for rounds using the previous protocol.
    #[must_use]
    pub const fn review_count(&self) -> Option<crate::config::ReviewCount> {
        self.review_count
    }

    /// A ledger holding `entries`, with no report waiting and no comment yet.
    ///
    /// # Errors
    /// [`InvalidFindingNumbers`] when the entries are not numbered 1, 2, … in order.
    pub fn new(entries: Vec<RecordedFinding>) -> Result<Self, InvalidFindingNumbers> {
        for (index, entry) in entries.iter().enumerate() {
            let expected = u32::try_from(index).map_or(u32::MAX, |n| n.saturating_add(1));
            if entry.number.get() != expected {
                return Err(InvalidFindingNumbers {
                    expected,
                    found: entry.number.get(),
                });
            }
        }
        Ok(Self {
            review_count: None,
            verification: None,
            entries,
            pending: BTreeMap::new(),
            comment: None,
        })
    }

    /// The findings, in permanent-number order.
    #[must_use]
    pub fn entries(&self) -> &[RecordedFinding] {
        &self.entries
    }

    /// Open findings and their severities, for validating verification answers.
    #[must_use]
    pub fn open(&self) -> Vec<(NonZeroU32, Severity)> {
        self.entries
            .iter()
            .filter(|entry| entry.status == FindingStatus::Open)
            .map(|entry| (entry.number, entry.finding.severity))
            .collect()
    }

    /// Keeps a report until all reviewers have answered; the owner accepts a task only once per pass.
    pub fn record(&mut self, task: TaskId, report: Report) {
        self.pending.insert(task, report);
    }

    /// Resolves unanimously accepted fixes and numbers new findings in request order, then report order.
    ///
    /// # Errors
    /// [`FindingNumbersExhausted`] if a new finding has no representable number.
    pub fn finish(&mut self, reviews: &[Review], pass: NonZeroU32) -> Result<(), FindingNumbersExhausted> {
        // Validate capacity before changing any entry, including resolution status.
        let added = reviews
            .iter()
            .filter_map(|review| self.pending.get(&review.task))
            .map(|report| report.findings.len())
            .try_fold(self.entries.len(), usize::checked_add)
            .ok_or(FindingNumbersExhausted::Exhausted)?;
        u32::try_from(added).map_err(FindingNumbersExhausted::Capacity)?;
        for entry in &mut self.entries {
            if !reviews.is_empty()
                && reviews.iter().all(|review| {
                    self.pending.get(&review.task).is_some_and(|report| {
                        report
                            .answers
                            .iter()
                            .any(|(n, answer)| *n == entry.number && *answer == Answer::Resolved)
                    })
                })
            {
                entry.status = FindingStatus::Resolved;
            }
        }
        for review in reviews {
            if let Some(report) = self.pending.remove(&review.task) {
                for finding in report.findings {
                    let number = u32::try_from(self.entries.len())
                        .map_err(FindingNumbersExhausted::from)?
                        .checked_add(1)
                        .and_then(NonZeroU32::new)
                        .ok_or(FindingNumbersExhausted::Exhausted)?;
                    self.entries.push(RecordedFinding {
                        number,
                        finding,
                        reporter: review.reviewer.clone(),
                        pass,
                        status: FindingStatus::Open,
                    });
                }
            }
        }
        self.pending.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
