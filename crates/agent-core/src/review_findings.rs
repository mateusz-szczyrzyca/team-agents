//! Structured review findings and the answers of a later review pass.

use crate::status::ReviewVerdict;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU32;

/// The importance of a review finding; only minor findings permit approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Severity {
    /// Broken logic, build, or gate.
    Blocker,
    /// An unexplained deviation from a specification.
    Major,
    /// Advice that never blocks approval.
    Minor,
}

impl Severity {
    /// The plain word used in reports and state.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Blocker => "BLOCKER",
            Self::Major => "MAJOR",
            Self::Minor => "MINOR",
        }
    }
    /// Whether an open finding prevents approval.
    #[must_use]
    pub const fn blocks(self) -> bool {
        !matches!(self, Self::Minor)
    }
}

/// One newly reported finding, before the harness gives it a permanent number.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    /// Its importance.
    pub severity: Severity,
    /// The specification rule, separate from severity.
    pub rule: String,
    /// File and line, or another concrete location.
    pub place: String,
    /// Problem and proposed fix.
    pub text: String,
    /// A short verification instruction.
    pub check: String,
}

/// A verifier's answer about one existing finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum Answer {
    /// The check no longer fails.
    Resolved,
    /// The problem remains, for this reason.
    Open {
        /// Why the check still fails.
        why: String,
    },
}

/// The structured part of a report, alongside its separately parsed verdict.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    /// New findings, in report order.
    pub findings: Vec<Finding>,
    /// Answers for the open findings, in report order.
    pub answers: Vec<(NonZeroU32, Answer)>,
}

/// A malformed finding or a verdict that contradicts the report.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid review report at line {line}: {kind}")]
pub struct ReportError {
    /// One-based source line, or the end of the report for an omitted answer.
    pub line: usize,
    /// Which part of the protocol failed.
    pub kind: ReportProblem,
}

/// The correction a report requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReportProblem {
    /// A finding does not have the specified syntax.
    #[error("expected N. [BLOCKER|MAJOR|MINOR] RULE file:line — problem → fix")]
    Finding,
    /// The verification instruction is missing.
    #[error("expected an indented CHECK: instruction after the finding")]
    Check,
    /// An answer is absent, duplicated, unknown, or malformed.
    #[error(
        "expected exactly one FINDING N: RESOLVED or FINDING N: OPEN — why for every open finding, before new findings"
    )]
    Answer,
    /// Blocking findings and verdict disagree.
    #[error("APPROVE requires no open BLOCKER or MAJOR; REQUEST_CHANGES requires at least one")]
    Verdict,
}

/// Reads findings and verification answers, checking the verdict against all open problems.
///
/// # Errors
/// [`ReportError`] identifies the malformed line or contradictory verdict.
pub fn parse_report(
    text: &str,
    verdict: ReviewVerdict,
    open: &[(NonZeroU32, Severity)],
) -> Result<Report, ReportError> {
    let lines: Vec<String> = text.lines().map(plain).collect();
    let mut report = Report::default();
    let mut in_findings = false;
    for (index, line) in lines.iter().enumerate() {
        let line_number = index.saturating_add(1);
        let fail = |kind| ReportError {
            line: line_number,
            kind,
        };
        let trimmed = line.trim().trim_start_matches('#').trim();
        if trimmed == "FINDINGS" {
            in_findings = true;
            continue;
        }
        if let Some(answer) = trimmed.strip_prefix("FINDING ") {
            if !report.findings.is_empty() {
                return Err(fail(ReportProblem::Answer));
            }
            let Some((id, value)) = answer.split_once(':') else {
                return Err(fail(ReportProblem::Answer));
            };
            let Some((id, _)) = open.iter().find(|(number, _)| number.to_string() == id.trim()) else {
                return Err(fail(ReportProblem::Answer));
            };
            let id = *id;
            if report.answers.iter().any(|(n, _)| *n == id) {
                return Err(fail(ReportProblem::Answer));
            }
            let answer = match value.trim() {
                "RESOLVED" => Answer::Resolved,
                value => match value.strip_prefix("OPEN — ").filter(|why| !why.trim().is_empty()) {
                    Some(why) => Answer::Open {
                        why: why.trim().to_owned(),
                    },
                    None => return Err(fail(ReportProblem::Answer)),
                },
            };
            report.answers.push((id, answer));
        } else if in_findings && numbered(trimmed) {
            let mut finding = read_finding(trimmed).ok_or_else(|| fail(ReportProblem::Finding))?;
            let check = lines
                .iter()
                .skip(index.saturating_add(1))
                .find(|next| !next.trim().is_empty())
                .and_then(|next| next.trim().strip_prefix("CHECK:"))
                .map(str::trim)
                .filter(|check| !check.is_empty())
                .ok_or_else(|| fail(ReportProblem::Check))?;
            finding.check = check.to_owned();
            report.findings.push(finding);
        }
    }
    if report.answers.len() != open.len() {
        return Err(ReportError {
            line: lines.len().saturating_add(1),
            kind: ReportProblem::Answer,
        });
    }
    let blocking = report.findings.iter().any(|finding| finding.severity.blocks())
        || open.iter().any(|(id, severity)| {
            severity.blocks()
                && report
                    .answers
                    .iter()
                    .any(|(n, answer)| n == id && matches!(answer, Answer::Open { .. }))
        });
    if blocking != (verdict == ReviewVerdict::RequestChanges) {
        return Err(ReportError {
            line: lines
                .iter()
                .position(|line| line.contains("VERDICT:"))
                .map_or(1, |n| n.saturating_add(1)),
            kind: ReportProblem::Verdict,
        });
    }
    Ok(report)
}

/// Removes only the Markdown emphasis and code delimiters allowed around protocol words.
fn plain(line: &str) -> String {
    line.chars().filter(|c| !matches!(c, '*' | '`')).collect()
}

/// Whether a line starts with a report-local numbered entry, including malformed zero entries.
fn numbered(line: &str) -> bool {
    line.split_once('.')
        .is_some_and(|(n, _)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// Parses the finding header; the following line supplies its check.
fn read_finding(line: &str) -> Option<Finding> {
    let (number, value) = line.split_once('.')?;
    if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) || number.chars().all(|c| c == '0') {
        return None;
    }
    let (severity, rest) = value.trim().strip_prefix('[')?.split_once(']')?;
    let severity = match severity {
        "BLOCKER" => Severity::Blocker,
        "MAJOR" => Severity::Major,
        "MINOR" => Severity::Minor,
        _ => return None,
    };
    let (location, text) = rest.trim().split_once(" — ")?;
    let (rule, place) = location.split_once(char::is_whitespace)?;
    let (problem, fix) = text.split_once(" → ")?;
    if rule.is_empty() || place.trim().is_empty() || problem.trim().is_empty() || fix.trim().is_empty() {
        return None;
    }
    Some(Finding {
        severity,
        rule: rule.to_owned(),
        place: place.trim().to_owned(),
        text: text.to_owned(),
        check: String::new(),
    })
}

#[cfg(test)]
mod tests;
