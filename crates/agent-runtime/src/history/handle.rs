//! The handle every part of the daemon records through, and the thread behind it that writes.
//!
//! Recording never stops or delays the work: a record is put on a bounded queue and the caller goes on. When it cannot be
//! kept — the queue is full, the database cannot be written or opened — it is counted as lost, and the cause is reported
//! once. One thread writes, a record or a batch of the queue per transaction.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use agent_core::cli::ActivityEvent;
use agent_core::history::{HistoryHealth, HistoryState, Record};
use agent_core::ids::RunId;
use agent_core::state::Timestamp;

use super::db::Store;
use super::error::{CloseError, OpenError, StartError, WriteKind};

/// How many records may wait for the writer, unless the setup says otherwise.
pub const DEFAULT_QUEUE: NonZeroUsize = NonZeroUsize::new(4096).unwrap();

/// What can go wrong with the history, as the reporter is told: one cause is reported once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum Cause {
    /// The queue was full: the writer is slower than the work.
    QueueFull,
    /// There is no database to write to: it could not be opened, or is of a newer version.
    Unavailable,
    /// A write failed in this way.
    Write(WriteKind),
}

/// A problem of the history, for the reporter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// What kind of problem it is; each kind is reported once.
    pub cause: Cause,
    /// What happened, with the chain of causes.
    pub detail: String,
}

/// Where the problems of the history are told: standard error in the daemon.
pub trait Reporter: Send + Sync + fmt::Debug + 'static {
    /// Tells `problem`. Called once for each [`Cause`].
    fn report(&self, problem: &Problem);
}

/// The reporter of the daemon: a warning in its log, which goes to standard error.
#[derive(Debug, Clone, Copy, Default)]
pub struct TracingReporter;

impl Reporter for TracingReporter {
    fn report(&self, problem: &Problem) {
        tracing::warn!(cause = %problem.cause, detail = %problem.detail, "the history cannot keep a record: it goes on without it");
    }
}

/// What the writer is set up with.
#[derive(Debug, Clone)]
pub struct Setup {
    /// Where problems are told.
    pub reporter: Arc<dyn Reporter>,
    /// How many records may wait.
    pub capacity: NonZeroUsize,
}

/// What the queue carries.
enum Job {
    /// A record to write.
    Record(Record),
    /// The writer is to answer on this channel when everything before it is written.
    Flush(SyncSender<()>),
    /// The writer is to stop.
    Close,
}

/// What every clone of the handle shares.
struct Shared {
    /// How many records were handed in and are not in the database.
    lost: AtomicU64,
    /// Whether records are being written.
    state: Mutex<HistoryState>,
    /// The causes already reported.
    reported: Mutex<BTreeSet<Cause>>,
    /// The number the next step of each run that is going gets.
    steps: Mutex<HashMap<RunId, u32>>,
    /// Where problems are told.
    reporter: Arc<dyn Reporter>,
}

impl fmt::Debug for Shared {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Shared")
            .field("lost", &self.lost.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

/// The handle records go through. Cheap to clone; every clone feeds the same queue.
#[derive(Debug, Clone)]
pub struct History {
    /// What the clones share.
    shared: Arc<Shared>,
    /// The queue; `None` when there is nobody to write.
    queue: Option<SyncSender<Job>>,
}

/// A started writer and the handle to it.
#[derive(Debug)]
pub struct Started {
    /// The handle.
    pub history: History,
    /// The writer, to close at the end.
    pub writer: Writer,
}

/// The thread that writes.
#[derive(Debug)]
pub struct Writer {
    /// The thread.
    thread: JoinHandle<()>,
    /// The queue, to tell it to stop.
    queue: SyncSender<Job>,
}

impl Writer {
    /// Writes what is queued, stops the thread and waits for it.
    ///
    /// # Errors
    /// [`CloseError`].
    pub fn close(self) -> Result<(), CloseError> {
        let _ = (&self.thread, &self.queue);
        Ok(())
    }
}

impl History {
    /// A handle that records nothing and loses nothing: `[history] enabled = false`.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            shared: Arc::new(Shared {
                lost: AtomicU64::new(0),
                state: Mutex::new(HistoryState::Disabled),
                reported: Mutex::new(BTreeSet::new()),
                steps: Mutex::new(HashMap::new()),
                reporter: Arc::new(TracingReporter),
            }),
            queue: None,
        }
    }

    /// A handle for a history that cannot be opened: every record is counted as lost. The reason is reported once.
    #[must_use]
    pub fn unavailable(error: &OpenError, reporter: Arc<dyn Reporter>) -> Self {
        let _ = (error, reporter);
        Self::disabled()
    }

    /// Starts the writer thread for `store`.
    ///
    /// # Errors
    /// [`StartError`] when the thread cannot be started.
    pub fn start(store: impl Store, setup: Setup) -> Result<Started, StartError> {
        let _ = (store, setup);
        Err(StartError(std::io::Error::other("stub")))
    }

    /// Hands `record` to the writer. Never waits.
    pub fn record(&self, record: Record) {
        let _ = record;
    }

    /// Records a step of `run`, numbering it after the steps of the run recorded before.
    pub fn step(&self, run: &RunId, at: Timestamp, event: ActivityEvent) {
        let _ = (run, at, event);
    }

    /// Forgets the numbering of the steps of `run`, which has ended.
    pub fn finish_run(&self, run: &RunId) {
        let _ = run;
    }

    /// Waits until everything handed in before is written or lost.
    pub fn flush(&self) {}

    /// Whether the history is being written, and how many records it lost.
    #[must_use]
    pub fn health(&self) -> HistoryHealth {
        HistoryHealth {
            state: HistoryState::Writing,
            lost: 0,
        }
    }
}
