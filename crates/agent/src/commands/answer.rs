//! `agent answer`: answers the questions of a task, in an editor or from a file.

use std::io::{self, Write};
use std::path::PathBuf;

use agent_core::config::CommandLine;
use agent_core::ids::TaskId;

use crate::client::{ClientError, Connection};

/// Where the answers come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// A questions file the human filled in, given with `--file`.
    File(PathBuf),
    /// The questions file of the task, which this editor is started on.
    Editor(CommandLine),
}

/// Why `agent answer` failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AnswerError {
    /// The daemon could not be asked, or refused.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// The output cannot be written.
    #[error("cannot write the result")]
    Output(#[source] io::Error),
}

/// Answers the questions of `task` from `source` and says what happened.
///
/// # Errors
/// [`AnswerError`].
pub(crate) async fn answer(
    connection: &mut Connection,
    task: TaskId,
    source: Source,
    out: &mut impl Write,
) -> Result<(), AnswerError> {
    let _ = (connection, task, source, out);
    Ok(())
}
