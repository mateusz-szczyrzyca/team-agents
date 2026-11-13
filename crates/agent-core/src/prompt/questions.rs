//! The texts of the questions and their answers: the instructions in a questions file, the words for an accepted
//! recommendation, what a task is told when its questions were answered, and the notification of answers that are not
//! complete (`prompts/harness/questions/<name>.md`).
//!
//! They are a fourth catalog beside [`PromptId`](super::PromptId), [`HarnessPrompt`](super::HarnessPrompt) and
//! [`TeamPrompt`](super::TeamPrompt), with the same shape and its own directory, which the integration test
//! `tests/harness_questions_prompts.rs` checks against the entries.

use super::{Audience, PromptSpec};

/// Every text of the questions and their answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum QuestionsPrompt {
    /// The instructions the human reads under the title of a questions file.
    Instructions,
    /// The words that tell the agent the human accepted a recommendation.
    Accepted,
    /// What a task is told when its questions were answered.
    Answered,
    /// The notification that a questions file is marked answered but its answers are not complete.
    NoticeIncomplete,
}

impl QuestionsPrompt {
    /// Every text of the catalog.
    pub const ALL: &'static [Self] = &[
        Self::Instructions,
        Self::Accepted,
        Self::Answered,
        Self::NoticeIncomplete,
    ];

    /// Where the text is, who reads it and which variables it uses.
    #[must_use]
    pub const fn spec(self) -> PromptSpec {
        match self {
            Self::Instructions => PromptSpec::new("harness/questions/instructions.md", Audience::Human, &["task_id"]),
            Self::Accepted => PromptSpec::new("harness/questions/accepted.md", Audience::Model, &["recommendation"]),
            Self::Answered => PromptSpec::new("harness/questions/answered.md", Audience::Model, &["number", "answers"]),
            Self::NoticeIncomplete => PromptSpec::new(
                "harness/questions/notice-incomplete.md",
                Audience::Human,
                &["agent", "task", "file", "problems"],
            ),
        }
    }
}
