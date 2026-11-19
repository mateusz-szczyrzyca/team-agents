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

/// The protocol rule a lead response must satisfy; instruction text belongs to the harness catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Correction {
    /// An unknown number must be replaced by the supplied blocking findings.
    OnePerFinding,
    /// An unexplained major accepts a fix or one explanation.
    FixedOrExplained,
    /// A blocker or previously explained major requires a code fix.
    FixedOnly,
}

/// A response that needs correction before verification can start.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid lead response at line {line} for finding {number}: {correction:?}")]
pub struct ResponseError {
    /// One-based source line, or the end of the message for an omitted response.
    pub line: usize,
    /// The affected permanent finding number.
    pub number: NonZeroU32,
    /// The typed correction the harness must render.
    pub correction: Correction,
}

/// Reads exactly one answer per blocking finding, refusing explanations already used or attached to blockers.
///
/// # Errors
/// [`ResponseError`] identifies an absent, duplicated, unknown or forbidden answer.
pub fn parse_lead(
    text: &str,
    open: &[(NonZeroU32, Severity, bool)],
) -> Result<Vec<(NonZeroU32, Response)>, ResponseError> {
    let mut answers = Vec::new();
    let mut ended = false;
    let mut fence = None;
    for (index, line) in text.lines().enumerate() {
        match (fence, crate::review_findings::fence_marker(line)) {
            (None, Some(open)) => {
                fence = Some(open);
                continue;
            }
            (Some(open), Some(marker)) if marker.closes(open) => {
                fence = None;
                continue;
            }
            (Some(_), _) => continue,
            (None, None) => {}
        }
        let line = line.trim();
        if line.starts_with("STATUS:") {
            ended = true;
        }
        let Some(value) = line.strip_prefix("FINDING ") else {
            continue;
        };
        let found = value.split_once(':').and_then(|(number, value)| {
            open.iter()
                .find(|(id, _, _)| id.to_string() == number.trim())
                .map(|entry| (entry, value.trim()))
        });
        let Some(((number, severity, used), value)) = found else {
            return Err(ResponseError {
                line: index.saturating_add(1),
                number: open.first().map_or(NonZeroU32::MIN, |entry| entry.0),
                correction: Correction::OnePerFinding,
            });
        };
        let correction = if *severity == Severity::Major && !used {
            Correction::FixedOrExplained
        } else {
            Correction::FixedOnly
        };
        let error = || ResponseError {
            line: index.saturating_add(1),
            number: *number,
            correction,
        };
        if ended || answers.iter().any(|(id, _)| id == number) {
            return Err(error());
        }
        if *severity == Severity::Minor && !crate::review_findings::accepts_reason(value, "FIXED") {
            continue;
        }
        let response = if crate::review_findings::accepts_reason(value, "FIXED") {
            Response::Fixed
        } else if *severity == Severity::Major && !used {
            let why = value
                .strip_prefix("EXPLAINED — ")
                .filter(|why| !why.trim().is_empty())
                .ok_or_else(error)?;
            Response::Explained(why.trim().to_owned())
        } else {
            return Err(error());
        };
        answers.push((*number, response));
    }
    for (number, severity, used) in open {
        if severity.blocks() && !answers.iter().any(|(id, _)| id == number) {
            return Err(ResponseError {
                line: text.lines().count().saturating_add(1),
                number: *number,
                correction: if *severity == Severity::Major && !used {
                    Correction::FixedOrExplained
                } else {
                    Correction::FixedOnly
                },
            });
        }
    }
    Ok(answers)
}

#[cfg(test)]
mod tests;

pub mod opinion;
