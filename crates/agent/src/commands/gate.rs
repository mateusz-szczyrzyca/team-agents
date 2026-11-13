//! `agent gate`: run the repository's gate for a task, one gate at a time on the machine, and say what it said.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use agent_core::ids::TaskId;
use agent_runtime::store::Home;

use crate::client::{ClientError, Connection};

/// How many lines of the gate's output are printed after it ran: the end, where a failure is.
pub(crate) const TAIL_LINES: usize = 60;

/// Why `agent gate` failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GateError {
    /// The daemon could not be asked, or refused.
    #[error(transparent)]
    Client(#[from] ClientError),
    /// The command was run outside a task's worktree and no task was named.
    #[error("not in a task's worktree ({directory:?}): name the task, `agent gate task-...`")]
    NoTask {
        /// The directory the command ran in.
        directory: PathBuf,
    },
    /// The daemon answered with something else than the result of the gate.
    #[error("the daemon answered with {kind}, not with the result of the gate")]
    UnexpectedReply {
        /// What it answered.
        kind: &'static str,
    },
    /// The gate ran and did not pass.
    #[error("the gate did not pass: {summary}; its output is in {log:?}")]
    NotPassed {
        /// What the gate said, in a few words.
        summary: String,
        /// The file with its output.
        log: PathBuf,
    },
    /// The result cannot be written.
    #[error("cannot write the result of the gate")]
    Output(#[source] io::Error),
}

/// The task whose worktree `directory` is in or is, if it is one of `home`.
pub(crate) async fn task_in(home: &Home, directory: &Path) -> Option<TaskId> {
    let _ = (home, directory);
    None
}

/// Asks the daemon to run the gate of `task`, waits for it, and prints the end of its output and what it said.
///
/// # Errors
/// [`GateError`]; [`GateError::NotPassed`] when the gate ran and failed or timed out.
pub(crate) async fn gate(connection: &mut Connection, task: TaskId, out: &mut impl Write) -> Result<(), GateError> {
    let _ = (connection, task, out);
    Ok(())
}

#[cfg(test)]
mod tests;
