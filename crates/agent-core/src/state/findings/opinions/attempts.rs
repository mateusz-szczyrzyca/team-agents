//! Durable attempts and ownership when an independent investigator cannot answer.
use super::{Findings, OpinionStateError};
use crate::ids::{InvestigatorName, TaskId};
use crate::state::Timestamp;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

/// Why an assigned investigator did not answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpinionFailure {
    /// The process or reported status failed.
    Failed,
    /// The run supplied no final status.
    NoStatus,
    /// Corrections were exhausted without an opinion line.
    MissingReport,
    /// The configured review inactivity interval elapsed.
    Silent,
    /// The interrupted task can no longer be resumed.
    Interrupted,
}

/// One investigator's assignment, retained after replacement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpinionAttempt {
    /// The investigator excluded from subsequent draws.
    pub investigator: InvestigatorName,
    /// The sole task permitted to answer this assignment.
    pub task: TaskId,
    /// When this investigator was asked; zero means an assignment migrated from schema 8.
    pub asked_at: Timestamp,
    /// Absent while the assignment owns the report, or after a successful report.
    pub failure: Option<OpinionFailure>,
}

impl Findings {
    /// The attempts of a request, oldest first; old state has no retained attempts.
    #[must_use]
    pub fn opinion_attempts(&self, _number: NonZeroU32) -> &[OpinionAttempt] {
        &[]
    }

    /// Assigns a request and retains its investigator and start time.
    ///
    /// # Errors
    /// Returns [`OpinionStateError`] if the request is not waiting or all three assignments were used.
    pub fn assign_opinion_at(
        &mut self,
        number: NonZeroU32,
        _investigator: InvestigatorName,
        _task: TaskId,
        _at: Timestamp,
    ) -> Result<(), OpinionStateError> {
        Err(OpinionStateError { number })
    }

    /// Ends only the owning assignment, making the request wait when another attempt is allowed.
    ///
    /// # Errors
    /// Returns [`OpinionStateError`] for an unknown request, a stale task or a repeated completion.
    pub fn fail_opinion(
        &mut self,
        number: NonZeroU32,
        _task: &TaskId,
        _failure: OpinionFailure,
    ) -> Result<(), OpinionStateError> {
        Err(OpinionStateError { number })
    }
}
