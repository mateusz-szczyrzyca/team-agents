//! One-time explanations and the durable correction budget of a fix response.
use super::{Answer, FindingStatus, Findings, Report, Severity};
use crate::ids::{InvestigatorName, RunId};
use crate::review_findings::ReportError;
use crate::review_response::{Response, ResponseError};
use crate::status::ReviewVerdict;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::num::NonZeroU32;

/// How the verification pass decided an explanation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplanationStatus {
    /// The verification pass has not completed.
    Pending,
    /// Every verifier accepted it; the finding is minor.
    Accepted,
    /// At least one verifier rejected it; a code fix is required.
    Rejected,
}

/// An explanation can be offered once, including when its verification rejects it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Explanation {
    /// The lead's justification.
    pub why: String,
    /// The pass's decision.
    pub status: ExplanationStatus,
    /// Each verifier's vote and evidence.
    pub votes: BTreeMap<InvestigatorName, Answer>,
}

/// One verifier's upward severity change and its evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaisedFinding {
    /// The permanent finding number.
    pub number: NonZeroU32,
    /// Who raised it.
    pub reviewer: InvestigatorName,
    /// The increased severity.
    pub severity: Severity,
    /// Why it was raised.
    pub why: String,
}

/// The protocol marker and durable decisions for new pull requests.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Disputes {
    /// Explanations, retained after acceptance, rejection and resolution.
    pub explanations: BTreeMap<NonZeroU32, Explanation>,
    /// Upward severity changes in verification order.
    pub raised: Vec<RaisedFinding>,
    /// The fix pass whose lead response has been accepted; zero before any fix pass.
    pub answered_pass: u32,
    /// Invalid responses since the last valid response; two corrections are allowed.
    pub bounces: u32,
    /// Last run checked, preventing repeated corrections during polling or recovery.
    pub checked_run: Option<RunId>,
    /// Rendered correction, retained for a resumed run after a daemon restart.
    pub correction: Option<String>,
}

impl Findings {
    /// The dispute protocol, absent for migrated active pull requests.
    #[must_use]
    pub const fn disputes(&self) -> Option<&Disputes> {
        self.disputes.as_ref()
    }
    /// The mutable correction state of the dispute protocol.
    pub const fn disputes_mut(&mut self) -> Option<&mut Disputes> {
        self.disputes.as_mut()
    }
    /// Reads a report with the protocol this pull request started with.
    ///
    /// # Errors
    /// [`ReportError`] for a malformed or contradictory report.
    pub fn parse_report(&self, text: &str, verdict: ReviewVerdict) -> Result<Report, ReportError> {
        if let Some(disputes) = &self.disputes {
            let pending: Vec<_> = disputes
                .explanations
                .iter()
                .filter(|(_, e)| e.status == ExplanationStatus::Pending)
                .map(|(n, _)| *n)
                .collect();
            crate::review_findings::parse_verification(text, verdict, &self.open(), &pending)
        } else {
            crate::review_findings::parse_report(text, verdict, &self.open())
        }
    }
    /// Checks and records the lead's complete response before a verification pass.
    ///
    /// # Errors
    /// [`ResponseError`] identifies the line to correct; no decisions change on failure.
    pub fn answer_lead(&mut self, _text: &str, _pass: u32) -> Result<Vec<(NonZeroU32, Response)>, ResponseError> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests;
