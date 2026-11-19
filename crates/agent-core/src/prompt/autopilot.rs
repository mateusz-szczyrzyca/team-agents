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
    /// Independence instructions for first-pass reviewers without a bus.
    ReviewIndependent,
    /// A later pass checks findings and the changes since the preceding review.
    ReviewVerification,
    /// The diff command for an ancestor reviewed commit.
    ReviewVerificationChanges,
    /// The fallback diff when the lead rewrote the reviewed history.
    ReviewVerificationRewritten,
    /// The review the harness posts on the pull request for a reviewer's report.
    ReviewComment,
    /// The verdict line of an approved review.
    ReviewVerdictApprove,
    /// The verdict line of a review that requests changes.
    ReviewVerdictRequestChanges,
    /// What the lead of a team is told when the reviewers asked for changes.
    ReviewFixPass,
    /// The one-time explanation protocol.
    ReviewExplanationVote,
    /// The one-time explanation protocol.
    ReviewExplanation,
    /// The one-time explanation protocol.
    ReviewDisputes,
    /// The one-time explanation protocol.
    ReviewResponseInvalid,
    /// The one-time explanation protocol.
    ReviewFixResponse,
    /// One reviewer's report among the findings the lead is given.
    ReviewFinding,
    /// What the lead is told when the fix passes are used up: to ask the human.
    ReviewAskHuman,
    /// What a reviewer is told when its report has no verdict.
    ReviewVerdictMissing,
    /// Structured findings protocol and presentation.
    ReviewSeverityMinor,
    /// Structured findings protocol and presentation.
    ReviewSeverityMajor,
    /// Structured findings protocol and presentation.
    ReviewSeverityBlocker,
    /// Structured findings protocol and presentation.
    ReviewFindingsRow,
    /// Structured findings protocol and presentation.
    ReviewFindingsTable,
    /// Structured findings protocol and presentation.
    ReviewOpenFinding,
    /// Structured findings protocol and presentation.
    ReviewOpenFindings,
    /// Structured findings protocol and presentation.
    ReviewFindingsInvalid,
    /// The notification that a reviewer gave no verdict.
    NoticeNoVerdict,
    /// The notification that the reviewers still ask for changes and the human has to decide.
    NoticeDecision,
    /// Startup warning when disjoint verifier groups cannot be guaranteed.
    NoticeReviewRotation,
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
        Self::ReviewIndependent,
        Self::ReviewVerification,
        Self::ReviewVerificationChanges,
        Self::ReviewVerificationRewritten,
        Self::ReviewComment,
        Self::ReviewVerdictApprove,
        Self::ReviewVerdictRequestChanges,
        Self::ReviewFixPass,
        Self::ReviewExplanationVote,
        Self::ReviewExplanation,
        Self::ReviewDisputes,
        Self::ReviewResponseInvalid,
        Self::ReviewFixResponse,
        Self::ReviewFinding,
        Self::ReviewAskHuman,
        Self::ReviewVerdictMissing,
        Self::ReviewSeverityMinor,
        Self::ReviewSeverityMajor,
        Self::ReviewSeverityBlocker,
        Self::ReviewFindingsRow,
        Self::ReviewFindingsTable,
        Self::ReviewOpenFinding,
        Self::ReviewOpenFindings,
        Self::ReviewFindingsInvalid,
        Self::NoticeNoVerdict,
        Self::NoticeDecision,
        Self::NoticeReviewRotation,
        Self::NoticeMerged,
    ];

    /// Where the text is, who reads it and which variables it uses.
    #[must_use]
    pub const fn spec(self) -> PromptSpec {
        let (path, variables) = self.definition();
        let audience = if matches!(
            self,
            Self::NoticeTeamAsleep
                | Self::NoticeStandingItem
                | Self::NoticeApproved
                | Self::NoticePublishFailed
                | Self::NoticeMergeFailed
                | Self::NoticeClosed
                | Self::NoticeNoVerdict
                | Self::NoticeDecision
                | Self::NoticeReviewRotation
                | Self::NoticeMerged
        ) {
            Audience::Human
        } else {
            Audience::Model
        };
        PromptSpec::new(path, audience, variables)
    }

    /// The path, audience and variables of this text.
    const fn definition(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::PrTitle => ("harness/pr/title.md", &["item_ids", "item_titles"]),
            Self::PrBody => ("harness/pr/body.md", &["team", "repo", "gate", "items"]),
            Self::PrBodyItem => (
                "harness/pr/body-item.md",
                &["item_id", "item_title", "item_description"],
            ),
            Self::PrBodyExpected => ("harness/pr/body-expected.md", &["expected"]),
            Self::PrBodyChecks => ("harness/pr/body-checks.md", &["checks"]),
            Self::PrUpdate => ("harness/pr/update.md", &["head", "pass", "gate"]),
            Self::GateNotPassed => ("harness/pr/gate-not-passed.md", &["branch", "reason", "sent", "limit"]),
            Self::BranchNotJoined => (
                "harness/pr/branch-not-joined.md",
                &["branch", "missing", "sent", "limit"],
            ),
            Self::TeamAsleep => (
                "harness/pr/team-asleep.md",
                &["team", "items", "minutes", "tasks", "woken", "limit"],
            ),
            Self::NoticeTeamAsleep => (
                "harness/pr/notice-team-asleep.md",
                &["team", "items", "minutes", "tasks", "woken", "lead"],
            ),
            Self::NoticeStandingItem => (
                "harness/pr/notice-standing-item.md",
                &["team", "item", "member", "task", "line"],
            ),
            Self::NoticeApproved => ("harness/pr/notice-approved.md", &["team", "repo", "pr", "url"]),
            Self::NoticePublishFailed => ("harness/pr/notice-publish-failed.md", &["team", "items", "reason"]),
            Self::NoticeMergeFailed => ("harness/pr/notice-merge-failed.md", &["team", "pr", "url", "reason"]),
            Self::NoticeClosed => ("harness/pr/notice-closed.md", &["team", "pr", "url"]),
            Self::NoticeMerged => ("harness/pr/notice-merged.md", &["team", "pr", "url", "items"]),
            Self::ReviewRequest => (
                "harness/review/request.md",
                &["team", "pr", "branch", "base", "head", "pass"],
            ),
            Self::ReviewVerification => (
                "harness/review/verification.md",
                &[
                    "team",
                    "pr",
                    "branch",
                    "base",
                    "head",
                    "pass",
                    "changes",
                    "lead_message",
                ],
            ),
            Self::ReviewVerificationChanges => ("harness/review/verification-changes.md", &["previous"]),
            Self::ReviewVerificationRewritten => ("harness/review/verification-rewritten.md", &["base"]),
            Self::ReviewIndependent => ("harness/review/independent.md", &[]),
            Self::ReviewComment => (
                "harness/review/comment.md",
                &["reviewer", "client", "verdict", "pass", "commit", "report"],
            ),
            Self::ReviewVerdictApprove => ("harness/review/verdict-approve.md", &["icon"]),
            Self::ReviewVerdictRequestChanges => ("harness/review/verdict-request-changes.md", &["icon"]),
            Self::ReviewFixPass => (
                "harness/review/fix-pass.md",
                &["pr", "pass", "passes", "branch", "findings"],
            ),
            Self::ReviewFixResponse => (
                "harness/review/fix-response.md",
                &["pr", "pass", "passes", "branch", "findings"],
            ),
            Self::ReviewResponseInvalid => ("harness/review/response-invalid.md", &["line", "number", "allowed"]),
            Self::ReviewDisputes => ("harness/review/disputes.md", &["findings"]),
            Self::ReviewExplanation => ("harness/review/explanation.md", &["status", "why", "votes"]),
            Self::ReviewExplanationVote => ("harness/review/explanation-vote.md", &["reviewer", "status", "why"]),
            Self::ReviewFinding => ("harness/review/finding.md", &["reviewer", "report"]),
            Self::ReviewAskHuman => (
                "harness/review/ask-human.md",
                &[
                    "pr",
                    "passes",
                    "additional_passes",
                    "findings",
                    "questions_file",
                    "number",
                    "task",
                ],
            ),
            Self::ReviewFindingsInvalid => ("harness/review/findings-invalid.md", &["line", "reason"]),
            Self::ReviewOpenFindings => ("harness/review/open-findings.md", &["findings"]),
            Self::ReviewOpenFinding => (
                "harness/review/open-finding.md",
                &["number", "severity", "rule", "place", "text", "check"],
            ),
            Self::ReviewFindingsTable => ("harness/review/findings-table.md", &["rows"]),
            Self::ReviewFindingsRow => (
                "harness/review/findings-row.md",
                &["number", "severity", "rule", "place", "text", "status", "reporter"],
            ),
            Self::ReviewSeverityBlocker => ("harness/review/severity-blocker.md", &[]),
            Self::ReviewSeverityMajor => ("harness/review/severity-major.md", &[]),
            Self::ReviewSeverityMinor => ("harness/review/severity-minor.md", &[]),
            Self::ReviewVerdictMissing => ("harness/review/verdict-missing.md", &[]),
            Self::NoticeNoVerdict => (
                "harness/review/notice-no-verdict.md",
                &["reviewer", "pr", "task", "why"],
            ),
            Self::NoticeReviewRotation => ("harness/review/notice-rotation.md", &["reviews"]),
            Self::NoticeDecision => (
                "harness/review/notice-decision.md",
                &["team", "pr", "passes", "additional_passes", "findings"],
            ),
        }
    }
}
