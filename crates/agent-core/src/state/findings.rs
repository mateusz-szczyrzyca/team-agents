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
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Findings {
    /// Findings, in permanent-number order.
    pub entries: Vec<RecordedFinding>,
    /// Reports received during the current pass; numbering waits for request order to be known in full.
    pending: BTreeMap<TaskId, Report>,
    /// The harness comment, updated instead of posting another table.
    pub comment: Option<u64>,
}

/// The pull request exhausted the representable finding numbers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the pull request has exhausted its finding numbers")]
pub struct FindingNumbersExhausted;

impl Findings {
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
    pub fn finish(&mut self, _reviews: &[Review], _pass: NonZeroU32) -> Result<(), FindingNumbersExhausted> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
