//! The questions file a run's `STATUS: QUESTIONS` points to: read and checked before the human is asked.

use std::io;
use std::path::{Path, PathBuf};
use std::string::FromUtf8Error;

use agent_core::questions::{MAX_QUESTIONS_BYTES, ParseError, QuestionsFile};
use tokio::io::AsyncReadExt;

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
    let read_error = |source| QuestionsProblem::Read {
        path: path.to_path_buf(),
        source,
    };
    let file = tokio::fs::File::open(path).await.map_err(read_error)?;
    let limit = u64::try_from(MAX_QUESTIONS_BYTES).map_or(u64::MAX, |limit| limit.saturating_add(1));
    let mut bytes = Vec::new();
    file.take(limit).read_to_end(&mut bytes).await.map_err(read_error)?;
    if bytes.len() > MAX_QUESTIONS_BYTES {
        return Err(QuestionsProblem::TooLarge {
            path: path.to_path_buf(),
        });
    }
    let text = String::from_utf8(bytes).map_err(|source| QuestionsProblem::NotUtf8 {
        path: path.to_path_buf(),
        source,
    })?;
    QuestionsFile::parse(&text).map_err(|source| QuestionsProblem::Format {
        path: path.to_path_buf(),
        source,
    })
}
