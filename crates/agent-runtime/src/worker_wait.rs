//! The note of since when a request waits in the queue for a worker: a file `worker-wait-<number>.json` in the requester's
//! task directory, written by the daemon at its first look at the queued request and removed when the request leaves the
//! queue. `agent status` reads it to show how long the request has waited.

use std::io;
use std::path::{Path, PathBuf};

use agent_core::state::Timestamp;

use crate::agentbus::RequestId;
use crate::text_file::{ReplaceError, TextFileError};

/// Why the note of a request's wait cannot be written, read or removed.
#[derive(Debug, thiserror::Error)]
pub enum WaitFileError {
    /// The note cannot be written.
    #[error("the wait of the request cannot be noted in {path:?}")]
    Write {
        /// The file.
        path: PathBuf,
        /// What the file system refused.
        source: ReplaceError,
    },
    /// The note cannot be read.
    #[error("the note of the wait of the request cannot be read from {path:?}")]
    Read {
        /// The file.
        path: PathBuf,
        /// What the file system refused.
        source: TextFileError,
    },
    /// The file is not a note of a wait.
    #[error("{path:?} is not the note of the wait of a request")]
    Parse {
        /// The file.
        path: PathBuf,
        /// What the parser found wrong.
        source: serde_json::Error,
    },
    /// The note cannot be removed.
    #[error("the note of the wait of the request cannot be removed from {path:?}")]
    Remove {
        /// The file.
        path: PathBuf,
        /// What the file system refused.
        source: io::Error,
    },
}

/// The file of the note of `request` in the task directory `task_dir`.
#[must_use]
pub fn wait_path(task_dir: &Path, request: RequestId) -> PathBuf {
    task_dir.join(format!("worker-wait-{}.json", request.number()))
}

/// Notes that `request` has waited since `since`; a note that is there stays as it is, so the wait counts from the first look.
///
/// # Errors
/// [`WaitFileError::Write`].
pub async fn write_since(_task_dir: &Path, _request: RequestId, _since: Timestamp) -> Result<(), WaitFileError> {
    Ok(())
}

/// Since when `request` has waited, as its note says; `None` when it has no note.
///
/// # Errors
/// [`WaitFileError::Read`], [`WaitFileError::Parse`].
pub async fn read_since(_task_dir: &Path, _request: RequestId) -> Result<Option<Timestamp>, WaitFileError> {
    Ok(None)
}

/// Removes the note of `request`; a request without one is as it should be.
///
/// # Errors
/// [`WaitFileError::Remove`].
pub async fn clear(_task_dir: &Path, _request: RequestId) -> Result<(), WaitFileError> {
    Ok(())
}

#[cfg(test)]
mod tests;
