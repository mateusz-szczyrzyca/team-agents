//! The watch on the questions files: tells the owner when the human saves one.
//!
//! One watch per task directory that holds an open question, not recursive, so the activity of the runs of other tasks
//! never wakes it. An editor saves by writing in place or by renaming a new file over the old one; both are seen. What
//! is told is only *that* `questions-<round>.md` of a task was written: the owner reads the file and decides.

use std::collections::HashSet;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use agent_core::ids::TaskId;
use tokio::sync::mpsc;

/// A questions file the human saved.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

/// The prefix and the suffix of the name of a questions file: `questions-<round>.md`.
const FILE_PREFIX: &str = "questions-";

/// See [`FILE_PREFIX`].
const FILE_SUFFIX: &str = ".md";

/// The saves not yet taken by the owner: a save of a file that is waiting is not queued again, so that a flood of saves of
/// one file neither grows the queue nor crowds out the save of another.
type Pending = Arc<Mutex<HashSet<Saved>>>;

/// The saves the watch reports, one for each file until the owner takes it.
#[derive(Debug)]
pub(in crate::daemon) struct Saves {
    /// The queue; it holds at most one entry for each questions file that was saved and not yet taken.
    queue: mpsc::UnboundedReceiver<Saved>,
    /// The entries of the queue, shared with the watch that fills it.
    pending: Pending,
}

impl Saves {
    /// The next save, `None` when the watch is gone. A save that comes after this returns is queued again, so the file
    /// is read as it is at least once after its last save.
    pub(in crate::daemon) async fn recv(&mut self) -> Option<Saved> {
        let saved = self.queue.recv().await?;
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&saved);
        Some(saved)
    }
}

/// The watch.
pub(in crate::daemon) struct Watcher {
    /// The system's watcher; it reports on a thread of its own, to the handler it was made with.
    inner: notify::RecommendedWatcher,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watcher").finish_non_exhaustive()
    }
}

impl Watcher {
    /// A watch on the directories it is given, and the saves it reports.
    ///
    /// # Errors
    /// [`WatchError::Start`].
    pub(in crate::daemon) fn new() -> Result<(Self, Saves), WatchError> {
        let (sender, queue) = mpsc::unbounded_channel();
        let pending = Pending::default();
        let shared = Arc::clone(&pending);
        let inner = notify::recommended_watcher(move |result: notify::Result<notify::Event>| match result {
            Ok(event) => report(&sender, &shared, &event),
            // The system lost track of something: a save may have been missed. The owner looks at the open questions
            // again when the daemon starts, and the human can always run `agent answer`.
            Err(error) => tracing::warn!(%error, "the watch on the questions files reported an error"),
        })
        .map_err(WatchError::Start)?;
        Ok((Self { inner }, Saves { queue, pending }))
    }

    /// Starts watching the directory `dir` of a task.
    ///
    /// # Errors
    /// [`WatchError::Watch`].
    pub(in crate::daemon) fn watch(&mut self, dir: &Path) -> Result<(), WatchError> {
        notify::Watcher::watch(&mut self.inner, dir, notify::RecursiveMode::NonRecursive).map_err(|source| {
            WatchError::Watch {
                path: dir.to_path_buf(),
                source,
            }
        })
    }

    /// Stops watching `dir`.
    ///
    /// # Errors
    /// [`WatchError::Unwatch`].
    pub(in crate::daemon) fn unwatch(&mut self, dir: &Path) -> Result<(), WatchError> {
        notify::Watcher::unwatch(&mut self.inner, dir).map_err(|source| WatchError::Unwatch {
            path: dir.to_path_buf(),
            source,
        })
    }
}

/// Queues each questions file `event` says was written, unless a save of it is queued already.
fn report(queue: &mpsc::UnboundedSender<Saved>, pending: &Pending, event: &notify::Event) {
    if !writes(event.kind) {
        return;
    }
    for path in &event.paths {
        let Some(file) = saved_file(path) else {
            continue;
        };
        let new = pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(file.clone());
        // The file is read as it is when the queued save is handled, so a repeat adds nothing.
        if !new {
            continue;
        }
        // The owner is gone when the daemon shuts down.
        if queue.send(file).is_err() {
            tracing::debug!("a save of a questions file came after the owner ended");
        }
    }
}

/// Whether an event of this kind put text in a file: a file made, written, or renamed into place.
const fn writes(kind: notify::EventKind) -> bool {
    use notify::EventKind::{Access, Any, Create, Modify, Other, Remove};
    use notify::event::ModifyKind;
    match kind {
        Create(_) | Any => true,
        Modify(modification) => matches!(
            modification,
            ModifyKind::Any | ModifyKind::Data(_) | ModifyKind::Name(_) | ModifyKind::Other
        ),
        Access(_) | Remove(_) | Other => false,
    }
}

/// The task and the round of the file at `path`, when it is `<task>/questions-<round>.md`.
fn saved_file(path: &Path) -> Option<Saved> {
    let name = path.file_name()?.to_str()?;
    // A name or a directory that is not a questions file of a task is not an error: the directory has other files, and
    // the watch tells only about these.
    let round = name
        .strip_prefix(FILE_PREFIX)?
        .strip_suffix(FILE_SUFFIX)?
        .parse::<NonZeroU32>()
        .ok()?;
    let task = path.parent()?.file_name()?.to_str()?.parse::<TaskId>().ok()?;
    Some(Saved { task, round })
}

#[cfg(test)]
mod tests;
