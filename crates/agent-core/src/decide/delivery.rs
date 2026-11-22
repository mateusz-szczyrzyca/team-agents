//! Typed coordinator claims; parsing a claim never verifies or completes an item.

use crate::state::PrNumber;

use crate::ids::RoundId;
use crate::state::Team;

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

/// The observed state of a verified delivering pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryObservation {
    /// Still open, including a reopening before the daemon observed a close.
    Open,
    /// Closed without merging.
    Closed,
    /// Merged into its base.
    Merged,
}

/// The action required by an observation of an accepted claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryAction {
    /// No current claim remains to act on.
    Ignore,
    /// Continue waiting for the same PR.
    Wait,
    /// Complete the item through the verified PR.
    Complete,
    /// End the claim and return the item to its member.
    Return,
}

/// Decides only for a currently waiting claim; ended claims and completed items cannot be revived.
#[must_use]
pub fn delivery_action(team: &Team, round: &RoundId, observed: DeliveryObservation) -> DeliveryAction {
    if !team.delivery_waits().any(|waiting| waiting.id() == round) {
        return DeliveryAction::Ignore;
    }
    match observed {
        DeliveryObservation::Open => DeliveryAction::Wait,
        DeliveryObservation::Closed => DeliveryAction::Return,
        DeliveryObservation::Merged => DeliveryAction::Complete,
    }
}

/// The validation outcome of a lead's delivery report for a working round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryVerdict {
    /// The PR is merged and targets a recognized branch of the item or part.
    Accepted,
    /// The PR is open and cannot be accepted.
    RefusedOpen,
    /// The PR was closed without being merged.
    RefusedClosed,
    /// The PR belongs to another repository.
    RefusedForeignRepository,
    /// The PR branch is neither the task's nor the part's branch.
    RefusedForeignBranch,
}

impl DeliveryVerdict {
    /// Whether the report is accepted.
    #[must_use]
    pub const fn is_accepted(&self) -> bool {
        matches!(self, Self::Accepted)
    }

    /// The human-readable explanation of why the report was refused, or none if accepted.
    #[must_use]
    pub const fn refusal_reason(&self) -> Option<&'static str> {
        match self {
            Self::Accepted => None,
            Self::RefusedOpen => Some("the delivering pull request is not merged"),
            Self::RefusedClosed => Some("the delivering pull request was closed without merging"),
            Self::RefusedForeignRepository => Some("the delivering pull request belongs to another repository"),
            Self::RefusedForeignBranch => Some("the delivering pull request does not use the task's or part's branch"),
        }
    }
}

/// What the lead's delivery report is evaluated against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveryReportFacts<'a> {
    /// Whether the pull request belongs to the team's repository.
    pub same_repo: bool,
    /// The branch of the pull request.
    pub pr_branch: &'a str,
    /// The task's branch.
    pub task_branch: &'a str,
    /// The part's branch, if delivering a split part.
    pub part_branch: Option<&'a str>,
    /// The observed state of the pull request.
    pub state: DeliveryObservation,
}

/// Evaluates a lead's delivery report: merged, open, closed without merge, another repository,
/// a branch that is neither the task's nor the part's branch.
#[must_use]
pub fn evaluate_delivery_report(facts: DeliveryReportFacts<'_>) -> DeliveryVerdict {
    if !facts.same_repo {
        return DeliveryVerdict::RefusedForeignRepository;
    }
    let branch_fits = match facts.part_branch {
        Some(part) => facts.pr_branch == part,
        None => facts.pr_branch == facts.task_branch,
    };
    if !branch_fits {
        return DeliveryVerdict::RefusedForeignBranch;
    }
    match facts.state {
        DeliveryObservation::Merged => DeliveryVerdict::Accepted,
        DeliveryObservation::Open => DeliveryVerdict::RefusedOpen,
        DeliveryObservation::Closed => DeliveryVerdict::RefusedClosed,
    }
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
    team: &crate::state::Team,
    rounds: &[crate::ids::RoundId],
) -> Result<crate::ids::RoundId, DeliveryBindingError> {
    let mut target = None;
    for id in rounds {
        let round = team
            .round(id)
            .ok_or_else(|| DeliveryBindingError::Unknown { round: id.clone() })?;
        if round.is_open()
            && round.delivered_by().is_none()
            && round
                .item()
                .and_then(|item| team.item(item))
                .is_some_and(|item| matches!(item.phase(), crate::state::ItemPhase::InProgress { .. }))
        {
            if target.is_some() {
                return Err(DeliveryBindingError::Ambiguous);
            }
            target = Some(id.clone());
        }
    }
    target.ok_or(DeliveryBindingError::Missing)
}
