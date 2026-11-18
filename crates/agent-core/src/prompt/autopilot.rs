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
    /// What the lead is told when it is done and the gate has not passed at the tip of its branch.
    GateNotPassed,
    /// What the lead is told when it is done, the gate passed, and its branch contains none of the finished items.
    BranchNotJoined,
    /// What the lead is told when every task of its team sleeps and an item is unfinished.
    TeamAsleep,
    /// The notification that a team sleeps with work unfinished after its lead was woken for it.
    NoticeTeamAsleep,
    /// The notification that a team waits for a standing item behind a failed member while its lead is done.
    NoticeStandingItem,
    /// The notification that a pull request is approved and waits for the human to merge it.
    NoticeApproved,
    /// The notification that a pull request could not be opened or updated.
    NoticePublishFailed,
    /// The notification that an approved pull request could not be merged.
    NoticeMergeFailed,
    /// The notification that a pull request was closed without being merged.
    NoticeClosed,
    /// What a reviewer is told about the pull request it reviews.
    ReviewRequest,
    /// The review the harness posts on the pull request for a reviewer's report.
    ReviewComment,
    /// The verdict line of an approved review.
    ReviewVerdictApprove,
    /// The verdict line of a review that requests changes.
    ReviewVerdictRequestChanges,
    /// What the lead of a team is told when the reviewers asked for changes.
    ReviewFixPass,
    /// One reviewer's report among the findings the lead is given.
    ReviewFinding,
    /// What the lead is told when the fix passes are used up: to ask the human.
    ReviewAskHuman,
    /// What a reviewer is told when its report has no verdict.
    ReviewVerdictMissing,
    /// The notification that a reviewer gave no verdict.
    NoticeNoVerdict,
    /// The notification that the reviewers still ask for changes and the human has to decide.
    NoticeDecision,
    /// The notification that a pull request was merged.
    NoticeMerged,
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
        Self::GateNotPassed,
        Self::BranchNotJoined,
        Self::TeamAsleep,
        Self::NoticeTeamAsleep,
        Self::NoticeStandingItem,
        Self::NoticeApproved,
        Self::NoticePublishFailed,
        Self::NoticeMergeFailed,
        Self::NoticeClosed,
        Self::ReviewRequest,
        Self::ReviewComment,
        Self::ReviewVerdictApprove,
        Self::ReviewVerdictRequestChanges,
        Self::ReviewFixPass,
        Self::ReviewFinding,
        Self::ReviewAskHuman,
        Self::ReviewVerdictMissing,
        Self::NoticeNoVerdict,
        Self::NoticeDecision,
        Self::NoticeMerged,
    ];

    /// Where the text is, who reads it and which variables it uses.
    #[must_use]
    pub const fn spec(self) -> PromptSpec {
        let (path, audience, variables) = self.definition();
        PromptSpec::new(path, audience, variables)
    }

    /// The path, audience and variables of this text.
    const fn definition(self) -> (&'static str, Audience, &'static [&'static str]) {
        match self {
            Self::PrTitle => ("harness/pr/title.md", Audience::Model, &["item_ids", "item_titles"]),
            Self::PrBody => (
                "harness/pr/body.md",
                Audience::Model,
                &["team", "repo", "gate", "items"],
            ),
            Self::PrBodyItem => (
                "harness/pr/body-item.md",
                Audience::Model,
                &["item_id", "item_title", "item_description"],
            ),
            Self::PrBodyExpected => ("harness/pr/body-expected.md", Audience::Model, &["expected"]),
            Self::PrBodyChecks => ("harness/pr/body-checks.md", Audience::Model, &["checks"]),
            Self::PrUpdate => ("harness/pr/update.md", Audience::Model, &["head", "pass", "gate"]),
            Self::GateNotPassed => (
                "harness/pr/gate-not-passed.md",
                Audience::Model,
                &["branch", "reason", "sent", "limit"],
            ),
            Self::BranchNotJoined => (
                "harness/pr/branch-not-joined.md",
                Audience::Model,
                &["branch", "missing", "sent", "limit"],
            ),
            Self::TeamAsleep => (
                "harness/pr/team-asleep.md",
                Audience::Model,
                &["team", "items", "minutes", "tasks", "woken", "limit"],
            ),
            Self::NoticeTeamAsleep => (
                "harness/pr/notice-team-asleep.md",
                Audience::Human,
                &["team", "items", "minutes", "tasks", "woken", "lead"],
            ),
            Self::NoticeStandingItem => (
                "harness/pr/notice-standing-item.md",
                Audience::Human,
                &["team", "item", "member", "task", "lead"],
            ),
            Self::NoticeApproved => (
                "harness/pr/notice-approved.md",
                Audience::Human,
                &["team", "repo", "pr", "url"],
            ),
            Self::NoticePublishFailed => (
                "harness/pr/notice-publish-failed.md",
                Audience::Human,
                &["team", "items", "reason"],
            ),
            Self::NoticeMergeFailed => (
                "harness/pr/notice-merge-failed.md",
                Audience::Human,
                &["team", "pr", "url", "reason"],
            ),
            Self::NoticeClosed => ("harness/pr/notice-closed.md", Audience::Human, &["team", "pr", "url"]),
            Self::NoticeMerged => (
                "harness/pr/notice-merged.md",
                Audience::Human,
                &["team", "pr", "url", "items"],
            ),
            Self::ReviewRequest => (
                "harness/review/request.md",
                Audience::Model,
                &["team", "pr", "branch", "base", "head", "pass"],
            ),
            Self::ReviewComment => (
                "harness/review/comment.md",
                Audience::Model,
                &["reviewer", "client", "verdict", "pass", "commit", "report"],
            ),
            Self::ReviewVerdictApprove => ("harness/review/verdict-approve.md", Audience::Model, &["icon"]),
            Self::ReviewVerdictRequestChanges => {
                ("harness/review/verdict-request-changes.md", Audience::Model, &["icon"])
            }
            Self::ReviewFixPass => (
                "harness/review/fix-pass.md",
                Audience::Model,
                &["pr", "pass", "passes", "branch", "findings"],
            ),
            Self::ReviewFinding => ("harness/review/finding.md", Audience::Model, &["reviewer", "report"]),
            Self::ReviewAskHuman => (
                "harness/review/ask-human.md",
                Audience::Model,
                &["pr", "passes", "findings", "questions_file", "number", "task"],
            ),
            Self::ReviewVerdictMissing => ("harness/review/verdict-missing.md", Audience::Model, &[]),
            Self::NoticeNoVerdict => (
                "harness/review/notice-no-verdict.md",
                Audience::Human,
                &["reviewer", "pr", "task", "why"],
            ),
            Self::NoticeDecision => (
                "harness/review/notice-decision.md",
                Audience::Human,
                &["team", "pr", "passes"],
            ),
        }
    }
}
