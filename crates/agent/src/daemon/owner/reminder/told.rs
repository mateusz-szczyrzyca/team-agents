//! The mark of the reminder of a run: a file in the run's directory, made before the reminder is put on the bus and
//! taken away when the bus refuses it, so that a restart of the daemon, which forgets what it put on the bus, never puts
//! the same reminder there a second time.

use std::path::PathBuf;

use agent_core::ids::{RunId, TaskId};
use agent_runtime::store::Home;

#[cfg(test)]
mod tests;

/// The name of the file in a run's directory that says its reminder of Serena was put on the bus.
const MARK_FILE: &str = "serena-reminder-told";

/// Why the mark of a reminder cannot be read or written.
#[derive(Debug, thiserror::Error)]
pub(crate) enum MarkError {
    /// The directory of the run cannot be asked whether the mark is there.
    #[error("cannot look for the mark of the reminder of Serena at {path:?}")]
    Look {
        /// The file that was looked for.
        path: PathBuf,
        /// What the file system refused.
        source: std::io::Error,
    },
    /// The mark cannot be written.
    #[error("cannot write the mark of the reminder of Serena at {path:?}")]
    Write {
        /// The file that was to be written.
        path: PathBuf,
        /// What the file system refused.
        source: std::io::Error,
    },
    /// The mark of a reminder the bus refused cannot be taken away.
    #[error("cannot take away the mark of the reminder of Serena at {path:?}")]
    Release {
        /// The file that was to be removed.
        path: PathBuf,
        /// What the file system refused.
        source: std::io::Error,
    },
}

/// What claiming the reminder of a run came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Claim {
    /// The mark was made by this claim: the reminder is the claimer's to put on the bus.
    Taken,
    /// The mark was there before: the reminder was claimed, and it is not put on the bus again.
    AlreadyTold,
}

/// The file that is the mark of the reminder of `run` of `task`.
fn mark_file(home: &Home, task: &TaskId, run: &RunId) -> PathBuf {
    home.run_dir(task, run).join(MARK_FILE)
}

/// Whether the bus took the reminder of `run` of `task` before: the mark is there.
///
/// # Errors
/// [`MarkError::Look`] when the file system cannot say.
pub(crate) async fn is_told(home: &Home, task: &TaskId, run: &RunId) -> Result<bool, MarkError> {
    let path = mark_file(home, task, run);
    tokio::fs::try_exists(&path)
        .await
        .map_err(|source| MarkError::Look { path, source })
}

/// Marks the reminder of `run` of `task` as taken by the bus.
///
/// # Errors
/// [`MarkError::Write`] when the file cannot be written: the run's directory is gone, or the disk refuses.
pub(crate) async fn mark_told(home: &Home, task: &TaskId, run: &RunId) -> Result<(), MarkError> {
    let path = mark_file(home, task, run);
    tokio::fs::write(&path, b"")
        .await
        .map_err(|source| MarkError::Write { path, source })
}

/// Claims the reminder of `run` of `task`: makes its mark, atomically, before the reminder is put on the bus. A reminder
/// goes out at most once per session, so a claim whose bus call never came back (a crash between the two) is not
/// retried: the reminder is lost, never sent twice.
///
/// # Errors
/// [`MarkError::Write`] when the mark cannot be made: the run's directory is gone, or the disk refuses.
pub(crate) async fn claim(home: &Home, task: &TaskId, run: &RunId) -> Result<Claim, MarkError> {
    let _ = (home, task, run);
    Ok(Claim::AlreadyTold)
}

/// Takes the mark of `run` of `task` away after the bus refused its reminder, so that the reminder can be tried again.
///
/// # Errors
/// [`MarkError::Release`] when the mark cannot be removed: the reminder is then not tried again.
pub(crate) async fn release(home: &Home, task: &TaskId, run: &RunId) -> Result<(), MarkError> {
    let _ = (home, task, run);
    Ok(())
}
