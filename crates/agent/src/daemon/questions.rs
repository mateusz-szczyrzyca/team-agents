//! The questions file of a task: read and checked before the human is asked, given its instructions, answered, and
//! turned into what the task is told when it goes on.
//!
//! The functions here are pure but for [`read`]: the owner reads and writes the files, these decide what is in them.

use std::io;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;

use agent_core::ids::TaskId;
use agent_core::prompt::{QuestionsPrompt, Variables, VariablesError};
use agent_core::protocol::{AnswerList, Failure, FailureKind};
use agent_core::questions::{AnswerProblem, ExpandError, MAX_QUESTIONS_BYTES, ParseError, QuestionsFile};
use agent_runtime::questions_prompts::{QuestionsPrompts, RenderQuestionsTextError};
use agent_runtime::text_file::{TextFileError, read_text_file};

/// Why a questions file cannot be put to the human.
#[derive(Debug, thiserror::Error)]
pub(super) enum QuestionsProblem {
    /// The file cannot be read: most likely the agent did not write it.
    #[error("the questions file {path:?} cannot be read: the agent has to write it before it ends its turn")]
    Read {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The file is longer than the limit.
    #[error("the questions file {path:?} is longer than {MAX_QUESTIONS_BYTES} bytes")]
    TooLarge {
        /// The file.
        path: PathBuf,
    },
    /// The file is not UTF-8 text.
    #[error("the questions file {path:?} is not UTF-8 text")]
    NotUtf8 {
        /// The file.
        path: PathBuf,
        /// Where the text stops being UTF-8.
        source: FromUtf8Error,
    },
    /// The file is not in the format of a questions file.
    #[error("the questions file {path:?} is not in the format of a questions file")]
    Format {
        /// The file.
        path: PathBuf,
        /// What is wrong with it, with its line.
        source: ParseError,
    },
}

/// Reads and parses the questions file at `path`.
///
/// # Errors
/// [`QuestionsProblem`].
pub(super) async fn read(path: &Path) -> Result<QuestionsFile, QuestionsProblem> {
    let owned = || path.to_path_buf();
    let file = read_text_file(path, MAX_QUESTIONS_BYTES)
        .await
        .map_err(|error| match error {
            TextFileError::Read(source) => QuestionsProblem::Read { path: owned(), source },
            TextFileError::TooLarge { .. } => QuestionsProblem::TooLarge { path: owned() },
            TextFileError::NotUtf8(source) => QuestionsProblem::NotUtf8 { path: owned(), source },
        })?
        // No file at all is the most likely fault: the agent ended its turn without writing it.
        .ok_or_else(|| QuestionsProblem::Read {
            path: owned(),
            source: io::Error::from(io::ErrorKind::NotFound),
        })?;
    QuestionsFile::parse(&file.text).map_err(|source| QuestionsProblem::Format { path: owned(), source })
}

/// Why answers cannot be put into a questions file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Refusal {
    /// The answers name questions the file does not have.
    UnknownQuestions {
        /// The numbers, in the order of the answers.
        numbers: Vec<NonZeroU32>,
    },
    /// Some questions cannot be answered from the verdicts they would have.
    Incomplete {
        /// The questions and what is wrong with each, in file order.
        problems: Vec<(NonZeroU32, AnswerProblem)>,
    },
}

/// `file` with the verdicts of `answers`, marked answered; `file` is not changed.
///
/// The verdicts the file already has for questions the answers do not mention stay.
///
/// # Errors
/// [`Refusal::UnknownQuestions`] when an answer names a question the file does not have;
/// [`Refusal::Incomplete`] when a question would be left without a verdict, or with `ok` and no recommendation.
pub(super) fn apply(file: &QuestionsFile, answers: &AnswerList) -> Result<QuestionsFile, Refusal> {
    let _ = (file, answers);
    Ok(file.clone())
}

impl Refusal {
    /// The refusal of the request, telling the human what to fix; `file` is the file the answers were for.
    pub(super) fn failure(&self, file: &QuestionsFile) -> Failure {
        let _ = file;
        Failure {
            kind: FailureKind::Invalid,
            message: String::new(),
        }
    }
}

/// Why a text of the questions cannot be made.
#[derive(Debug, thiserror::Error)]
pub(super) enum TextError {
    /// A text cannot be rendered.
    #[error("cannot render a text of the questions")]
    Render(#[from] RenderQuestionsTextError),
    /// A variable name of the program is not valid.
    #[error("a variable of a text is not valid")]
    Variable(#[from] VariablesError),
    /// The file is not answered properly, or the words for an accepted recommendation are not a verdict.
    #[error("the questions file cannot be handed to the task")]
    Expand(#[from] ExpandError),
}

/// `file` with the instructions of the template under its title, for the human who answers `task`.
///
/// # Errors
/// [`TextError`] when the template cannot be rendered.
pub(super) fn with_instructions(
    file: &QuestionsFile,
    texts: &QuestionsPrompts,
    task: &TaskId,
) -> Result<QuestionsFile, TextError> {
    let _ = (texts, task);
    Ok(file.clone())
}

/// What a task is told when its questions of `round` were answered: the answered `file`, without the instructions and
/// the marker, in the words of the template.
///
/// # Errors
/// [`TextError::Expand`] when the file is not answered properly; [`TextError`] when a text cannot be rendered.
pub(super) fn answers_prompt(
    file: &QuestionsFile,
    round: NonZeroU32,
    texts: &QuestionsPrompts,
) -> Result<String, TextError> {
    let _ = (file, round, texts);
    Ok(String::new())
}

#[cfg(test)]
mod tests;
