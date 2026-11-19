//! Durable one-shot opinions, independent of daemon scheduling.
use super::{FindingStatus, Findings, Severity};
use crate::ids::{InvestigatorName, TaskId};
use crate::review_response::opinion::{Evidence, OpinionReport, Outcome};
use crate::state::CommitId;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

/// Context that remains unchanged while an opinion waits or is replaced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpinionRequest {
    /// The fix pass requesting the opinion, counted once even after confirmation.
    pub pass: NonZeroU32,
    /// The exact commit the investigator must examine.
    pub head: CommitId,
    /// Why the lead disputes the blocker.
    pub reason: Evidence,
}

/// Scheduling and outcome of one requested opinion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case", deny_unknown_fields)]
pub enum OpinionStage {
    /// No eligible investigator is free yet.
    Waiting,
    /// The selected task owns the report, including across restarts.
    Assigned {
        /// Selected independent investigator.
        investigator: InvestigatorName,
        /// Existing task to resume after a restart.
        task: TaskId,
    },
    /// A report has been accepted and must not be requested again.
    Reported {
        /// Investigator responsible for the evidence.
        investigator: InvestigatorName,
        /// Task that supplied the report.
        task: TaskId,
        /// Decision and evidence.
        report: OpinionReport,
    },
}

/// A finding's one permitted second opinion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecondOpinion {
    /// Immutable reason, head and fix-pass identity.
    pub request: OpinionRequest,
    /// Durable scheduling or completed outcome.
    pub stage: OpinionStage,
}

/// A request or transition that cannot apply to this finding.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("finding {number} is ineligible for this opinion transition")]
pub struct OpinionStateError {
    /// Permanent finding number whose state did not permit the operation.
    pub number: NonZeroU32,
}

impl Findings {
    /// The durable opinions indexed by permanent finding number.
    #[must_use]
    pub fn opinions(&self) -> &std::collections::BTreeMap<NonZeroU32, SecondOpinion> {
        &self.opinions
    }

    /// Requests one opinion for an open blocker, before selecting an investigator.
    ///
    /// # Errors
    /// [`OpinionStateError`] if missing, closed, non-blocking or already requested.
    pub fn request_opinion(&mut self, number: NonZeroU32, request: OpinionRequest) -> Result<(), OpinionStateError> {
        let _request = request;
        Err(OpinionStateError { number })
    }

    /// Assigns or replaces a pending opinion task; completed reports cannot be replaced.
    ///
    /// # Errors
    /// [`OpinionStateError`] if the opinion was not requested or already completed.
    pub fn assign_opinion(
        &mut self,
        number: NonZeroU32,
        investigator: InvestigatorName,
        task: TaskId,
    ) -> Result<(), OpinionStateError> {
        let _inputs = (investigator, task);
        Err(OpinionStateError { number })
    }

    /// Accepts the assigned task's report once, closing only an unconfirmed finding.
    ///
    /// # Errors
    /// [`OpinionStateError`] if this task does not own an unanswered opinion.
    pub fn record_opinion(
        &mut self,
        number: NonZeroU32,
        task: &TaskId,
        report: OpinionReport,
    ) -> Result<(), OpinionStateError> {
        let _inputs = (task, report);
        Err(OpinionStateError { number })
    }
}

#[cfg(test)]
mod tests;
