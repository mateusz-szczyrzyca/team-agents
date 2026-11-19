//! One-time explanations and the durable correction budget of a fix response.
use super::{Answer, FindingStatus, Findings, Report, Severity};
use crate::ids::{InvestigatorName, RunId};
use crate::review_findings::ReportError;
use crate::review_response::{Response, ResponseError};
use crate::status::ReviewVerdict;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU32;

/// How the verification pass decided an explanation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExplanationStatus {
    /// The verification pass has not completed.
    Pending,
    /// Every verifier accepted it; the finding is minor.
    Accepted,
    /// At least one verifier rejected it; a code fix is required.
    Rejected,
}

/// An explanation can be offered once, including when its verification rejects it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Explanation {
    /// The lead's justification.
    pub why: String,
    /// The pass's decision.
    pub status: ExplanationStatus,
    /// Each verifier's vote and evidence.
    pub votes: BTreeMap<InvestigatorName, Answer>,
}

/// One verifier's upward severity change and its evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaisedFinding {
    /// The permanent finding number.
    pub number: NonZeroU32,
    /// Who raised it.
    pub reviewer: InvestigatorName,
    /// The increased severity.
    pub severity: Severity,
    /// Why it was raised.
    pub why: String,
}

/// The protocol marker and durable decisions for new pull requests.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Disputes {
    /// Commit at which each accepted explanation became minor.
    #[serde(default, deserialize_with = "crate::protocol::finding_map::deserialize_map")]
    pub accepted_heads: BTreeMap<NonZeroU32, crate::state::CommitId>,
    /// Accepted findings whose file differs at the current review head.
    #[serde(default, deserialize_with = "crate::protocol::finding_map::deserialize_set")]
    pub changed_findings: BTreeSet<NonZeroU32>,
    /// Explanations, retained after acceptance, rejection and resolution.
    #[serde(deserialize_with = "crate::protocol::finding_map::deserialize_map")]
    pub explanations: BTreeMap<NonZeroU32, Explanation>,
    /// Upward severity changes in verification order.
    pub raised: Vec<RaisedFinding>,
    /// The fix pass whose lead response has been accepted; zero before any fix pass.
    pub answered_pass: u32,
    /// Invalid responses since the last valid response; two corrections are allowed.
    pub bounces: u32,
    /// Last run checked, preventing repeated corrections during polling or recovery.
    pub checked_run: Option<RunId>,
    /// Rendered correction, retained for a resumed run after a daemon restart.
    pub correction: Option<String>,
}

impl Findings {
    /// Records the commit at which an explanation became accepted.
    pub fn accepted_at(&mut self, head: &crate::state::CommitId) {
        if let Some(disputes) = &mut self.disputes {
            for (number, explanation) in &disputes.explanations {
                if explanation.status == ExplanationStatus::Accepted {
                    disputes.accepted_heads.entry(*number).or_insert_with(|| head.clone());
                }
            }
        }
    }
    /// Marks accepted findings whose file changed since acceptance as eligible for raising.
    pub fn changed_since_acceptance(
        &mut self,
        base: &crate::state::CommitId,
        files: &[crate::decide::checks::FileChange],
    ) {
        let Some(disputes) = &mut self.disputes else {
            return;
        };
        for entry in &self.entries {
            if disputes.accepted_heads.get(&entry.number) != Some(base) {
                continue;
            }
            let path = entry
                .finding
                .place
                .rsplit_once(':')
                .map_or(entry.finding.place.as_str(), |(path, _)| path);
            let changed = files.iter().any(|file| {
                file.path.as_str() == path
                    || matches!(&file.change, crate::decide::checks::Change::Renamed {from} if from.as_str() == path)
            });
            if changed {
                disputes.changed_findings.insert(entry.number);
            } else {
                disputes.changed_findings.remove(&entry.number);
            }
        }
    }

