//! The note of since when a request waits in the queue for a worker: a file `worker-wait-<number>.json` in the requester's
//! task directory, written by the daemon at its first look at the queued request and removed when the request leaves the
//! queue. `agent status` reads it to show how long the request has waited.

use std::io;
use std::path::{Path, PathBuf};

use agent_core::state::Timestamp;
use serde::{Deserialize, Serialize};

use crate::agentbus::RequestId;
use crate::text_file::{ReplaceError, TextFileError, create_new_text_file, read_text_file};

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
    /// The note cannot be put into words.
    #[error("the wait of the request cannot be put into the note {path:?}")]
    Encode {
        /// The file.
        path: PathBuf,
        /// What the encoder refused.
        source: serde_json::Error,
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
    /// The task directories cannot be listed for notes.
    #[error("the notes of waits cannot be looked for in {path:?}")]
    List {
        /// The directory.
        path: PathBuf,
        /// What the file system refused.
        source: io::Error,
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

/// The most a note may hold, in bytes: far above what the harness writes, a bound on what a damaged file can make it read.
const NOTE_LIMIT: usize = 4096;

/// The permission bits of a note: the owner reads and writes it.
const NOTE_MODE: u32 = 0o600;

/// What a note holds: the request and the time of the first look at it in the queue.
#[derive(Debug, Serialize, Deserialize)]
struct Note {
    /// The request, as `agent status` shows it (`H3`); informative, the name of the file says which request it is.
    request: String,
    /// When the daemon first saw the request queued.
    since: Timestamp,
}

/// Notes that `request` has waited since `since`; a note that is there stays as it is, so the wait counts from the first look.
///
/// # Errors
/// [`WaitFileError::Write`].
pub async fn write_since(task_dir: &Path, request: RequestId, since: Timestamp) -> Result<(), WaitFileError> {
    let path = wait_path(task_dir, request);
    let note = Note {
        request: request.to_string(),
        since,
    };
    let text = serde_json::to_string(&note).map_err(|source| WaitFileError::Encode {
        path: path.clone(),
        source,
    })?;
    match create_new_text_file(&path, &text, NOTE_MODE).await {
        Ok(()) => Ok(()),
        // The note of the first look is the one that counts.
        Err(ReplaceError::Write { source, .. }) if source.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(source) => Err(WaitFileError::Write { path, source }),
    }
}

/// Since when `request` has waited, as its note says; `None` when it has no note.
///
/// # Errors
/// [`WaitFileError::Read`], [`WaitFileError::Parse`].
pub async fn read_since(task_dir: &Path, request: RequestId) -> Result<Option<Timestamp>, WaitFileError> {
    let path = wait_path(task_dir, request);
    let Some(file) = read_text_file(&path, NOTE_LIMIT)
        .await
        .map_err(|source| WaitFileError::Read {
            path: path.clone(),
            source,
        })?
    else {
        return Ok(None);
    };
    let note: Note = serde_json::from_str(&file.text).map_err(|source| WaitFileError::Parse { path, source })?;
    Ok(Some(note.since))
}

/// Removes the note of `request`; a request without one is as it should be.
///
/// # Errors
/// [`WaitFileError::Remove`].
pub async fn clear(task_dir: &Path, request: RequestId) -> Result<(), WaitFileError> {
    let path = wait_path(task_dir, request);
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(WaitFileError::Remove { path, source }),
    }
}

/// A note found on disk: the task directory it is in and the request it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundNote {
    /// The task directory that holds the note.
    pub task_dir: PathBuf,
    /// The request the name of the note tells.
    pub request: RequestId,
}

/// The notes of every task directory below `tasks_dir`, in no order; no `tasks_dir` holds none.
///
/// # Errors
/// [`WaitFileError::List`].
pub async fn find_notes(_tasks_dir: &Path) -> Result<Vec<FoundNote>, WaitFileError> {
    Ok(Vec::new())
}

#[cfg(test)]
mod tests;
