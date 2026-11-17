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
    let runs = home.runs_dir(task);
    let cannot_read = |path: &std::path::Path| {
        let path = path.to_path_buf();
        move |source| ActivityError { path, source }
    };
    let entries = match std::fs::read_dir(&runs) {
        Ok(entries) => entries,
        // A task that has not run has no directory of runs.
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(cannot_read(&runs)(source)),
    };
    let mut newest: Option<SystemTime> = None;
    for entry in entries {
        let output = entry.map_err(cannot_read(&runs))?.path().join(STDOUT_FILE);
        let changed = match std::fs::metadata(&output) {
            Ok(metadata) => metadata.modified().map_err(cannot_read(&output))?,
            // A run that has written nothing yet, or an entry that is no run.
            Err(source) if matches!(source.kind(), io::ErrorKind::NotFound | io::ErrorKind::NotADirectory) => {
                continue;
            }
            Err(source) => return Err(cannot_read(&output)(source)),
        };
        newest = Some(newest.map_or(changed, |known| known.max(changed)));
    }
    Ok(newest)
}
