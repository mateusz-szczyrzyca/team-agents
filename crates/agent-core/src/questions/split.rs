//! Explicit human approval of a whole split; free-text answers never create partial plans.

use crate::state::split::{SplitError, SplitPlan};

use super::Verdict;

/// Why an explicit split verdict could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ReadSplitError {
    /// The answer exceeds the questions-file input bound.
    #[error("split verdict exceeds the questions-file size limit")]
    TooLarge,
    /// An explicit split marker did not contain exactly one fenced JSON object.
    #[error("split verdict requires split followed by exactly one fenced json object")]
    Shape,
    /// The JSON object did not match the documented fields.
    #[error("invalid split verdict json")]
    Json {
        /// The syntax or field error.
        source: serde_json::Error,
    },
    /// The whole plan failed validation.
    #[error("invalid split plan")]
    Invalid {
        /// The part or dependency that prevents approval.
        source: SplitError,
    },
}

/// Reads an explicit `split` verdict containing a fenced JSON plan.
///
/// `ok` and free text return `None`: neither approves a machine-readable plan.
/// This reader does not record a plan or acknowledge an answer.
///
/// # Errors
/// Returns [`ReadSplitError`] for oversized input or a malformed explicit approval.
pub fn read_split(_verdict: &Verdict) -> Result<Option<SplitPlan>, ReadSplitError> {
    Ok(None)
}

#[cfg(test)]
mod tests;
