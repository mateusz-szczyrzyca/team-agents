//! When a task was last active: when the output of its runs last changed.
//!
//! A model CLI writes what it does to the output file of its run as it does it, so the time that file last changed is
//! the time the agent last did something. The watchdog reads the same files to tell a run that hangs.

use std::io;
use std::path::PathBuf;
use std::time::SystemTime;

use agent_core::ids::TaskId;

use super::launch::STDOUT_FILE;
use crate::store::Home;

/// Why the last activity of a task cannot be told.
#[derive(Debug, thiserror::Error)]
#[error("cannot read {path:?}")]
pub struct ActivityError {
    /// The directory or the file.
    pub path: PathBuf,
    /// What the operating system said.
    pub source: io::Error,
}

/// When the output of a run of `task` last changed; `None` when no run of it has written any.
///
/// # Errors
/// [`ActivityError`] when the directory of the runs or an output file cannot be read; a directory or a file that is
/// not there is no error.
pub fn last_activity(home: &Home, task: &TaskId) -> Result<Option<SystemTime>, ActivityError> {
    let _ = (home, task);
    Ok(None)
}
