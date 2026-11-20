//! Typed coordinator claims; parsing a claim never verifies or completes an item.

use crate::state::PrNumber;

/// Why a delivery report cannot be interpreted.
#[derive(Debug, thiserror::Error)]
pub enum DeliveryReportError {
    /// More than one claim cannot identify a single integration outcome.
    #[error("more than one delivery report")]
    Duplicate,
    /// Only a final successful report may claim delivery.
    #[error("delivery report requires a final STATUS: DONE")]
    NotDone,
    /// The number is zero or exceeds the PR number range.
    #[error("invalid delivery PR number on line {line}")]
    Number {
        /// One-based line containing the number.
        line: usize,
        /// The numeric parser's reason.
        #[source]
        source: std::num::ParseIntError,
    },
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
pub fn delivery_report(message: &str) -> Result<Option<PrNumber>, DeliveryReportError> {
    let mut report = None;
    for (index, text) in message.lines().enumerate() {
        let Some(value) = text.strip_prefix("DELIVERED:") else {
            continue;
        };
        if report.is_some() {
            return Err(DeliveryReportError::Duplicate);
        }
        let line = index.saturating_add(1);
        let number = value.strip_prefix(" #").ok_or(DeliveryReportError::Syntax { line })?;
        if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(DeliveryReportError::Syntax { line });
        }
        let number = number
            .parse::<std::num::NonZeroU32>()
            .map_err(|source| DeliveryReportError::Number { line, source })?;
        report = Some(PrNumber::new(number));
    }
    if report.is_some()
        && (message.lines().rev().find(|line| !line.trim().is_empty()) != Some("STATUS: DONE")
            || super::outcome::parse_status_line(message)
                != super::outcome::StatusLine::Reported(crate::status::RunOutcome::Done))
    {
        return Err(DeliveryReportError::NotDone);
    }
    Ok(report)
}

#[cfg(test)]
mod tests;

/// Why a coordinator run does not identify exactly one working delivery round.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeliveryBindingError {
    /// A report cannot fall back to an unrelated team's working item.
    #[error("the coordinator run names no open working item round")]
    Missing,
    /// An unqualified report cannot choose between multiple working items.
    #[error("the coordinator run names multiple open working item rounds")]
    Ambiguous,
    /// Run references must belong to the team that made the report.
    #[error("the coordinator run names unknown round {round}")]
    Unknown {
        /// The foreign or missing round.
        round: crate::ids::RoundId,
    },
}

/// Binds a report only to an open working item among the coordinator run's durable round references.
///
/// # Errors
/// Returns [`DeliveryBindingError`] when the references cannot identify exactly one working item.
pub fn bind_delivery(
    _team: &crate::state::Team,
    _rounds: &[crate::ids::RoundId],
) -> Result<crate::ids::RoundId, DeliveryBindingError> {
    Err(DeliveryBindingError::Missing)
}
