//! The pull request of the task's item and its review, as the briefing of a new session tells them: the pull request, where
//! the review stands and the findings the reviews recorded.

use std::num::NonZeroU32;

use agent_core::state::RecordedFinding;

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

/// The values of the briefing's review variables for `review`.
pub(super) fn review_values(_review: &BriefingReview<'_>) -> ReviewValues {
    ReviewValues {
        pull_request: String::new(),
        review_state: String::new(),
        findings: String::new(),
    }
}

#[cfg(test)]
mod tests;
