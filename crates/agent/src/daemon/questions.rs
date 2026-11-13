//! The questions file of a task: read and checked before the human is asked, given its instructions, answered, and
//! turned into what the task is told when it goes on.
//!
//! The functions here are pure but for [`read`]: the owner reads and writes the files, these decide what is in them.

use std::cell::RefCell;
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
    let unknown: Vec<NonZeroU32> = answers
        .as_slice()
        .iter()
        .map(|answer| answer.question)
        .filter(|question| file.entry(*question).is_none())
        .collect();
    if !unknown.is_empty() {
        return Err(Refusal::UnknownQuestions { numbers: unknown });
    }
    let mut answered = file.clone();
    for answer in answers.as_slice() {
        // Every question was checked to be in the file above, so the file takes each verdict.
        answered
            .set_verdict(answer.question, Some(answer.verdict.clone()))
            .map_err(|missing| Refusal::UnknownQuestions {
                numbers: vec![missing.number],
            })?;
    }
    if let Err(incomplete) = answered.answers() {
        return Err(Refusal::Incomplete {
            problems: incomplete.problems,
        });
    }
    answered.mark_answered();
    Ok(answered)
}

impl Refusal {
    /// The refusal of the request, telling the human what to fix; `file` is the file the answers were for.
    pub(super) fn failure(&self, file: &QuestionsFile) -> Failure {
        let message = match self {
            Self::UnknownQuestions { numbers } => {
                let named: Vec<String> = numbers.iter().map(ToString::to_string).collect();
                format!("the questions file has no question {}", named.join(", "))
            }
            Self::Incomplete { problems } => format!("the answers are not complete:\n{}", describe(file, problems)),
        };
        Failure {
            kind: FailureKind::Invalid,
            message,
        }
    }
}

/// The questions of `problems`, one on each line: its number, its title and what is wrong.
pub(super) fn describe(file: &QuestionsFile, problems: &[(NonZeroU32, AnswerProblem)]) -> String {
    let lines: Vec<String> = problems
        .iter()
        .map(|(number, problem)| {
            let title = file.entry(*number).map_or("", |entry| entry.title());
            format!("- question {number} ({title}): {problem}")
        })
        .collect();
    lines.join("\n")
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
    let values = Variables::new().with("task_id", task.as_str())?;
    let mut prepared = file.clone();
    prepared.set_instructions(&texts.render(QuestionsPrompt::Instructions, &values)?);
    Ok(prepared)
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
    // `expand` takes words that cannot fail; a rendering that does is kept and reported after it.
    let failed = RefCell::new(None);
    let expanded = file.expand(|recommendation| {
        let rendered = Variables::new()
            .with("recommendation", recommendation)
            .map_err(TextError::from)
            .and_then(|values| {
                texts
                    .render(QuestionsPrompt::Accepted, &values)
                    .map_err(TextError::from)
            });
        rendered.unwrap_or_else(|error| {
            failed.borrow_mut().get_or_insert(error);
            String::new()
        })
    })?;
    if let Some(error) = failed.into_inner() {
        return Err(error);
    }
    let values = Variables::new()
        .with("round", round.to_string())?
        .with("answers", expanded.trim_end())?;
    Ok(texts.render(QuestionsPrompt::Answered, &values)?)
}

pub(super) mod watch;

#[cfg(test)]
mod tests;
