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
                allowed: "one answer for each supplied blocking number",
            });
        };
        let allowed = if *severity == Severity::Major && !used {
            "FIXED or EXPLAINED — why"
        } else {
            "FIXED (explanation and second opinion are forbidden)"
        };
        let error = || ResponseError {
            line: index.saturating_add(1),
            number: *number,
            allowed,
        };
        if ended || answers.iter().any(|(id, _)| id == number) {
            return Err(error());
        }
        let response = if value == "FIXED" {
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
        if !answers.iter().any(|(id, _)| id == number) {
            return Err(ResponseError {
                line: text.lines().count().saturating_add(1),
                number: *number,
                allowed: if *severity == Severity::Major && !used {
                    "FIXED or EXPLAINED — why"
                } else {
                    "FIXED"
                },
            });
        }
    }
    Ok(answers)
}

#[cfg(test)]
mod tests;