    /// The dispute protocol, absent for migrated active pull requests.
    #[must_use]
    pub const fn disputes(&self) -> Option<&Disputes> {
        self.disputes.as_ref()
    }
    /// The mutable correction state of the dispute protocol.
    pub const fn disputes_mut(&mut self) -> Option<&mut Disputes> {
        self.disputes.as_mut()
    }
    /// Reads a report with the protocol this pull request started with.
    ///
    /// # Errors
    /// [`ReportError`] for a malformed or contradictory report.
    pub fn parse_report(&self, text: &str, verdict: ReviewVerdict) -> Result<Report, ReportError> {
        if self.disputes.is_none() {
            return crate::review_findings::parse_report(text, verdict, &self.open());
        }
        // What the brief offered for each finding is what is taken: the same list decides.
        let offered = self.verifier_answers();
        let pending: Vec<_> = offered
            .iter()
            .filter(|answers| answers.decision == super::Decision::Explanation)
            .map(|answers| answers.number)
            .collect();
        let report = crate::review_findings::parse_verification(text, verdict, &self.open(), &pending)?;
        for (number, answer) in &report.answers {
            // The parser already refuses a raise that the severity does not allow; what is left is the protection of an
            // accepted explanation whose code did not change.
            let refused = matches!(answer, Answer::Raised { .. })
                && offered
                    .iter()
                    .any(|answers| answers.number == *number && answers.raise == super::Raise::Never);
            if refused {
                let prefix = format!("FINDING {number}:");
                return Err(ReportError {
                    line: text
                        .lines()
                        .position(|line| line.trim().starts_with(&prefix))
                        .map_or(1, |i| i.saturating_add(1)),
                    kind: crate::review_findings::ReportProblem::UnchangedExplanation,
                    detail: crate::review_findings::AnswerDiagnostic {
                        source_line: text
                            .lines()
                            .find(|line| line.trim().starts_with(&prefix))
                            .map(str::to_owned),
                        number: Some(*number),
                        mistake: crate::review_findings::AnswerMistake::NotAllowed,
                    },
                });
            }
        }
        Ok(report)
    }
    /// Checks and records the lead's complete response before a verification pass.
    ///
    /// # Errors
    /// [`ResponseError`] identifies the line to correct; no decisions change on failure.
    pub fn answer_lead(&mut self, text: &str, pass: u32) -> Result<Vec<(NonZeroU32, Response)>, ResponseError> {
        let Some(disputes) = &mut self.disputes else {
            return Ok(vec![]);
        };
        let blocking: Vec<_> = self
            .entries
            .iter()
            .filter(|e| e.status == FindingStatus::Open)
            .map(|e| {
                (
                    e.number,
                    e.finding.severity,
                    disputes.explanations.contains_key(&e.number),
                )
            })
            .collect();
        let responses = crate::review_response::parse_lead(text, &blocking)?;
        for (number, response) in &responses {
            if let Response::Explained(why) = response {
                disputes.explanations.insert(
                    *number,
                    Explanation {
                        why: why.clone(),
                        status: ExplanationStatus::Pending,
                        votes: BTreeMap::new(),
                    },
                );
            }
        }
        disputes.answered_pass = pass;
        disputes.bounces = 0;
        disputes.correction = None;
        Ok(responses)
    }
    /// Applies the completed pass's votes before ordinary fix resolutions.
    pub(super) fn finish_disputes(&mut self, reviews: &[super::Review]) {
        let Some(disputes) = &mut self.disputes else {
            return;
        };
        for entry in &mut self.entries {
            let votes: BTreeMap<_, _> = reviews
                .iter()
                .filter_map(|review| {
                    self.pending
                        .get(&review.task)?
                        .answers
                        .iter()
                        .find(|(n, _)| *n == entry.number)
                        .map(|(_, answer)| (review.reviewer.clone(), answer.clone()))
                })
                .collect();
            if let Some(explanation) = disputes.explanations.get_mut(&entry.number)
                && explanation.status == ExplanationStatus::Pending
                && votes.len() == reviews.len()
                && !votes.is_empty()
            {
                let accepted = votes.values().all(|a| matches!(a, Answer::Accepted { .. }));
                explanation.status = if accepted {
                    ExplanationStatus::Accepted
                } else {
                    ExplanationStatus::Rejected
                };
                explanation.votes = votes.clone();
                if accepted {
                    entry.finding.severity = Severity::Minor;
                }
            }
            for (reviewer, answer) in votes {
                if let Answer::Raised { severity, why } = answer {
                    if severity == Severity::Blocker || entry.finding.severity == Severity::Minor {
                        entry.finding.severity = severity;
                    }
                    entry.status = FindingStatus::Open;
                    disputes.raised.push(RaisedFinding {
                        number: entry.number,
                        reviewer,
                        severity,
                        why,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
