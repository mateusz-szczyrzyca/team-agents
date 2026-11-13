//! The texts of the autopilot: what is written on a pull request, what a reviewer and the team are told, and what the
//! human is told about a pull request (`prompts/harness/pr/<name>.md` and `prompts/harness/review/<name>.md`).
//!
//! They are a fourth catalog beside [`PromptId`](super::PromptId), [`HarnessPrompt`](super::HarnessPrompt) and
//! [`TeamPrompt`](super::TeamPrompt), with the same shape — a file under [`PROMPTS_DIR`](super::PROMPTS_DIR), an
//! [`Audience`] and the variables its text uses — and two directories of their own, which the integration test
//! `tests/harness_autopilot_prompts.rs` checks against the entries. What goes on GitHub is English only, so it is for
//! [`Audience::Model`]; only the notifications (`notice-*.md`) are for the human.

use super::{Audience, PromptSpec};

/// Every text of the autopilot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AutopilotPrompt {
    /// The title of a pull request.
    PrTitle,
    /// The description of a pull request.
    PrBody,
    /// One item in the description of a pull request.
    PrBodyItem,
    /// The expected result of an item, in the description of a pull request.
    PrBodyExpected,
    /// The checks of an item, in the description of a pull request.
    PrBodyChecks,
    /// The comment that tells the reviewers a fix pass updated the pull request.
    PrUpdate,
    /// The notification that a pull request is approved and waits for the human to merge it.
    NoticeApproved,
    /// The notification that a pull request could not be opened or updated.
    NoticePublishFailed,
    /// The notification that an approved pull request could not be merged.
    NoticeMergeFailed,
    /// What a reviewer is told about the pull request it reviews.
    ReviewRequest,
    /// The review the harness posts on the pull request for a reviewer's report.
    ReviewComment,
    /// What the lead of a team is told when the reviewers asked for changes.
    ReviewFixPass,
    /// What the lead is told when the fix passes are used up: to ask the human.
    ReviewAskHuman,
    /// What a reviewer is told when its report has no verdict.
    ReviewVerdictMissing,
    /// The notification that a reviewer gave no verdict.
    NoticeNoVerdict,
    /// The notification that the reviewers still ask for changes and the human has to decide.
    NoticeDecision,
}

impl AutopilotPrompt {
    /// Every text of the catalog.
    pub const ALL: &'static [Self] = &[
        Self::PrTitle,
        Self::PrBody,
        Self::PrBodyItem,
        Self::PrBodyExpected,
        Self::PrBodyChecks,
        Self::PrUpdate,
        Self::NoticeApproved,
        Self::NoticePublishFailed,
        Self::NoticeMergeFailed,
        Self::ReviewRequest,
        Self::ReviewComment,
        Self::ReviewFixPass,
        Self::ReviewAskHuman,
        Self::ReviewVerdictMissing,
        Self::NoticeNoVerdict,
        Self::NoticeDecision,
    ];

    /// Where the text is, who reads it and which variables it uses.
    #[must_use]
    pub const fn spec(self) -> PromptSpec {
        let _ = self;
        PromptSpec::new("harness/pr/missing.md", Audience::Model, &[])
    }
}
