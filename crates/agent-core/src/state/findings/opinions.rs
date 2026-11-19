//! Durable one-shot opinions, independent of daemon scheduling.
mod attempts;
use super::{FindingStatus, Findings, Severity};
use crate::ids::{InvestigatorName, TaskId};
use crate::review_response::opinion::{Evidence, OpinionReport, Outcome};
use crate::state::CommitId;
pub use attempts::{OpinionAttempt, OpinionFailure};
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
    /// Records a complete fix response and its opinion requests atomically.
    ///
    /// # Errors
    /// Returns the protocol error without changing any finding when a line is invalid.
    pub fn answer_opinions(
        &mut self,
        text: &str,
        pass: NonZeroU32,
        head: &CommitId,
        permission: crate::review_response::opinion::Permission,
    ) -> Result<
        Vec<(NonZeroU32, crate::review_response::opinion::LeadAnswer)>,
        crate::review_response::opinion::OpinionError,
    > {
        use crate::review_response::opinion::{LeadAnswer, OpenFinding};
        let fixed = self.fixed_answers(pass);
        let open = self
            .entries
            .iter()
            .filter(|entry| {
                entry.status == FindingStatus::Open
                    && !fixed.contains(&entry.number)
                    && !self.opinions.get(&entry.number).is_some_and(|opinion| {
                        matches!(opinion.stage, OpinionStage::Waiting | OpinionStage::Assigned { .. })
                    })
                    && !self.disputes.as_ref().is_some_and(|disputes| {
                        disputes
                            .explanations
                            .get(&entry.number)
                            .is_some_and(|explanation| explanation.status == super::ExplanationStatus::Pending)
                    })
            })
            .map(|entry| OpenFinding {
                number: entry.number,
                severity: entry.finding.severity,
                explained: self
                    .disputes
                    .as_ref()
                    .is_some_and(|disputes| disputes.explanations.contains_key(&entry.number)),
                opinion_used: self.opinions.contains_key(&entry.number),
            })
            .collect::<Vec<_>>();
        let answers = crate::review_response::opinion::parse_lead_located(text, &open, permission)?;
        for (number, answer) in &answers {
            match answer {
                LeadAnswer::Fixed => {
                    self.fixed.entry(pass).or_default().insert(*number);
                }
                LeadAnswer::Explained(why) => {
                    if let Some(disputes) = &mut self.disputes {
                        disputes.explanations.insert(
                            *number,
                            super::Explanation {
                                why: why.clone(),
                                status: super::ExplanationStatus::Pending,
                                votes: std::collections::BTreeMap::default(),
                            },
                        );
                    }
                }
                LeadAnswer::SecondOpinion(reason) => {
                    self.opinions.insert(
                        *number,
                        SecondOpinion {
                            request: OpinionRequest {
                                pass,
                                head: head.clone(),
                                reason: reason.clone(),
                            },
                            stage: OpinionStage::Waiting,
                        },
                    );
                }
            }
        }
        if let Some(disputes) = &mut self.disputes {
            disputes.answered_pass = pass.get();
            disputes.bounces = 0;
            disputes.correction = None;
        }
        Ok(answers)
    }

    /// Accepted code fixes for a particular fix pass, retained across restarts.
    #[must_use]
    pub fn fixed_answers(&self, pass: NonZeroU32) -> std::collections::BTreeSet<NonZeroU32> {
        self.fixed.get(&pass).cloned().unwrap_or_default()
    }

    /// Checks cross-record invariants once at the deserialization boundary.
    pub(super) fn check_opinions(&self) -> Result<(), OpinionStateError> {
        for (number, opinion) in &self.opinions {
            let entry = self
                .entries
                .iter()
                .find(|entry| entry.number == *number)
                .ok_or(OpinionStateError { number: *number })?;
            let consistent = match &opinion.stage {
                OpinionStage::Waiting | OpinionStage::Assigned { .. } => entry.status == FindingStatus::Open,
                OpinionStage::Reported { report, .. } => match report.outcome {
                    Outcome::Confirmed => matches!(entry.status, FindingStatus::Open | FindingStatus::Resolved),
                    Outcome::NotConfirmed => entry.status == FindingStatus::NotConfirmed,
                },
            };
            if entry.finding.severity != Severity::Blocker || !consistent {
                return Err(OpinionStateError { number: *number });
            }
        }
        self.check_attempts()?;
        for entry in &self.entries {
            if entry.status == FindingStatus::NotConfirmed && !self.opinions.contains_key(&entry.number) {
                return Err(OpinionStateError { number: entry.number });
            }
        }
        Ok(())
    }

    /// Claims the one human notice for an unanswered opinion, returning whether it is new.
    ///
    /// # Errors
    /// [`OpinionStateError`] when no assigned opinion exists for this finding.
    pub fn mark_opinion_notice(&mut self, number: NonZeroU32) -> Result<bool, OpinionStateError> {
        if !self
            .opinions
            .get(&number)
            .is_some_and(|opinion| matches!(opinion.stage, OpinionStage::Assigned { .. }))
        {
            return Err(OpinionStateError { number });
        }
        Ok(self.opinion_notices.insert(number))
    }

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
        if self.opinions.contains_key(&number)
            || !self.entries.iter().any(|entry| {
                entry.number == number
                    && entry.status == FindingStatus::Open
                    && entry.finding.severity == Severity::Blocker
            })
        {
            return Err(OpinionStateError { number });
        }
        self.opinions.insert(
            number,
            SecondOpinion {
                request,
                stage: OpinionStage::Waiting,
            },
        );
        Ok(())
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
        let opinion = self.opinions.get_mut(&number).ok_or(OpinionStateError { number })?;
        match opinion.stage {
            OpinionStage::Waiting | OpinionStage::Assigned { .. } => {
                opinion.stage = OpinionStage::Assigned { investigator, task }
            }
            OpinionStage::Reported { .. } => return Err(OpinionStateError { number }),
        }
        Ok(())
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
        if self
            .opinion_attempts(number)
            .last()
            .is_some_and(|attempt| attempt.failure.is_some())
        {
            return Err(OpinionStateError { number });
        }
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
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.number == number)
            .ok_or(OpinionStateError { number })?;
        entry.status = match report.outcome {
            Outcome::Confirmed => FindingStatus::Open,
            Outcome::NotConfirmed => FindingStatus::NotConfirmed,
        };
        opinion.stage = OpinionStage::Reported {
            investigator: investigator.clone(),
            task: owner.clone(),
            report,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests;
