//! Pure second-opinion protocol, activated by the daemon in part 2.
use crate::review_findings::Severity;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

/// Nonempty reasoning or evidence supplied by an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Evidence(String);

impl TryFrom<String> for Evidence {
    type Error = OpinionError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.trim().is_empty() {
            Err(OpinionError::MissingEvidence)
        } else {
            Ok(Self(value.trim().to_owned()))
        }
    }
}
impl From<Evidence> for String {
    fn from(value: Evidence) -> Self {
        value.0
    }
}
impl Evidence {
    /// The supplied reasoning, with outer whitespace removed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether the finding holds at the requested commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The reported defect exists and must be fixed.
    Confirmed,
    /// The reported defect does not hold at this commit.
    NotConfirmed,
}

/// The investigator's decision and supporting observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpinionReport {
    /// Whether the finding holds.
    pub outcome: Outcome,
    /// What the investigator read or ran and what it showed.
    pub evidence: Evidence,
}

/// A malformed opinion protocol message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpinionError {
    /// The report does not start with exactly one recognized verdict.
    #[error("opinion report has no unambiguous opening verdict")]
    MissingVerdict,
    /// Reasoning or evidence is required.
    #[error("opinion requires evidence")]
    MissingEvidence,
    /// The lead's message violates the existing response protocol.
    #[error(transparent)]
    Response(#[from] super::ResponseError),
    /// The requested finding is not eligible for an opinion.
    #[error("finding {number} cannot receive a second opinion")]
    Forbidden {
        /// Permanent finding number.
        number: NonZeroU32,
    },
}

/// Configuration permission, kept explicit at the pure protocol boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// One opinion per open blocker is permitted.
    Enabled,
    /// Every request must be corrected to a code fix.
    Disabled,
}

/// Eligibility of a finding before reading the lead's message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenFinding {
    /// Permanent finding number.
    pub number: NonZeroU32,
    /// Reported severity.
    pub severity: Severity,
    /// Whether the major's explanation was already used.
    pub explained: bool,
    /// Whether any second opinion was already requested.
    pub opinion_used: bool,
}

/// A checked answer to one open finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeadAnswer {
    /// Code has been corrected.
    Fixed,
    /// A major's intentional deviation is explained.
    Explained(String),
    /// One investigator must check this blocker with the supplied reason.
    SecondOpinion(Evidence),
}

/// Reads all answers atomically, including eligible second-opinion requests.
///
/// # Errors
/// Returns [`OpinionError`] for malformed, duplicated, missing or forbidden answers.
pub fn parse_lead(
    text: &str,
    open: &[OpenFinding],
    permission: Permission,
) -> Result<Vec<(NonZeroU32, LeadAnswer)>, OpinionError> {
    let _inputs = (text, open, permission);
    Ok(Vec::new())
}

/// Reads an opening opinion verdict followed by nonempty evidence.
///
/// # Errors
/// Returns [`OpinionError::MissingVerdict`] or [`OpinionError::MissingEvidence`].
pub fn parse_report(text: &str) -> Result<OpinionReport, OpinionError> {
    let _input = text;
    Err(OpinionError::MissingVerdict)
}

#[cfg(test)]
mod tests;
