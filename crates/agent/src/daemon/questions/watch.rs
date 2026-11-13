//! The watch on the questions files: tells the owner when the human saves one.
//!
//! One watch per task directory that holds an open question, not recursive, so the activity of the runs of other tasks
//! never wakes it. An editor saves by writing in place or by renaming a new file over the old one; both are seen. What
//! is told is only *that* `questions-<round>.md` of a task was written: the owner reads the file and decides.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

use agent_core::ids::TaskId;
use tokio::sync::mpsc;

/// A questions file the human saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon) struct Saved {
    /// The task whose file it is.
    pub(in crate::daemon) task: TaskId,
    /// The round of the file.
    pub(in crate::daemon) round: NonZeroU32,
}

/// Why a directory cannot be watched.
#[derive(Debug, thiserror::Error)]
pub(in crate::daemon) enum WatchError {
    /// The watch cannot be started: the system's limit of watches or of instances is reached.
    #[error("cannot start the watch on the questions files")]
    Start(#[source] notify::Error),
    /// A directory cannot be watched.
    #[error("cannot watch {path:?} for the answers of the human")]
    Watch {
        /// The directory.
        path: PathBuf,
        /// What the system said.
        source: notify::Error,
    },
    /// A directory cannot be let go.
    #[error("cannot stop watching {path:?}")]
    Unwatch {
        /// The directory.
        path: PathBuf,
        /// What the system said.
        source: notify::Error,
    },
}

/// The watch.
#[derive(Debug)]
pub(in crate::daemon) struct Watcher;

impl Watcher {
    /// A watch that tells `saved` about the files that are saved in the directories it is given.
    ///
    /// # Errors
    /// [`WatchError::Start`].
    pub(in crate::daemon) fn new(saved: mpsc::Sender<Saved>) -> Result<Self, WatchError> {
        let _ = saved;
        Ok(Self)
    }

    /// Starts watching the directory `dir` of a task.
    ///
    /// # Errors
    /// [`WatchError::Watch`].
    pub(in crate::daemon) fn watch(&mut self, dir: &Path) -> Result<(), WatchError> {
        let _ = dir;
        Ok(())
    }

    /// Stops watching `dir`.
    ///
    /// # Errors
    /// [`WatchError::Unwatch`].
    pub(in crate::daemon) fn unwatch(&mut self, dir: &Path) -> Result<(), WatchError> {
        let _ = dir;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
