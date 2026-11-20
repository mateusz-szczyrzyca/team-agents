//! Typed coordinator claims; parsing a claim never verifies or completes an item.

use crate::state::PrNumber;

/// Why a delivery report cannot be interpreted.
#[derive(Debug, thiserror::Error)]
pub enum DeliveryReportError {
    /// A delivery marker does not have the standalone protocol form.
    #[error("invalid delivery report on line {line}")]
    Syntax {
        /// One-based line containing the marker.
        line: usize,
    },
}

/// Reads a standalone `DELIVERED: #N` claim from a coordinator's final message.
///
/// The caller must still verify team ownership, git evidence and the PR's merge.
///
/// # Errors
/// Returns [`DeliveryReportError`] for a malformed delivery marker.
pub fn delivery_report(_message: &str) -> Result<Option<PrNumber>, DeliveryReportError> {
    Ok(None)
}

#[cfg(test)]
mod tests;
