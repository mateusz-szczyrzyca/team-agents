//! Pure second-opinion protocol, activated by the daemon in part 2.
use crate::review_findings::Severity;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

/// Nonempty reasoning or evidence supplied by an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Evidence(String);

impl TryFrom<String> for Evidence {
    type Error = OpinionError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.trim().is_empty() {
            Err(OpinionError::MissingEvidence)
        } else {
            Ok(Self(value.trim().to_owned()))
        }
    }
}
impl From<Evidence> for String {
    fn from(value: Evidence) -> Self {
        value.0
    }
}
impl Evidence {
    /// The supplied reasoning, with outer whitespace removed.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether the finding holds at the requested commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The reported defect exists and must be fixed.
    Confirmed,
    /// The reported defect does not hold at this commit.
    NotConfirmed,
}

/// The investigator's decision and supporting observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpinionReport {
    /// Whether the finding holds.
    pub outcome: Outcome,
    /// What the investigator read or ran and what it showed.
    pub evidence: Evidence,
}

/// A malformed opinion protocol message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OpinionError {
    /// The report does not start with exactly one recognized verdict.
    #[error("opinion report has no unambiguous opening verdict")]
    MissingVerdict,
    /// Reasoning or evidence is required.
    #[error("opinion requires evidence")]
    MissingEvidence,
    /// The lead's message violates the existing response protocol.
    #[error(transparent)]
    Response(#[from] super::ResponseError),
    /// The requested finding is not eligible for an opinion.
    #[error("finding {number} cannot receive a second opinion")]
    Forbidden {
        /// Permanent finding number.
        number: NonZeroU32,
    },
}

/// Configuration permission, kept explicit at the pure protocol boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// One opinion per open blocker is permitted.
    Enabled,
    /// Every request must be corrected to a code fix.
    Disabled,
}

/// Eligibility of a finding before reading the lead's message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenFinding {
    /// Permanent finding number.
    pub number: NonZeroU32,
    /// Reported severity.
    pub severity: Severity,
    /// Whether the major's explanation was already used.
    pub explained: bool,
    /// Whether any second opinion was already requested.
    pub opinion_used: bool,
}

/// A checked answer to one open finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeadAnswer {
    /// Code has been corrected.
    Fixed,
    /// A major's intentional deviation is explained.
    Explained(String),
    /// One investigator must check this blocker with the supplied reason.
    SecondOpinion(Evidence),
}

/// Reads all answers atomically, including eligible second-opinion requests.
///
/// # Errors
/// Returns [`OpinionError`] for malformed, duplicated, missing or forbidden answers.
pub fn parse_lead(
    text: &str,
    open: &[OpenFinding],
    permission: Permission,
) -> Result<Vec<(NonZeroU32, LeadAnswer)>, OpinionError> {
    let mut requests = std::collections::BTreeMap::new();
    let mut normalized = String::new();
    let mut fence = None;
    for line in text.lines() {
        match (fence, crate::review_findings::fence_marker(line)) {
            (None, Some(marker)) => fence = Some(marker),
            (Some(marker), Some(end)) if end.closes(marker) => fence = None,
            _ => {}
        }
        let request = if fence.is_none() {
            line.trim()
                .strip_prefix("FINDING ")
                .and_then(|value| value.split_once(':'))
                .and_then(|(number, value)| {
                    let finding = open.iter().find(|entry| entry.number.to_string() == number.trim())?;
                    let reason = value.trim().strip_prefix("SECOND OPINION —")?;
                    Some((finding.number, reason.trim()))
                })
        } else {
            None
        };
        if let Some((number, reason)) = request {
            requests.insert(number, reason);
            normalized.push_str(&format!("FINDING {number}: FIXED"));
        } else {
            normalized.push_str(line);
        }
        normalized.push('\n');
    }
    let legacy: Vec<_> = open
        .iter()
        .map(|entry| (entry.number, entry.severity, entry.explained))
        .collect();
    super::parse_lead(&normalized, &legacy)?
        .into_iter()
        .map(|(number, answer)| {
            let answer = if let Some(reason) = requests.get(&number) {
                let eligible = open
                    .iter()
                    .any(|entry| entry.number == number && entry.severity == Severity::Blocker && !entry.opinion_used);
                if permission == Permission::Disabled || !eligible {
                    return Err(OpinionError::Forbidden { number });
                }
                LeadAnswer::SecondOpinion((*reason).to_owned().try_into()?)
            } else {
                match answer {
                    super::Response::Fixed => LeadAnswer::Fixed,
                    super::Response::Explained(why) => LeadAnswer::Explained(why),
                }
            };
            Ok((number, answer))
        })
        .collect()
}

/// Reads an opening opinion verdict followed by nonempty evidence.
///
/// # Errors
/// Returns [`OpinionError::MissingVerdict`] or [`OpinionError::MissingEvidence`].
pub fn parse_report(text: &str) -> Result<OpinionReport, OpinionError> {
    let mut lines = text.trim().lines();
    let first = lines.next().ok_or(OpinionError::MissingVerdict)?;
    let keyword = |line: &str| line.trim().trim_matches(['*', '`']).trim().to_owned();
    let outcome = match keyword(first).as_str() {
        "OPINION: CONFIRMED" => Outcome::Confirmed,
        "OPINION: NOT CONFIRMED" => Outcome::NotConfirmed,
        _ => return Err(OpinionError::MissingVerdict),
    };
    let remaining: Vec<_> = lines.collect();
    if remaining.iter().any(|line| keyword(line).starts_with("OPINION:")) {
        return Err(OpinionError::MissingVerdict);
    }
    Ok(OpinionReport {
        outcome,
        evidence: remaining.join("\n").try_into()?,
    })
}

#[cfg(test)]
mod tests;
