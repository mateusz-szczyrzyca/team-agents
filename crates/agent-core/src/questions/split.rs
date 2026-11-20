//! Explicit human approval of a whole split; free-text answers never create partial plans.

use crate::ids::ItemId;
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
pub fn read_split(verdict: &Verdict) -> Result<Option<SplitPlan>, ReadSplitError> {
    let Verdict::Own(answer) = verdict else {
        return Ok(None);
    };
    let text = answer.as_str();
    if text.len() > super::MAX_QUESTIONS_BYTES {
        return Err(ReadSplitError::TooLarge);
    }
    let mut lines = text.lines();
    if lines.next() != Some("split") {
        return Ok(None);
    }
    let Some(open) = lines.next() else {
        return Ok(None);
    };
    if !open.starts_with("```") {
        return Ok(None);
    }
    if open != "```json" {
        return Err(ReadSplitError::Shape);
    }
    let rest = lines.collect::<Vec<_>>();
    let Some((close, body)) = rest.split_last() else {
        return Err(ReadSplitError::Shape);
    };
    if *close != "```" || body.iter().any(|line| line.starts_with("```")) {
        return Err(ReadSplitError::Shape);
    }
    let wire: Approval = serde_json::from_str(&body.join("\n")).map_err(|source| ReadSplitError::Json { source })?;
    SplitPlan::from_input(wire.parent, &wire.parts)
        .map(Some)
        .map_err(|source| ReadSplitError::Invalid { source })
}

/// Complete human input; graph validation retains its typed cause separately from JSON errors.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Approval {
    /// Existing task-list item to split.
    parent: ItemId,
    /// All parts in their permanent letter order.
    parts: Vec<crate::state::split::proposal::PartInput>,
}

#[cfg(test)]
mod tests;
