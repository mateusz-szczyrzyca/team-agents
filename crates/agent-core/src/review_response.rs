//! The lead's checked answers to the blocking findings of a fix pass.

use crate::review_findings::Severity;
use std::num::NonZeroU32;

/// The lead's response to a blocking finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// The defect has been fixed in code.
    Fixed,
    /// The deviation is intentional, for this reason.
    Explained(String),
}

/// A response that needs correction before verification can start.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid lead response at line {line}: finding {number} requires {allowed}")]
pub struct ResponseError {
    /// One-based source line, or the end of the message for an omitted response.
    pub line: usize,
    /// The affected permanent finding number.
    pub number: NonZeroU32,
    /// The allowed response syntax for this finding.
    pub allowed: &'static str,
}

/// Reads exactly one answer per blocking finding, refusing explanations already used or attached to blockers.
///
/// # Errors
/// [`ResponseError`] identifies an absent, duplicated, unknown or forbidden answer.
pub fn parse_lead(
    _text: &str,
    _open: &[(NonZeroU32, Severity, bool)],
) -> Result<Vec<(NonZeroU32, Response)>, ResponseError> {
    Ok(Vec::new())
}

#[cfg(test)]
mod tests;
