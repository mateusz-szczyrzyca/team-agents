//! The questions file a run's `STATUS: QUESTIONS` points to: read and checked before the human is asked.

use std::io;
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;

use agent_core::questions::{MAX_QUESTIONS_BYTES, ParseError, QuestionsFile};
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
