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
    /// Describe an idle round in status output.
    WaitNothing,
    /// Describe an idle round in status output.
    WaitCorrection,
    /// Describe an idle round in status output.
    WaitHuman,
    /// Describe an idle round in status output.
    WaitOpinion,
    /// Describe an idle round in status output.
    WaitReview,
    /// Describe an idle round in status output.
    WaitGate,
    /// Ask an independent investigator to check one finding at the handed-over commit.
    ReviewOpinionRequest,
    /// Return an unchanged fix pass that requires a commit.
    ReviewFixNoCommit,
    /// Explain why the unchanged fix pass cannot enter verification.
    ReviewFixNoCommitReason,
    /// The missing outcome named in an exhausted handover notice.
    ReviewFixNoCommitUndone,
    /// Ask the human to recover a handover after its corrections are exhausted.
    NoticeHandoverHeld,
    /// Return an opinion report that lacks a verdict or supporting evidence.
    ReviewOpinionCorrection,
    /// Offer the permitted second-opinion answer in a fix-pass prompt.
    ReviewOpinionOffer,
    /// Show the investigator, outcome and evidence beside a finding.
    ReviewOpinionResult,
    /// Return confirmed blockers to the lead for fixes in the current pass.
    ReviewOpinionConfirmed,
    /// Describe the allowed answers for an unused blocker with permission enabled.
    ReviewResponseOpinion,
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
    /// Measured pull request size.
    PrSize,
    /// Task-authored path allowances used by this round.
    PrAllowedBeforehand,
    /// Gate provenance for a pull request.
    PrGate,
    /// A passing retry after a failed gate.
    PrGateRetry,
    /// Missing lead contribution.
    PrNoLead,
    /// Instructions for the lead contribution file.
    PrLeadInstructions,
    /// Matching-head gate handed to a reviewer.
    ReviewGate,
    /// Missing matching-head gate.
    ReviewGateMissing,
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
    /// The lead repeatedly supplied an invalid fix response.
    NoticeResponseRefused,
    /// A verdict was present but the reviewer report was refused.
    NoticeReportRefused,
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
    /// The verdict answer correction.
    ReviewMistakeVerdict,
    /// The malformed answer correction.
    ReviewMistakeMalformed,
    /// The after-findings answer correction.
    ReviewMistakeAfterFindings,
    /// The missing answer correction.
    ReviewMistakeMissing,
    /// The forbidden answer correction.
    ReviewMistakeForbidden,
    /// The after-status answer correction.
    ReviewMistakeAfterStatus,
    /// The duplicate answer correction.
    ReviewMistakeDuplicate,
    /// The unknown answer correction.
    ReviewMistakeUnknown,
    /// Permitted increases from minor.
    ReviewRaiseMajor,
    /// Permitted increase from major to blocker.
    ReviewRaiseBlocker,
    /// Verifier forms for a pending explanation.
    ReviewVerifierPending,
    /// Verifier forms without a pending explanation.
    ReviewVerifierResolved,
    /// One numbered permitted answer.
    ReviewResponseForm,
    /// Source evidence and all available answers after a refused line.
    ReviewAnswerDiagnostic,
    /// The same for a refused line whose label is no finding number.
    ReviewAnswerDiagnosticUnnumbered,
    /// Correction for a refused line whose label is no finding number.
    ReviewResponseUnnumbered,
    /// Correction for an unknown blocking finding number.
    ReviewResponseNumbers,
    /// Correction allowing an unused major explanation.
    ReviewResponseExplain,
    /// Correction requiring a code fix.
    ReviewResponseFixed,
    /// The one-time explanation protocol.
    ReviewFixResponse,
    /// Tell a lead woken during an open fix pass that the pass is still open and its response is due.
    ReviewFixPassOpen,
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
    /// Retain an unsuccessful opinion assignment in the human notice.
    OpinionAttemptFailed,
    /// Retain an unsuccessful opinion assignment in the human notice.
    OpinionAttemptNoStatus,
    /// Retain an unsuccessful opinion assignment in the human notice.
    OpinionAttemptMissingReport,
    /// Retain an unsuccessful opinion assignment in the human notice.
    OpinionAttemptSilent,
    /// Retain an unsuccessful opinion assignment in the human notice.
    OpinionAttemptInterrupted,
    /// An unanswered opinion exhausted its automatic replacements.
    NoticeOpinion,
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
        Self::ReviewOpinionRequest,
        Self::WaitGate,
        Self::WaitReview,
        Self::WaitOpinion,
        Self::WaitHuman,
        Self::WaitCorrection,
        Self::WaitNothing,
        Self::ReviewFixNoCommit,
        Self::ReviewFixNoCommitReason,
        Self::ReviewFixNoCommitUndone,
        Self::NoticeHandoverHeld,
        Self::ReviewOpinionCorrection,
        Self::ReviewOpinionOffer,
        Self::ReviewOpinionResult,
        Self::ReviewOpinionConfirmed,
        Self::ReviewResponseOpinion,
        Self::PrTitle,
        Self::PrBody,
        Self::PrBodyItem,
        Self::PrBodyExpected,
        Self::PrBodyChecks,
        Self::PrUpdate,
        Self::PrSize,
        Self::PrAllowedBeforehand,
        Self::PrGate,
        Self::PrGateRetry,
        Self::PrNoLead,
        Self::PrLeadInstructions,
        Self::ReviewGate,
        Self::ReviewGateMissing,
        Self::GateNotPassed,
        Self::BranchNotJoined,
        Self::TeamAsleep,
        Self::NoticeTeamAsleep,
        Self::NoticeStandingItem,
        Self::NoticeApproved,
        Self::NoticePublishFailed,
        Self::NoticeResponseRefused,
        Self::NoticeReportRefused,
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
        Self::ReviewMistakeVerdict,
        Self::ReviewMistakeMalformed,
        Self::ReviewMistakeAfterFindings,
        Self::ReviewMistakeMissing,
        Self::ReviewMistakeForbidden,
        Self::ReviewMistakeAfterStatus,
        Self::ReviewMistakeDuplicate,
        Self::ReviewMistakeUnknown,
        Self::ReviewRaiseMajor,
        Self::ReviewRaiseBlocker,
        Self::ReviewVerifierPending,
        Self::ReviewVerifierResolved,
        Self::ReviewResponseForm,
        Self::ReviewAnswerDiagnostic,
        Self::ReviewAnswerDiagnosticUnnumbered,
        Self::ReviewResponseUnnumbered,
        Self::ReviewResponseNumbers,
        Self::ReviewResponseExplain,
        Self::ReviewResponseFixed,
        Self::ReviewFixResponse,
        Self::ReviewFixPassOpen,
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
        Self::OpinionAttemptFailed,
        Self::OpinionAttemptNoStatus,
        Self::OpinionAttemptMissingReport,
        Self::OpinionAttemptSilent,
        Self::OpinionAttemptInterrupted,
        Self::NoticeOpinion,
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
            Self::NoticeHandoverHeld
                | Self::NoticeTeamAsleep
                | Self::NoticeStandingItem
                | Self::NoticeApproved
                | Self::NoticeResponseRefused
                | Self::NoticeReportRefused
                | Self::NoticePublishFailed
                | Self::NoticeMergeFailed
                | Self::NoticeClosed
                | Self::NoticeNoVerdict
                | Self::NoticeOpinion
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
        (self.path(), self.variables())
    }

    /// The catalog file of this text.
    const fn path(self) -> &'static str {
        match self {
            Self::WaitNothing => "harness/pr/wait-nothing.md",
            Self::WaitCorrection => "harness/pr/wait-correction.md",
            Self::WaitHuman => "harness/pr/wait-human.md",
            Self::WaitOpinion => "harness/pr/wait-opinion.md",
            Self::WaitReview => "harness/pr/wait-review.md",
            Self::WaitGate => "harness/pr/wait-gate.md",
            Self::ReviewFixNoCommit => "harness/review/fix-no-commit.md",
            Self::ReviewFixNoCommitReason => "harness/review/fix-no-commit-reason.md",
            Self::ReviewFixNoCommitUndone => "harness/review/fix-no-commit-undone.md",
            Self::NoticeHandoverHeld => "harness/pr/notice-handover-held.md",
            Self::ReviewOpinionRequest => "harness/review/opinion-request.md",
            Self::ReviewOpinionCorrection => "harness/review/opinion-correction.md",
            Self::ReviewOpinionOffer => "harness/review/opinion-offer.md",
            Self::ReviewOpinionResult => "harness/review/opinion-result.md",
            Self::ReviewOpinionConfirmed => "harness/review/opinion-confirmed.md",
            Self::ReviewResponseOpinion => "harness/review/response-opinion.md",
            Self::PrTitle => "harness/pr/title.md",
            Self::PrBody => "harness/pr/body.md",
            Self::PrBodyItem => "harness/pr/body-item.md",
            Self::PrBodyExpected => "harness/pr/body-expected.md",
            Self::PrBodyChecks => "harness/pr/body-checks.md",
            Self::PrUpdate => "harness/pr/update.md",
            Self::PrSize => "harness/pr/size.md",
            Self::PrAllowedBeforehand => "harness/pr/allowed-beforehand.md",
            Self::PrGate => "harness/pr/gate.md",
            Self::PrGateRetry => "harness/pr/gate-retry.md",
            Self::PrNoLead => "harness/pr/no-lead.md",
            Self::PrLeadInstructions => "harness/pr/lead-description.md",
            Self::ReviewGate => "harness/review/gate.md",
            Self::ReviewGateMissing => "harness/review/gate-missing.md",
            Self::GateNotPassed => "harness/pr/gate-not-passed.md",
            Self::BranchNotJoined => "harness/pr/branch-not-joined.md",
            Self::TeamAsleep => "harness/pr/team-asleep.md",
            Self::NoticeTeamAsleep => "harness/pr/notice-team-asleep.md",
            Self::NoticeStandingItem => "harness/pr/notice-standing-item.md",
            Self::NoticeApproved => "harness/pr/notice-approved.md",
            Self::NoticePublishFailed => "harness/pr/notice-publish-failed.md",
            Self::NoticeResponseRefused => "harness/pr/notice-response-refused.md",
            Self::NoticeReportRefused => "harness/review/notice-report-refused.md",
            Self::NoticeMergeFailed => "harness/pr/notice-merge-failed.md",
            Self::NoticeClosed => "harness/pr/notice-closed.md",
            Self::NoticeMerged => "harness/pr/notice-merged.md",
            Self::ReviewRequest => "harness/review/request.md",
            Self::ReviewVerification => "harness/review/verification.md",
            Self::ReviewVerificationChanges => "harness/review/verification-changes.md",
            Self::ReviewVerificationRewritten => "harness/review/verification-rewritten.md",
            Self::ReviewIndependent => "harness/review/independent.md",
            Self::ReviewComment => "harness/review/comment.md",
            Self::ReviewVerdictApprove => "harness/review/verdict-approve.md",
            Self::ReviewVerdictRequestChanges => "harness/review/verdict-request-changes.md",
            Self::ReviewFixPass => "harness/review/fix-pass.md",
            Self::ReviewFixResponse => "harness/review/fix-response.md",
            Self::ReviewFixPassOpen => "harness/review/fix-pass-open.md",
            Self::ReviewResponseNumbers => "harness/review/response-numbers.md",
            Self::ReviewResponseExplain => "harness/review/response-explain.md",
            Self::ReviewResponseFixed => "harness/review/response-fixed.md",
            Self::ReviewResponseInvalid => "harness/review/response-invalid.md",
            Self::ReviewMistakeVerdict => "harness/review/mistake-verdict.md",
            Self::ReviewMistakeMalformed => "harness/review/mistake-malformed.md",
            Self::ReviewMistakeAfterFindings => "harness/review/mistake-after-findings.md",
            Self::ReviewMistakeMissing => "harness/review/mistake-missing.md",
            Self::ReviewMistakeForbidden => "harness/review/mistake-forbidden.md",
            Self::ReviewMistakeAfterStatus => "harness/review/mistake-after-status.md",
            Self::ReviewMistakeDuplicate => "harness/review/mistake-duplicate.md",
            Self::ReviewMistakeUnknown => "harness/review/mistake-unknown.md",
            Self::ReviewRaiseMajor => "harness/review/raise-major.md",
            Self::ReviewRaiseBlocker => "harness/review/raise-blocker.md",
            Self::ReviewVerifierPending => "harness/review/verifier-pending.md",
            Self::ReviewVerifierResolved => "harness/review/verifier-resolved.md",
            Self::ReviewResponseForm => "harness/review/response-form.md",
            Self::ReviewAnswerDiagnostic => "harness/review/answer-diagnostic.md",
            Self::ReviewAnswerDiagnosticUnnumbered => "harness/review/answer-diagnostic-unnumbered.md",
            Self::ReviewResponseUnnumbered => "harness/review/response-unnumbered.md",
            Self::ReviewDisputes => "harness/review/disputes.md",
            Self::ReviewExplanation => "harness/review/explanation.md",
            Self::ReviewExplanationVote => "harness/review/explanation-vote.md",
            Self::ReviewFinding => "harness/review/finding.md",
            Self::ReviewAskHuman => "harness/review/ask-human.md",
            Self::ReviewFindingsInvalid => "harness/review/findings-invalid.md",
            Self::ReviewOpenFindings => "harness/review/open-findings.md",
            Self::ReviewOpenFinding => "harness/review/open-finding.md",
            Self::ReviewFindingsTable => "harness/review/findings-table.md",
            Self::ReviewFindingsRow => "harness/review/findings-row.md",
            Self::ReviewSeverityBlocker => "harness/review/severity-blocker.md",
            Self::ReviewSeverityMajor => "harness/review/severity-major.md",
            Self::ReviewSeverityMinor => "harness/review/severity-minor.md",
            Self::ReviewVerdictMissing => "harness/review/verdict-missing.md",
            Self::OpinionAttemptFailed => "harness/review/opinion-attempt-failed.md",
            Self::OpinionAttemptNoStatus => "harness/review/opinion-attempt-nostatus.md",
            Self::OpinionAttemptMissingReport => "harness/review/opinion-attempt-missingreport.md",
            Self::OpinionAttemptSilent => "harness/review/opinion-attempt-silent.md",
            Self::OpinionAttemptInterrupted => "harness/review/opinion-attempt-interrupted.md",
            Self::NoticeOpinion => "harness/review/notice-opinion.md",
            Self::NoticeNoVerdict => "harness/review/notice-no-verdict.md",
            Self::NoticeReviewRotation => "harness/review/notice-rotation.md",
            Self::NoticeDecision => "harness/review/notice-decision.md",
        }
    }

    /// The context required to render this text.
    const fn variables(self) -> &'static [&'static str] {
        match self {
            Self::PrGateRetry
            | Self::PrNoLead
            | Self::ReviewFixNoCommitUndone
            | Self::WaitNothing
            | Self::WaitCorrection
            | Self::WaitHuman
            | Self::WaitOpinion
            | Self::WaitReview
            | Self::WaitGate
            | Self::ReviewOpinionCorrection
            | Self::ReviewOpinionOffer
            | Self::ReviewResponseOpinion
            | Self::ReviewIndependent
            | Self::ReviewResponseNumbers
            | Self::ReviewResponseExplain
            | Self::ReviewResponseFixed
            | Self::ReviewSeverityBlocker
            | Self::ReviewSeverityMajor
            | Self::ReviewSeverityMinor
            | Self::ReviewMistakeVerdict
            | Self::ReviewMistakeMalformed
            | Self::ReviewMistakeAfterFindings
            | Self::ReviewMistakeMissing
            | Self::ReviewMistakeForbidden
            | Self::ReviewMistakeAfterStatus
            | Self::ReviewMistakeDuplicate
            | Self::ReviewMistakeUnknown
            | Self::ReviewRaiseMajor
            | Self::ReviewRaiseBlocker
            | Self::ReviewVerifierPending
            | Self::ReviewVerifierResolved
            | Self::ReviewVerdictMissing => &[],
            Self::ReviewFixNoCommit => &["branch", "tip", "sent", "limit"],
            Self::ReviewFixNoCommitReason => &["tip"],
            Self::NoticeHandoverHeld => &["team", "items", "reason", "rounds", "task"],
            Self::ReviewOpinionRequest => OPINION_REQUEST_VARIABLES,
            Self::ReviewOpinionResult => &["reviewer", "outcome", "evidence"],
            Self::ReviewOpinionConfirmed | Self::ReviewDisputes | Self::ReviewOpenFindings => &["findings"],
            Self::PrTitle => &["item_ids", "item_titles"],
            Self::PrBody => &["team", "repo", "gate", "items"],
            Self::PrBodyItem => PR_ITEM_VARIABLES,
            Self::PrBodyExpected => &["expected"],
            Self::PrBodyChecks => &["checks"],
            Self::PrUpdate => &["head", "pass", "gate"],
            Self::PrSize => &["size"],
            Self::PrAllowedBeforehand => &["paths"],
            Self::PrGate | Self::ReviewGate => &["head", "finished", "result", "log"],
            Self::PrLeadInstructions => &["description_file"],
            Self::ReviewGateMissing => &["head"],
            Self::GateNotPassed => &["branch", "reason", "sent", "limit"],
            Self::BranchNotJoined => BRANCH_VARIABLES,
            Self::TeamAsleep => &["team", "items", "minutes", "tasks", "woken", "limit"],
            Self::NoticeTeamAsleep => &["team", "items", "minutes", "tasks", "woken", "lead"],
            Self::NoticeStandingItem => &["team", "item", "member", "task", "line"],
            Self::NoticeApproved => &["team", "repo", "pr", "url"],
            Self::NoticePublishFailed => &["team", "items", "reason"],
            Self::NoticeResponseRefused | Self::NoticeReportRefused => {
                &["team", "items", "reason", "bounces", "task", "reviewer", "pr"]
            }
            Self::NoticeMergeFailed => &["team", "pr", "url", "reason"],
            Self::NoticeClosed => &["team", "pr", "url"],
            Self::NoticeMerged => &["team", "pr", "url", "items"],
            Self::ReviewRequest => &["team", "pr", "branch", "base", "head", "pass"],
            Self::ReviewVerification => VERIFICATION_VARIABLES,
            Self::ReviewVerificationChanges => &["previous"],
            Self::ReviewVerificationRewritten => &["base"],
            Self::ReviewComment => &["reviewer", "client", "verdict", "pass", "commit", "report"],
            Self::ReviewVerdictApprove | Self::ReviewVerdictRequestChanges => &["icon"],
            Self::ReviewFixPass | Self::ReviewFixResponse => FIX_PASS_VARIABLES,
            Self::ReviewFixPassOpen => &["pr", "pass"],
            Self::ReviewResponseInvalid => &["line", "number", "allowed"],
            Self::ReviewResponseForm => &["number", "allowed"],
            Self::ReviewAnswerDiagnostic => &["number", "source", "mistake", "forms"],
            Self::ReviewAnswerDiagnosticUnnumbered => &["source", "mistake", "forms"],
            Self::ReviewResponseUnnumbered => &["line"],
            Self::ReviewExplanation => &["status", "why", "votes"],
            Self::ReviewExplanationVote => &["reviewer", "status", "why"],
            Self::ReviewFinding => &["reviewer", "report"],
            Self::ReviewAskHuman => ASK_HUMAN_VARIABLES,
            Self::ReviewFindingsInvalid => &["line", "reason"],
            Self::ReviewOpenFinding => &["number", "severity", "rule", "place", "text", "check"],
            Self::ReviewFindingsTable => &["rows"],
            Self::ReviewFindingsRow => &["number", "severity", "rule", "place", "text", "status", "reporter"],
            Self::OpinionAttemptFailed
            | Self::OpinionAttemptNoStatus
            | Self::OpinionAttemptMissingReport
            | Self::OpinionAttemptSilent
            | Self::OpinionAttemptInterrupted => OPINION_ATTEMPT_VARIABLES,
            Self::NoticeOpinion => OPINION_NOTICE_VARIABLES,
            Self::NoticeNoVerdict => &["reviewer", "pr", "task", "why"],
            Self::NoticeReviewRotation => &["reviews"],
            Self::NoticeDecision => &["team", "pr", "passes", "additional_passes", "findings"],
        }
    }
}

