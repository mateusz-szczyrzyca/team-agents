//! The texts of the checks before a review: what the team is told when its round goes back, the reasons the checks give,
//! the questions to the human about what only they may allow, and what the lead is told around them
//! (`prompts/harness/checks/<name>.md`).
//!
//! They are a fifth catalog beside [`PromptId`](super::PromptId), [`HarnessPrompt`](super::HarnessPrompt),
//! [`TeamPrompt`](super::TeamPrompt) and [`AutopilotPrompt`](super::AutopilotPrompt), with the same shape and a directory
//! of its own, which the integration test `tests/harness_checks_prompts.rs` checks against the entries. The reasons and
//! what the lead is told are for a model, in English; the questions and the notification are for the human, who may have
//! a version of them in their language.

use super::{Audience, PromptSpec};

/// Every text of the checks before a review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChecksPrompt {
    /// The reason: an existing test file was deleted.
    IssueTestDeleted,
    /// The reason: an existing test file was renamed.
    IssueTestRenamed,
    /// The reason: lines were taken out of an existing test file.
    IssueTestEdited,
    /// The reason: an existing binary test file was replaced.
    IssueTestBinary,
    /// The reason: the mode of an existing test file changed.
    IssueTestMode,
    /// The reason: a protected file was changed.
    IssueProtected,
    /// The reason: a test of a `RED:` commit was edited by a later commit.
    IssueRedEdited,
    /// The reason: production code changed and no test line was added.
    IssueNoTests,
    /// The reason: added lines skip a test.
    IssueSkipsTest,
    /// The reason: added lines suppress a lint.
    IssueSuppressesLint,
    /// The reason: added lines ask whether the code runs under test.
    IssueChecksUnderTest,
    /// What the lead is told when the round goes back to the team.
    TeamPass,
    /// The part of that message about what the human refused to allow.
    Refused,
    /// What the lead is told to put the harness's questions to the human.
    AskRelay,
    /// What the lead is told when the human answered those questions.
    Answered,
    /// The title of the questions file.
    QuestionsTitle,
    /// The title of a question about a file.
    QuestionTitlePath,
    /// The title of the question about the lack of tests.
    QuestionTitleNoTests,
    /// The text of a question: why the human is asked, the issues and how to answer.
    QuestionBody,
    /// The recommendation of a question.
    QuestionRecommend,
    /// Why the human is asked when only they may allow what was found.
    WhyOnlyHuman,
    /// Why the human is asked when the passes of the team are used.
    WhyPassesUsed,
    /// The notification that the lead did not put the question to the human.
    NoticeNotAsked,
}

impl ChecksPrompt {
    /// Every text of the catalog.
    pub const ALL: &'static [Self] = &[
        Self::IssueTestDeleted,
        Self::IssueTestRenamed,
        Self::IssueTestEdited,
        Self::IssueTestBinary,
        Self::IssueTestMode,
        Self::IssueProtected,
        Self::IssueRedEdited,
        Self::IssueNoTests,
        Self::IssueSkipsTest,
        Self::IssueSuppressesLint,
        Self::IssueChecksUnderTest,
        Self::TeamPass,
        Self::Refused,
        Self::AskRelay,
        Self::Answered,
        Self::QuestionsTitle,
        Self::QuestionTitlePath,
        Self::QuestionTitleNoTests,
        Self::QuestionBody,
        Self::QuestionRecommend,
        Self::WhyOnlyHuman,
        Self::WhyPassesUsed,
        Self::NoticeNotAsked,
    ];

    /// Where the text is, who reads it and which variables it uses.
    #[must_use]
    pub const fn spec(self) -> PromptSpec {
        match self {
            Self::IssueTestDeleted => {
                PromptSpec::new("harness/checks/issue-test-deleted.md", Audience::Model, &["path"])
            }
            Self::IssueTestRenamed => {
                PromptSpec::new("harness/checks/issue-test-renamed.md", Audience::Model, &["path", "to"])
            }
            Self::IssueTestEdited => PromptSpec::new(
                "harness/checks/issue-test-edited.md",
                Audience::Model,
                &["path", "removed"],
            ),
            Self::IssueTestBinary => PromptSpec::new("harness/checks/issue-test-binary.md", Audience::Model, &["path"]),
            Self::IssueTestMode => PromptSpec::new("harness/checks/issue-test-mode.md", Audience::Model, &["path"]),
            Self::IssueProtected => PromptSpec::new(
                "harness/checks/issue-protected.md",
                Audience::Model,
                &["path", "change"],
            ),
            Self::IssueRedEdited => PromptSpec::new(
                "harness/checks/issue-red-edited.md",
                Audience::Model,
                &["path", "red", "edit"],
            ),
            Self::IssueNoTests => PromptSpec::new("harness/checks/issue-no-tests.md", Audience::Model, &[]),
            Self::IssueSkipsTest => PromptSpec::new(
                "harness/checks/issue-skips-test.md",
                Audience::Model,
                &["path", "line", "count"],
            ),
            Self::IssueSuppressesLint => PromptSpec::new(
                "harness/checks/issue-suppresses-lint.md",
                Audience::Model,
                &["path", "line", "count"],
            ),
            Self::IssueChecksUnderTest => PromptSpec::new(
                "harness/checks/issue-checks-under-test.md",
                Audience::Model,
                &["path", "line", "count"],
            ),
            Self::TeamPass => PromptSpec::new(
                "harness/checks/team-pass.md",
                Audience::Model,
                &["pr", "pass", "passes", "reasons", "refused"],
            ),
            Self::Refused => PromptSpec::new("harness/checks/refused.md", Audience::Model, &["subjects"]),
            Self::AskRelay => PromptSpec::new(
                "harness/checks/ask-relay.md",
                Audience::Model,
                &["questions_file", "number", "task"],
            ),
            Self::Answered => PromptSpec::new("harness/checks/answered.md", Audience::Model, &[]),
            Self::QuestionsTitle => {
                PromptSpec::new("harness/checks/questions-title.md", Audience::Human, &["team", "task"])
            }
            Self::QuestionTitlePath => {
                PromptSpec::new("harness/checks/question-title-path.md", Audience::Human, &["path"])
            }
            Self::QuestionTitleNoTests => {
                PromptSpec::new("harness/checks/question-title-no-tests.md", Audience::Human, &[])
            }
            Self::QuestionBody => PromptSpec::new(
                "harness/checks/question-body.md",
                Audience::Human,
                &["why", "issues", "allow", "refuse"],
            ),
            Self::QuestionRecommend => {
                PromptSpec::new("harness/checks/question-recommend.md", Audience::Human, &["refuse"])
            }
            Self::WhyOnlyHuman => PromptSpec::new("harness/checks/why-only-human.md", Audience::Human, &[]),
            Self::WhyPassesUsed => PromptSpec::new("harness/checks/why-passes-used.md", Audience::Human, &["passes"]),
            Self::NoticeNotAsked => {
                PromptSpec::new("harness/checks/notice-not-asked.md", Audience::Human, &["team", "task"])
            }
        }
    }
}
