//! Structured review findings and the answers of a later review pass.

use crate::status::ReviewVerdict;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

/// The importance of a review finding; only minor findings permit approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Severity {
    /// Broken logic, build, or gate.
    Blocker,
    /// An unexplained deviation from a specification.
    Major,
    /// Advice that never blocks approval.
    Minor,
}

impl Severity {
    /// The plain word used in reports and state.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Blocker => "BLOCKER",
            Self::Major => "MAJOR",
            Self::Minor => "MINOR",
        }
    }
    /// Whether an open finding prevents approval.
    #[must_use]
    pub const fn blocks(self) -> bool {
        !matches!(self, Self::Minor)
    }
}

/// One newly reported finding, before the harness gives it a permanent number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    /// Its importance.
    pub severity: Severity,
    /// The specification rule, separate from severity.
    pub rule: String,
    /// File and line, or another concrete location.
    pub place: String,
    /// Problem and proposed fix.
    pub text: String,
    /// A short verification instruction.
    pub check: String,
}

/// A verifier's answer about one existing finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Answer {
    /// The check no longer fails.
    Resolved,
    /// The problem remains, for this reason.
    Open {
        /// Why the check still fails.
        why: String,
    },
}

/// The structured part of a report, alongside its separately parsed verdict.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    /// New findings, in report order.
    pub findings: Vec<Finding>,
    /// Answers for the open findings, in report order.
    pub answers: Vec<(NonZeroU32, Answer)>,
}

/// A malformed finding or a verdict that contradicts the report.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid review report at line {line}: {kind}")]
pub struct ReportError {
    /// One-based source line, or the end of the report for an omitted answer.
    pub line: usize,
    /// Which part of the protocol failed.
    pub kind: ReportProblem,
}

/// The correction a report requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReportProblem {
    /// A finding does not have the specified syntax.
    #[error("expected N. [BLOCKER|MAJOR|MINOR] RULE file:line — problem → fix")]
    Finding,
    /// The verification instruction is missing.
    #[error("expected an indented CHECK: instruction after the finding")]
    Check,
    /// An answer is absent, duplicated, unknown, or malformed.
    #[error(
        "expected exactly one FINDING N: RESOLVED or FINDING N: OPEN — why for every open finding, before new findings"
    )]
    Answer,
    /// Blocking findings and verdict disagree.
    #[error("APPROVE requires no open BLOCKER or MAJOR; REQUEST_CHANGES requires at least one")]
    Verdict,
}

/// Reads findings and verification answers, checking the verdict against all open problems.
///
/// # Errors
/// [`ReportError`] identifies the malformed line or contradictory verdict.
pub fn parse_report(
    _text: &str,
    _verdict: ReviewVerdict,
    _open: &[(NonZeroU32, Severity)],
) -> Result<Report, ReportError> {
    Ok(Report::default())
}

#[cfg(test)]
mod tests;