/// Context supplied to an incremental verification request.
const VERIFICATION_VARIABLES: &[&str] = &[
    "team",
    "pr",
    "branch",
    "base",
    "head",
    "pass",
    "changes",
    "lead_message",
];
/// Context supplied when the review budget needs a human decision.
const ASK_HUMAN_VARIABLES: &[&str] = &[
    "pr",
    "passes",
    "additional_passes",
    "findings",
    "questions_file",
    "number",
    "task",
];

/// Context identifying an unsuccessful opinion assignment.
const OPINION_ATTEMPT_VARIABLES: &[&str] = &["investigator", "task"];
/// Context explaining the exhausted opinion request to the human.
const OPINION_NOTICE_VARIABLES: &[&str] = &["round", "number", "task", "attempts"];
/// Finding and reviewed head supplied to an independent investigator.
const OPINION_REQUEST_VARIABLES: &[&str] = &["number", "severity", "rule", "place", "text", "check", "reason", "head"];

/// Required context of this catalog entry.
const PR_ITEM_VARIABLES: &[&str] = &["item_id", "item_title", "item_description"];

/// Required context of this catalog entry.
const BRANCH_VARIABLES: &[&str] = &["branch", "missing", "sent", "limit"];

/// Required context of this catalog entry.
const FIX_PASS_VARIABLES: &[&str] = &["pr", "pass", "passes", "branch", "findings"];
