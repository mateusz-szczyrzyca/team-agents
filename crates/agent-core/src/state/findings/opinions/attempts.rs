//! Durable attempts and ownership when an independent investigator cannot answer.
use super::{Findings, OpinionStage, OpinionStateError};
use crate::decide::opinion::MAX_OPINION_ATTEMPTS;
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
    /// When this investigator was asked; absent for an assignment migrated from schema 8.
    pub asked_at: Option<Timestamp>,
    /// Absent while the assignment owns the report, or after a successful report.
    pub failure: Option<OpinionFailure>,
}

impl Findings {
    /// Validates the new history without imposing new fields on legacy assignments.
    pub(super) fn check_attempts(&self) -> Result<(), OpinionStateError> {
        for (number, attempts) in &self.opinion_attempts {
            let invalid = || OpinionStateError { number: *number };
            let opinion = self.opinions.get(number).ok_or_else(invalid)?;
            if attempts.is_empty() || attempts.len() > usize::from(MAX_OPINION_ATTEMPTS) {
                return Err(invalid());
            }
            let mut investigators = std::collections::HashSet::new();
            let mut tasks = std::collections::HashSet::new();
            let (last, previous) = attempts.split_last().ok_or_else(invalid)?;
            for attempt in attempts {
                if !investigators.insert(&attempt.investigator) || !tasks.insert(&attempt.task) {
                    return Err(invalid());
                }
            }
            if previous.iter().any(|attempt| attempt.failure.is_none()) {
                return Err(invalid());
            }
            let consistent = match &opinion.stage {
                OpinionStage::Waiting => last.failure.is_some() && attempts.len() < usize::from(MAX_OPINION_ATTEMPTS),
                OpinionStage::Assigned { investigator, task } => {
                    last.investigator == *investigator
                        && last.task == *task
                        && (last.failure.is_none() || attempts.len() == usize::from(MAX_OPINION_ATTEMPTS))
                }
                OpinionStage::Reported { investigator, task, .. } => {
                    last.investigator == *investigator && last.task == *task && last.failure.is_none()
                }
            };
            if !consistent {
                return Err(invalid());
            }
        }
        Ok(())
    }

    /// The attempts of a request, oldest first; old state has no retained attempts.
    #[must_use]
    pub fn opinion_attempts(&self, number: NonZeroU32) -> &[OpinionAttempt] {
        self.opinion_attempts.get(&number).map_or(&[], Vec::as_slice)
    }

    /// Assigns a request and retains its investigator and start time.
    ///
    /// # Errors
    /// Returns [`OpinionStateError`] if the request is not waiting or all three assignments were used.
    pub fn assign_opinion_at(
        &mut self,
        number: NonZeroU32,
        investigator: InvestigatorName,
        task: TaskId,
        at: Timestamp,
    ) -> Result<(), OpinionStateError> {
        if self.opinion_attempts(number).len() >= usize::from(MAX_OPINION_ATTEMPTS)
            || self
                .opinion_attempts(number)
                .iter()
                .any(|attempt| attempt.investigator == investigator || attempt.task == task)
            || !self
                .opinions
                .get(&number)
                .is_some_and(|opinion| matches!(opinion.stage, OpinionStage::Waiting))
        {
            return Err(OpinionStateError { number });
        }
        let opinion = self.opinions.get_mut(&number).ok_or(OpinionStateError { number })?;
        opinion.stage = OpinionStage::Assigned {
            investigator: investigator.clone(),
            task: task.clone(),
        };
        self.opinion_attempts.entry(number).or_default().push(OpinionAttempt {
            investigator,
            task,
            asked_at: Some(at),
            failure: None,
        });
        Ok(())
    }

    /// Ends only the owning assignment, making the request wait when another attempt is allowed.
    ///
    /// # Errors
    /// Returns [`OpinionStateError`] for an unknown request, a stale task or a repeated completion.
    pub fn fail_opinion(
        &mut self,
        number: NonZeroU32,
        task: &TaskId,
        failure: OpinionFailure,
    ) -> Result<(), OpinionStateError> {
        let opinion = self.opinions.get_mut(&number).ok_or(OpinionStateError { number })?;
        let OpinionStage::Assigned {
            investigator,
            task: owner,
        } = &opinion.stage
        else {
            return Err(OpinionStateError { number });
        };
        if owner != task {
            return Err(OpinionStateError { number });
        }
        let attempts = self.opinion_attempts.entry(number).or_default();
        if attempts.is_empty() {
            // A schema-8 interim notice must not consume the final three-attempt notice.
            self.opinion_notices.remove(&number);
            attempts.push(OpinionAttempt {
                investigator: investigator.clone(),
                task: task.clone(),
                asked_at: None,
                failure: None,
            });
        }
        let last = attempts.last_mut().ok_or(OpinionStateError { number })?;
        if last.task != *task || last.failure.is_some() {
            return Err(OpinionStateError { number });
        }
        last.failure = Some(failure);
        if crate::decide::opinion::unanswered(failure, attempts.len()) == crate::decide::opinion::Unanswered::Replace {
            opinion.stage = OpinionStage::Waiting;
        }
        Ok(())
    }
}
