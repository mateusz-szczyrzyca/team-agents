//! The watch on the questions files: tells the owner when the human saves one.
//!
//! One watch per task directory that holds an open question, not recursive, so the activity of the runs of other tasks
//! never wakes it. An editor saves by writing in place or by renaming a new file over the old one; both are seen. What
//! is told is only *that* `questions-<number>.md` of a task was written: the owner reads the file and decides. The queue
//! of saves is bounded; when it is full, or the system reports an error, the owner is told to read every open file.

use std::collections::VecDeque;
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
    /// The number of the questions file.
    pub(in crate::daemon) number: NonZeroU32,
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

/// The prefix and the suffix of the name of a questions file: `questions-<number>.md`.
const FILE_PREFIX: &str = "questions-";

/// See [`FILE_PREFIX`].
const FILE_SUFFIX: &str = ".md";

/// The most saves that wait for the owner. A save that finds the queue full is not lost: it sets the wish for a rescan,
/// and the owner reads every open questions file, so the queue never grows with the number of files.
const MAX_QUEUED: usize = 256;

/// What the watch tells the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon) enum Report {
    /// The human saved this questions file.
    Saved(Saved),
    /// Saves may have been missed: the owner reads every open questions file again.
    Rescan,
}

/// The saves and the wish for a rescan that the owner has not taken yet.
#[derive(Debug, Default)]
struct Queue {
    /// The saved files in the order they were saved, each once.
    saves: VecDeque<Saved>,
    /// Whether a rescan is wanted.
    rescan: bool,
}

impl Queue {
    /// Notes a save of `file`, unless a save of it waits already; when the queue is full, asks for a rescan instead.
    fn push(&mut self, file: Saved) {
        if self.saves.contains(&file) {
            return;
        }
        if self.saves.len() >= MAX_QUEUED {
            self.rescan = true;
            return;
        }
        self.saves.push_back(file);
    }

    /// Asks for a rescan: a save may have been missed.
    fn lost(&mut self) {
        self.rescan = true;
    }

    /// The next thing to tell: the saves first, in order, then the rescan.
    fn take(&mut self) -> Option<Report> {
        if let Some(saved) = self.saves.pop_front() {
            return Some(Report::Saved(saved));
        }
        std::mem::take(&mut self.rescan).then_some(Report::Rescan)
    }
}

/// The queue and the signal that it has something, shared between the watch and the owner.
#[derive(Debug, Clone)]
struct Shared {
    /// What was reported and not taken.
    queue: Arc<Mutex<Queue>>,
    /// Wakes the owner; a wake that is waiting already says all that another would.
    wake: mpsc::Sender<()>,
}

/// The side of the watch that fills the queue.
#[derive(Debug, Clone)]
struct Reporter {
    /// The queue and the wake.
    shared: Shared,
}

impl Reporter {
    /// Takes what the system said: queues each questions file written; an error of the system asks for a rescan.
    fn event(&self, result: notify::Result<notify::Event>) {
        match result {
            Ok(event) => self.written(&event),
            Err(error) => {
                // The system lost track of something: a save may have been missed, and the owner looks at every open file.
                tracing::warn!(%error, "the watch on the questions files reported an error; the open files are read again");
                self.shared.queue.lock().unwrap_or_else(PoisonError::into_inner).lost();
                self.wake();
            }
        }
    }

    /// Queues each questions file `event` says was written.
    fn written(&self, event: &notify::Event) {
        if !writes(event.kind) {
            return;
        }
        let mut queue = self.shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
        for path in &event.paths {
            if let Some(file) = saved_file(path) {
                queue.push(file);
            }
        }
        drop(queue);
        self.wake();
    }

    /// Wakes the owner.
    fn wake(&self) {
        match self.shared.wake.try_send(()) {
            // A wake is waiting already: the owner reads the whole queue when it comes.
            Ok(()) | Err(mpsc::error::TrySendError::Full(())) => {}
            // The owner is gone when the daemon shuts down.
            Err(mpsc::error::TrySendError::Closed(())) => {
                tracing::debug!("a questions file was saved after the owner ended");
            }
        }
    }
}

/// The reports of the watch, in the order the saves came.
#[derive(Debug)]
pub(in crate::daemon) struct Saves {
    /// What the watch reported and the owner has not taken.
    queue: Arc<Mutex<Queue>>,
    /// Tells that the queue may have something; ends when the watch is gone.
    wake: mpsc::Receiver<()>,
}

impl Saves {
    /// The next report, `None` when the watch is gone. A save that comes after this returns is reported again, so the
    /// file is read as it is at least once after its last save.
    pub(in crate::daemon) async fn recv(&mut self) -> Option<Report> {
        loop {
            if let Some(report) = self.take() {
                return Some(report);
            }
            if self.wake.recv().await.is_none() {
                return self.take();
            }
        }
    }

    /// The next report that waits.
    fn take(&self) -> Option<Report> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner).take()
    }
}

/// A queue and its two ends.
fn channel() -> (Reporter, Saves) {
    let queue = Arc::new(Mutex::new(Queue::default()));
    let (wake, woken) = mpsc::channel(1);
    let shared = Shared {
        queue: Arc::clone(&queue),
        wake,
    };
    (Reporter { shared }, Saves { queue, wake: woken })
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
        let (reporter, saves) = channel();
        let inner = notify::recommended_watcher(move |result| reporter.event(result)).map_err(WatchError::Start)?;
        Ok((Self { inner }, saves))
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

/// The task and the number of the file at `path`, when it is `<task>/questions-<number>.md`.
fn saved_file(path: &Path) -> Option<Saved> {
    let name = path.file_name()?.to_str()?;
    // A name or a directory that is not a questions file of a task is not an error: the directory has other files, and
    // the watch tells only about these.
    let number = name
        .strip_prefix(FILE_PREFIX)?
        .strip_suffix(FILE_SUFFIX)?
        .parse::<NonZeroU32>()
        .ok()?;
    let task = path.parent()?.file_name()?.to_str()?.parse::<TaskId>().ok()?;
    Some(Saved { task, number })
}

#[cfg(test)]
mod tests;
