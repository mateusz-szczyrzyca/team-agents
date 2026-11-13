//! `agent gate`: run the repository's gate for a task, one gate at a time on the machine, and say what it said.

use std::io::{self, SeekFrom, Write};
use std::path::{Path, PathBuf};

use agent_core::ids::TaskId;
use agent_core::protocol::{Client, Command, GateRequest, Reply};
use agent_core::state::{GateOutcome, GateTree};
use agent_runtime::process::error_chain;
use agent_runtime::store::Home;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::status::kind_of;
use crate::client::{ClientError, Connection};

/// The client the command line is.
const CLIENT: Client = Client::Cli;

/// The most of the end of a gate's output that is read to find its last lines.
const TAIL_BYTES: u64 = 64 * 1024;

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
///
/// A worktree is `<home>/worktrees/<repository>/<task>`; the working directory of a process is a resolved path, and
/// `AGENTS_HOME` may name the home through a link, so the home's own resolved path is tried too.
pub(crate) async fn task_in(home: &Home, directory: &Path) -> Option<TaskId> {
    let worktrees = home.worktrees_dir();
    if let Some(task) = task_under(&worktrees, directory) {
        return Some(task);
    }
    match tokio::fs::canonicalize(&worktrees).await {
        Ok(resolved) => task_under(&resolved, directory),
        Err(error) => {
            // A home with no worktrees directory has no task in it: the answer is "none", which the command reports.
            tracing::debug!(%error, path = %worktrees.display(), "the worktrees of the home cannot be resolved");
            None
        }
    }
}

/// The task of `directory` when it is `<repository>/<task>[/...]` under `worktrees`.
fn task_under(worktrees: &Path, directory: &Path) -> Option<TaskId> {
    let Ok(inside) = directory.strip_prefix(worktrees) else {
        // Not under the worktrees: the directory is no task's.
        return None;
    };
    let mut names = inside.components();
    let _repository = names.next()?;
    let name = names.next()?.as_os_str().to_str()?;
    match name.parse() {
        Ok(task) => Some(task),
        // A directory not named like a task is no task's worktree.
        Err(not_a_task) => {
            tracing::debug!(%not_a_task, "a directory under the worktrees is not named like a task");
            None
        }
    }
}

/// Asks the daemon to run the gate of `task`, waits for it, and prints the end of its output and what it said.
///
/// # Errors
/// [`GateError`]; [`GateError::NotPassed`] when the gate ran and failed or timed out.
pub(crate) async fn gate(connection: &mut Connection, task: TaskId, out: &mut impl Write) -> Result<(), GateError> {
    say(
        out,
        &format!("Asking the daemon to run the gate of {task}; it runs after the gates asked before it."),
    )?;
    let reply = connection
        .request_until_answered(CLIENT, Command::Gate(GateRequest { task }))
        .await?;
    let Reply::GateRecorded { record } = reply else {
        return Err(GateError::UnexpectedReply { kind: kind_of(&reply) });
    };
    let log = record.result.log.as_path().to_path_buf();
    match tail(&log).await {
        Ok(lines) => {
            for line in lines {
                say(out, &line)?;
            }
        }
        // The verdict matters more than the end of the output: it is told without it.
        Err(error) => say(
            out,
            &format!(
                "gate: cannot read the output {}: {}",
                log.display(),
                error_chain(&error)
            ),
        )?,
    }
    let summary = record.summary();
    say(
        out,
        &format!("gate: {summary}; recorded. Full output: {}", log.display()),
    )?;
    if record.result.outcome == GateOutcome::Passed && record.tree == GateTree::NotClean {
        say(
            out,
            "gate: the tree was not clean, so this is not a pass of the commit: commit your work (or finish the merge) and run `agent gate` again.",
        )?;
    }
    match record.result.outcome {
        GateOutcome::Passed => Ok(()),
        GateOutcome::Failed { .. } | GateOutcome::TimedOut => Err(GateError::NotPassed { summary, log }),
    }
}

/// Writes `text` and a line break to `out` and flushes it, so that whoever reads the output sees it at once.
fn say(out: &mut impl Write, text: &str) -> Result<(), GateError> {
    writeln!(out, "{text}").map_err(GateError::Output)?;
    out.flush().map_err(GateError::Output)
}

/// The last [`TAIL_LINES`] lines of the file at `path`, reading no more than its last [`TAIL_BYTES`] bytes.
async fn tail(path: &Path) -> Result<Vec<String>, io::Error> {
    let mut file = tokio::fs::File::open(path).await?;
    let length = file.metadata().await?.len();
    let start = length.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).await?;
    let mut bytes = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut bytes).await?;
    // The output of a gate is a person's to read, not data: a byte that is not text is shown as the replacement
    // character, as a terminal would show it.
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        // The read began inside a line: its first part is not a whole line.
        lines.remove(0);
    }
    let keep = lines.len().saturating_sub(TAIL_LINES);
    Ok(lines.into_iter().skip(keep).map(str::to_owned).collect())
}

#[cfg(test)]
mod tests;
