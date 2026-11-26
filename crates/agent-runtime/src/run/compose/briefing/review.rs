//! The pull request of the task's item and its review, as the briefing of a new session tells them: the pull request, where
//! the review stands and the findings the reviews recorded.

use std::num::NonZeroU32;

use agent_core::state::{FindingStatus, RecordedFinding};

/// The pull request of the item the task works for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BriefingPullRequest<'a> {
    /// Its number in the repository.
    pub number: NonZeroU32,
    /// The branch it merges.
    pub branch: &'a str,
    /// When it was opened, as the briefing shows the time.
    pub opened: &'a str,
}

/// Where the review of the item stands. A pass counts from 1, as the history shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewStanding {
    /// The reviewers are reading the pull request.
    InReview {
        /// The review pass.
        pass: NonZeroU32,
    },
    /// The team fixes what the reviews asked for.
    Fixing {
        /// The fix pass.
        pass: NonZeroU32,
        /// The configured number of fix passes.
        limit: u32,
    },
    /// The fix pass was sent back as often as it is; it waits for the human.
    WaitingForHuman {
        /// The fix pass.
        pass: NonZeroU32,
        /// The configured number of fix passes.
        limit: u32,
    },
    /// The reviews approved the pull request; it waits to be merged.
    Approved,
    /// The item has no pull request in review.
    NotInReview,
}

/// What the reviews of the pull request recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BriefingFindings<'a> {
    /// The numbered findings, in the order of their permanent numbers; none yet when the slice is empty.
    Ledger(&'a [RecordedFinding]),
    /// The pull request was reviewed before the findings were numbered: only its comments tell them.
    Legacy {
        /// The pull request whose comments hold the findings.
        pull_request: NonZeroU32,
    },
}

/// The pull request of the task's item and its review. Its default is an item with no pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BriefingReview<'a> {
    /// The pull request, when the item has one.
    pub pull_request: Option<BriefingPullRequest<'a>>,
    /// Where its review stands.
    pub standing: ReviewStanding,
    /// What the reviews recorded.
    pub findings: BriefingFindings<'a>,
}

impl Default for BriefingReview<'_> {
    fn default() -> Self {
        Self {
            pull_request: None,
            standing: ReviewStanding::NotInReview,
            findings: BriefingFindings::Ledger(&[]),
        }
    }
}

/// The values of the variables `pull_request`, `review_state` and `findings` of the briefing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewValues {
    /// The value of `{{pull_request}}`.
    pub(super) pull_request: String,
    /// The value of `{{review_state}}`.
    pub(super) review_state: String,
    /// The value of `{{findings}}`.
    pub(super) findings: String,
}

/// The line that tells there is no pull request.
const NO_PULL_REQUEST: &str = "No pull request yet.";

/// How many findings the briefing lists whole; above it only the open ones are listed.
const FINDINGS_LISTED: usize = 40;

/// The values of the briefing's review variables for `review`.
pub(super) fn review_values(review: &BriefingReview<'_>) -> ReviewValues {
    ReviewValues {
        pull_request: pull_request_line(review.pull_request.as_ref()),
        review_state: standing_line(review.standing),
        findings: findings_text(&review.findings),
    }
}

/// The line that names the pull request, or tells there is none.
fn pull_request_line(pull_request: Option<&BriefingPullRequest<'_>>) -> String {
    match pull_request {
        Some(pull_request) => format!(
            "Pull request #{} on branch {}, opened {}.",
            pull_request.number, pull_request.branch, pull_request.opened
        ),
        None => NO_PULL_REQUEST.to_owned(),
    }
}

/// The line that tells where the review stands.
fn standing_line(standing: ReviewStanding) -> String {
    match standing {
        ReviewStanding::InReview { pass } => {
            format!("in review, pass {pass}: the reviewers are reading it; wait for their findings.")
        }
        ReviewStanding::Fixing { pass, limit } => {
            format!("fix pass {pass} of {limit}: fix the open findings below, in this pull request.")
        }
        ReviewStanding::WaitingForHuman { pass, limit } => format!(
            "fix pass {pass} of {limit} was sent back to its limit; it waits for the human. Read the human's messages first."
        ),
        ReviewStanding::Approved => "approved: nothing is left to fix; wait for the merge.".to_owned(),
        ReviewStanding::NotInReview => "not in review yet.".to_owned(),
    }
}

/// The lines of the findings: every one up to [`FINDINGS_LISTED`], above that the open ones and a count of the others.
fn findings_text(findings: &BriefingFindings<'_>) -> String {
    match findings {
        BriefingFindings::Legacy { pull_request } => format!(
            "The review of this pull request kept no numbered findings: read its comments with gh pr view {pull_request} --comments."
        ),
        BriefingFindings::Ledger(ledger) if ledger.len() <= FINDINGS_LISTED => {
            ledger.iter().map(finding_line).collect::<Vec<_>>().join("\n")
        }
        BriefingFindings::Ledger(ledger) => {
            let (open, closed): (Vec<_>, Vec<_>) =
                ledger.iter().partition(|finding| finding.status == FindingStatus::Open);
            let mut lines: Vec<String> = open.into_iter().map(finding_line).collect();
            if !closed.is_empty() {
                lines.push(format!(
                    "{} resolved or not confirmed findings left out: see the pull request.",
                    closed.len()
                ));
            }
            lines.join("\n")
        }
    }
}

/// The line of one finding: its number, severity, place and text on one line, and where it stands.
fn finding_line(recorded: &RecordedFinding) -> String {
    let text = recorded
        .finding
        .text
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "- #{} [{}] {} — {} ({}; reported in pass {} by {})",
        recorded.number,
        recorded.finding.severity.word(),
        recorded.finding.place,
        text,
        status_word(recorded.status),
        recorded.pass,
        recorded.reporter
    )
}

/// The word that tells whether a finding is still to be fixed.
const fn status_word(status: FindingStatus) -> &'static str {
    match status {
        FindingStatus::Open => "open",
        FindingStatus::Resolved => "resolved",
        FindingStatus::NotConfirmed => "not confirmed",
    }
}

#[cfg(test)]
mod tests;
