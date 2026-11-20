//! The handle every part of the daemon records through, and the thread behind it that writes.
//!
//! Recording never stops or delays the work: a record is put on a bounded queue and the caller goes on. When it cannot be
//! kept — the queue is full, the database cannot be written or opened — it is counted as lost, and the cause is reported
//! once. One thread writes, a record or a batch of the queue per transaction.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::future::Future;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use agent_core::cli::{ActivityEvent, SessionId};
use agent_core::history::{
    EventRecord, HistoryEvent, HistoryHealth, HistoryState, LostCause, LostKind, Record, RecordKind, RunSession, Step,
};
use agent_core::ids::{Clock, RoundId, RunId, TaskId, TeamId};
use agent_core::log::LogRecord;
use agent_core::state::Timestamp;

use super::db::Store;
use super::error::{CloseError, StartError, WriteError, WriteKind};
use super::redact::Redactor;
use crate::process::error_chain;

/// How many records may wait for the writer, unless the setup says otherwise.
pub const DEFAULT_QUEUE: NonZeroUsize = NonZeroUsize::new(4096).unwrap();

/// The most records one transaction holds: a burst is written in a few transactions, not in one that holds the lock of
/// the database for as long as the burst is.
const MAX_BATCH: usize = 256;

/// What can go wrong with the history, as the reporter is told: one cause is reported once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::Display)]
pub enum Cause {
    /// Retention failed; work continues and cleanup can be retried.
    #[strum(serialize = "retention")]
    Retention,

    /// The queue was full: the writer is slower than the work.
    #[strum(serialize = "queue_full")]
    QueueFull,
    /// There is no database to write to: it could not be opened, is of a newer version, or the writer has stopped.
    #[strum(serialize = "unavailable")]
    Unavailable,
    /// A write failed in this way.
    #[strum(to_string = "write_{0}")]
    Write(WriteKind),
    /// A record or its launch head could not be captured.
    #[strum(to_string = "capture_{0}")]
    Capture(CaptureKind),
    /// An event could not be built from the daemon's state.
    #[strum(serialize = "capture_event")]
    CaptureEvent,
    /// A known credential occurs in an identity that cannot safely be replaced.
    #[strum(serialize = "secret_identity")]
    SecretIdentity,
}

/// The snapshot that failed before a record could reach the writer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::Display)]
pub enum CaptureKind {
    /// The launch record lacks its starting commit.
    #[strum(serialize = "head")]
    Head,
    /// The changes record could not be built.
    #[strum(serialize = "changes")]
    Changes,
    /// The gate record could not be built after the command ran.
    #[strum(serialize = "gate")]
    Gate,
    /// A questions file could not be read.
    #[strum(serialize = "questions")]
    Questions,
    /// The clock could not give a valid timestamp for an event.
    #[strum(serialize = "clock")]
    Clock,
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
        tracing::warn!(
            cause = %problem.cause,
            detail = %problem.detail,
            "the history cannot keep some records: the work goes on, the records are counted as lost (agent status)"
        );
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
    /// Scheduled cleanup, ordered after records already handed in.
    Retain(super::Retention),

    /// A record to write.
    Record(Record),
    /// A daemon log entry to write.
    Log(LogRecord),
    /// Reads of a task's plan to write.
    Plans(Vec<super::plans::PlanRecord>),
    /// The writer is to answer on this channel when everything before it is written.
    Flush(SyncSender<()>),
    /// The writer is to stop.
    Close,
}

/// What every clone of the handle shares.
struct Shared {
    /// How many records were handed in and are not in the database.
    lost: AtomicU64,
    /// Credentials removed on the writer thread before persistence.
    redactor: Mutex<Redactor>,
    /// Optional sink for fallback lines on `stderr` in tests.
    stderr_sink: Mutex<Option<super::log::StderrSink>>,
    /// Whether records are being written.
    state: Mutex<HistoryState>,
    /// The causes already reported.
    reported: Mutex<BTreeSet<Cause>>,
    /// The daemon-log causes already reported directly to `stderr`.
    reported_log: Mutex<BTreeSet<Cause>>,
    /// The number the next step of each run that is going gets, less one: how many steps it has had.
    steps: Mutex<HashMap<RunId, u32>>,
    /// Where problems are told.
    reporter: Arc<dyn Reporter>,
    /// Latest observed session and sequence number of each active run.
    sessions: Mutex<HashMap<RunId, (SessionId, u32)>>,
    /// Active runs' team, task, rounds, and start time for run-scoped capture loss notes.
    runs: Mutex<HashMap<RunId, ActiveRun>>,
}

/// The team, task, rounds, and start time of an active run.
type ActiveRun = (TeamId, TaskId, Vec<RoundId>, Timestamp);

/// The data behind `mutex`. A thread that panicked while it held the lock did at most one insert, one removal or one
/// copy, so the data is consistent whatever happened: the poison is not an error here.
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Shared {
    /// A shared state in `state`, with `reporter`.
    fn new(state: HistoryState, reporter: Arc<dyn Reporter>, redactor: Redactor) -> Arc<Self> {
        Arc::new(Self {
            lost: AtomicU64::new(0),
            redactor: Mutex::new(redactor),
            stderr_sink: Mutex::new(None),
            state: Mutex::new(state),
            reported: Mutex::new(BTreeSet::new()),
            reported_log: Mutex::new(BTreeSet::new()),
            steps: Mutex::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            runs: Mutex::new(HashMap::new()),
            reporter,
        })
    }

    /// Counts `records` as lost.
    fn lose(&self, records: u64) {
        self.lost.fetch_add(records, Ordering::Relaxed);
    }

    /// Reports `cause` with `detail`, unless it was reported before.
    fn report(&self, cause: Cause, detail: impl FnOnce() -> String) {
        let first = locked(&self.reported).insert(cause);
        if first {
            let detail = locked(&self.redactor).text(&detail());
            self.reporter.report(&Problem { cause, detail });
        }
    }

    /// Writes `line` to the configured fallback sink or directly to `stderr`.
    fn write_stderr(&self, line: &str) {
        let sink = locked(&self.stderr_sink).clone();
        if let Some(sink) = sink {
            sink(line);
        } else if std::io::Write::write_all(&mut std::io::stderr(), format!("{line}\n").as_bytes()).is_err() {
            // Best-effort fallback to stderr when history.db cannot keep a log entry.
        }
    }

    /// Reports a dropped or failed daemon-log entry directly to `stderr` once per [`Cause`], without re-entering
    /// `tracing` or incrementing [`Self::lost`].
    fn report_log(&self, cause: Cause, detail: impl FnOnce() -> String) {
        let first = locked(&self.reported_log).insert(cause);
        if first {
            let detail = locked(&self.redactor).text(&detail());
            self.write_stderr(&format!("the history cannot keep a log entry: {detail}"));
        }
    }

    /// Counts a write that failed and reports why, once for each kind of failure.
    fn lose_to(&self, records: u64, error: &WriteError) {
        self.lose(records);
        self.report(Cause::Write(error.kind()), || error_chain(error));
    }
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
    /// Writes what is queued, stops the thread and waits for it. Call it when everything that records has ended: a record
    /// handed in afterwards is counted as lost.
    ///
    /// # Errors
    /// [`CloseError::Panicked`] when the thread panicked, [`CloseError::Gone`] when it had stopped already.
    pub fn close(self) -> Result<(), CloseError> {
        let told = self.queue.send(Job::Close);
        // A thread that panicked carries a payload that is not an error: there is nothing to keep of it.
        if self.thread.join().is_err() {
            return Err(CloseError::Panicked);
        }
        // The queue is closed when the thread stopped before it was told to: the send gave the job back.
        if told.is_err() {
            return Err(CloseError::Gone);
        }
        Ok(())
    }
}

impl History {
    /// A handle that records nothing and loses nothing: `[history] enabled = false`.
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            shared: Shared::new(HistoryState::Disabled, Arc::new(TracingReporter), Redactor::default()),
            queue: None,
        }
    }

    /// A handle for a history whose database or writer cannot start: every record is counted as lost. The reason is reported once.
    #[must_use]
    pub fn unavailable(error: &impl std::error::Error, reporter: Arc<dyn Reporter>) -> Self {
        let reason = error_chain(error);
        let shared = Shared::new(
            HistoryState::Unavailable { reason: reason.clone() },
            reporter,
            Redactor::default(),
        );
        shared.report(Cause::Unavailable, || reason);
        Self { shared, queue: None }
    }

    /// Starts the writer thread for `store`.
    ///
    /// # Errors
    /// [`StartError`] when the thread cannot be started.
    pub fn start(store: impl Store, setup: Setup) -> Result<Started, StartError> {
        Self::start_with_secrets(store, setup, agent_core::config::Secrets::default())
    }

    /// Starts a writer that redacts known secret values before persistence.
    ///
    /// # Errors
    /// [`StartError`] when the writer thread cannot start.
    pub fn start_with_secrets(
        store: impl Store,
        setup: Setup,
        secrets: agent_core::config::Secrets,
    ) -> Result<Started, StartError> {
        let shared = Shared::new(HistoryState::Writing, setup.reporter, Redactor::new(secrets));
        let (queue, jobs) = sync_channel(setup.capacity.get());
        let writing = Arc::clone(&shared);
        let dispatch = tracing::dispatcher::get_default(Clone::clone);
        let thread = std::thread::Builder::new()
            .name("history-writer".to_owned())
            .spawn(move || tracing::dispatcher::with_default(&dispatch, || write_jobs(store, &jobs, &writing)))
            .map_err(StartError)?;
        Ok(Started {
            history: Self {
                shared,
                queue: Some(queue.clone()),
            },
            writer: Writer { thread, queue },
        })
    }

    /// Configures the redactor and optional `stderr` sink used when daemon log entries fall back to `stderr`.
    pub(super) fn configure_log_fallback(&self, redactor: Redactor, sink: Option<super::log::StderrSink>) {
        *locked(&self.shared.redactor) = redactor;
        *locked(&self.shared.stderr_sink) = sink;
    }

    /// Hands `record` to the writer. Never waits.
    pub fn record(&self, record: Record) {
        let Some(queue) = &self.queue else {
            self.lose_without_writer();
            return;
        };
        if let Record::Run(run) = &record {
            locked(&self.shared.runs).insert(
                run.run.clone(),
                (run.team.clone(), run.task.clone(), run.rounds.clone(), run.at),
            );
        }
        match queue.try_send(Job::Record(record)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.shared.lose(1);
                self.shared.report(Cause::QueueFull, || {
                    "the writer of the history is slower than the work: records are dropped while its queue is full"
                        .to_owned()
                });
            }
            Err(TrySendError::Disconnected(_)) => {
                self.shared.lose(1);
                self.stopped();
            }
        }
    }

    /// Hands a daemon log `record` to the writer without counting drops in [`HistoryHealth::lost`].
    pub fn record_log(&self, record: LogRecord) {
        if self.is_disabled() {
            return;
        }
        let Some(queue) = &self.queue else {
            self.shared
                .report_log(Cause::Unavailable, || "the history database is unavailable".to_owned());
            return;
        };
        match queue.try_send(Job::Log(record)) {
            Err(TrySendError::Full(Job::Log(_))) => {
                self.shared.report_log(Cause::QueueFull, || {
                    "the writer of the history is slower than the work: log entries are dropped while its queue is full"
                        .to_owned()
                });
            }
            Err(TrySendError::Disconnected(Job::Log(_))) => {
                self.shared.report_log(Cause::Unavailable, || {
                    "the writer of the history has stopped".to_owned()
                });
            }
            Ok(()) | Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {}
        }
    }

    /// Capture one record only while a writer is available. An unavailable history counts it as lost without polling
    /// the capture future; a disabled history does neither. Capture failures do not fail the model's work.
    pub async fn record_captured<E: std::error::Error>(
        &self,
        run: &RunId,
        kind: CaptureKind,
        clock: &impl Clock,
        capture: impl Future<Output = Result<Record, E>>,
    ) {
        if !self.is_writing() {
            self.lose_without_writer();
            return;
        }
        match capture.await {
            Ok(record) => self.record(record),
            Err(error) => self.capture_run_failed(run, kind, clock, &error),
        }
    }

    /// Counts a failed run snapshot, reports its cause once, and queues a round-linked loss note when the history is writing.
    pub fn capture_run_failed(
        &self,
        run: &RunId,
        kind: CaptureKind,
        clock: &impl Clock,
        error: &impl std::error::Error,
    ) {
        self.capture_failed(kind, error);
        let Some((team, task, rounds, start_at)) = locked(&self.shared.runs)
            .get(run)
            .cloned()
            .filter(|_| self.is_writing())
        else {
            return;
        };
        let (what, cause) = match kind {
            CaptureKind::Head => (LostKind::Record(RecordKind::Launch), LostCause::CaptureHead),
            CaptureKind::Changes | CaptureKind::Gate | CaptureKind::Questions | CaptureKind::Clock => {
                (LostKind::Record(RecordKind::Changes), LostCause::CaptureChanges)
            }
        };
        let at = Timestamp::now(clock).unwrap_or(start_at);
        self.record(Record::Event(Box::new(EventRecord {
            at,
            team,
            rounds,
            event: HistoryEvent::Lost {
                task: Some(task),
                what,
                cause,
                detail: error_chain(error),
            },
        })));
    }

    /// Count a failed snapshot (including an incomplete launch) and report its cause once.
    pub fn capture_failed(&self, kind: CaptureKind, error: &impl std::error::Error) {
        if !self.is_disabled() {
            self.shared.lose(1);
            self.shared.report(Cause::Capture(kind), || error_chain(error));
        }
    }

    /// Counts an event that could not be built from the daemon's state and reports its cause once.
    pub fn capture_event_failed(&self, error: &impl std::error::Error) {
        if !self.is_disabled() {
            self.shared.lose(1);
            self.shared.report(Cause::CaptureEvent, || error_chain(error));
        }
    }

    /// Counts one record that could not be built before reaching the queue, unless the history is disabled.
    pub fn record_lost(&self) {
        if !self.is_disabled() {
            self.shared.lose(1);
        }
    }

    /// Whether snapshot work has a writer to receive it. An unavailable history must not run Git just to discard it.
    #[must_use]
    pub fn is_writing(&self) -> bool {
        matches!(*locked(&self.shared.state), HistoryState::Writing)
    }

    /// Counts a record handed to a history that has no writer: lost when the history is broken, nothing when it is off.
    fn lose_without_writer(&self) {
        if matches!(*locked(&self.shared.state), HistoryState::Unavailable { .. }) {
            self.shared.lose(1);
        }
    }

    /// Notes that the writer is gone: a record that reaches it is lost, and the history says so.
    fn stopped(&self) {
        let reason = "the writer of the history has stopped".to_owned();
        *locked(&self.shared.state) = HistoryState::Unavailable { reason: reason.clone() };
        self.shared.report(Cause::Unavailable, || reason);
    }

    /// Records a step of `run`, numbering it after the steps of the run recorded before.
    pub fn step(&self, run: &RunId, at: Timestamp, event: ActivityEvent) {
        if self.queue.is_none() {
            self.lose_without_writer();
            return;
        }
        let numbered = {
            let mut steps = locked(&self.shared.steps);
            let had = steps.entry(run.clone()).or_insert(0);
            had.checked_add(1).inspect(|next| *had = *next)
        };
        let Some(number) = numbered.and_then(NonZeroU32::new) else {
            // Four billion steps of one run: the number cannot be kept, so the step is not either.
            self.shared.lose(1);
            return;
        };
        self.record(Record::Step(Box::new(Step {
            run: run.clone(),
            number,
            at,
            event,
        })));
    }

    /// Records a changed session immediately, even when its notification has no activity event.
    pub(crate) fn session(&self, run: &RunId, session: &SessionId, clock: &impl Clock) {
        if self.is_disabled() {
            return;
        }
        let number = {
            let mut sessions = locked(&self.shared.sessions);
            let previous = sessions.get(run);
            if previous.is_some_and(|(known, _)| known == session) {
                return;
            }
            let next = previous.map_or(Some(1), |(_, n)| n.checked_add(1));
            // Remember an observation even if its record is lost; duplicate notifications are not new records.
            sessions.insert(run.clone(), (session.clone(), next.unwrap_or(u32::MAX)));
            next.and_then(NonZeroU32::new)
        };
        let Some(number) = number else {
            self.shared.lose(1);
            self.shared.report(Cause::Write(WriteKind::Encoding), || {
                "the run has exhausted its session sequence numbers".to_owned()
            });
            return;
        };
        let at = match Timestamp::now(clock) {
            Ok(at) => at,
            Err(error) => {
                // History failure must not fail the run; retain the loss and report its cause.
                self.shared.lose(1);
                self.shared
                    .report(Cause::Write(WriteKind::Encoding), || error_chain(&error));
                return;
            }
        };
        let stored_session = match session.as_str().parse() {
            Ok(session) => session,
            Err(error) => {
                self.shared.lose(1);
                self.shared
                    .report(Cause::Write(WriteKind::Encoding), || error_chain(&error));
                return;
            }
        };
        self.record(Record::Session(Box::new(RunSession {
            run: run.clone(),
            number,
            at,
            session: stored_session,
        })));
    }

    /// Forgets the numbering of the steps and sessions of `run`, which has ended.
    pub fn finish_run(&self, run: &RunId) {
        locked(&self.shared.steps).remove(run);
        locked(&self.shared.sessions).remove(run);
        locked(&self.shared.runs).remove(run);
    }

    /// Queues retention on the sole database writer without blocking work.
    pub fn retain(&self, request: super::Retention) {
        let Some(queue) = &self.queue else {
            return;
        };
        match queue.try_send(Job::Retain(request)) {
            Ok(()) => (),
            Err(TrySendError::Full(_)) => self.shared.report(Cause::Retention, || {
                "retention deferred: history queue is full".to_owned()
            }),
            Err(TrySendError::Disconnected(_)) => self.shared.report(Cause::Retention, || {
                "retention deferred: history writer has stopped".to_owned()
            }),
        }
    }

    /// Hands the plan `records` to the writer unless they are the task's last ones; never waits.
    pub fn write_plans(&self, records: Vec<super::plans::PlanRecord>) {
        if records.is_empty() {
            return;
        }
        let Some(queue) = &self.queue else {
            self.shared.lose(count(records.len()));
            return;
        };
        match queue.try_send(Job::Plans(records)) {
            Ok(()) => {}
            Err(TrySendError::Full(Job::Plans(records))) => {
                self.shared.lose(count(records.len()));
                self.shared.report(Cause::QueueFull, || {
                    "the writer of the history is slower than the work: plan versions are dropped while its queue is full"
                        .to_owned()
                });
            }
            Err(TrySendError::Disconnected(Job::Plans(records))) => {
                self.shared.lose(count(records.len()));
                self.stopped();
            }
            // Only `Job::Plans` is sent here, so every other job comes back as the one it was.
            Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => {
                let lost = match job {
                    Job::Plans(plans) => count(plans.len()),
                    Job::Record(_) | Job::Flush(_) | Job::Retain(_) | Job::Log(_) | Job::Close => 1,
                };
                self.shared.lose(lost);
            }
        }
    }

    /// Waits until everything handed in before is written or lost.
    pub fn flush(&self) {
        let Some(queue) = &self.queue else {
            return;
        };
        let (answer, answered) = sync_channel(1);
        if queue.send(Job::Flush(answer)).is_err() {
            // The writer is gone: nothing is left to wait for.
            return;
        }
        // The writer answers when it has written what came before; if it stops first the channel closes and the wait ends.
        if answered.recv().is_err() {
            tracing::debug!("the history writer stopped before it answered a flush");
        }
    }

    /// Whether the history is off: nothing is recorded, so nothing is worth working out for it.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        matches!(*locked(&self.shared.state), HistoryState::Disabled)
    }

    /// Whether the history is being written, and how many records it lost.
    #[must_use]
    pub fn health(&self) -> HistoryHealth {
        HistoryHealth {
            state: locked(&self.shared.state).clone(),
            lost: self.shared.lost.load(Ordering::Relaxed),
        }
    }
}

/// The body of the writer thread: takes what is queued, a batch at a time, and writes it until it is told to stop or every
/// handle is gone.
fn write_jobs(mut store: impl Store, jobs: &Receiver<Job>, shared: &Shared) {
    let mut closing = false;
    while !closing {
        let Ok(first) = jobs.recv() else {
            return;
        };
        let mut batch: Vec<Record> = Vec::new();
        let mut log_batch: Vec<LogRecord> = Vec::new();
        let mut flushes: Vec<SyncSender<()>> = Vec::new();
        let mut next = Some(first);
        while let Some(job) = next.take() {
            match job {
                Job::Record(mut record) => {
                    if locked(&shared.redactor).record(&mut record) {
                        batch.push(record);
                    } else {
                        shared.lose(1);
                        shared.report(Cause::SecretIdentity, || {
                            "a record contains a credential in a structured identity".to_owned()
                        });
                    }
                }
                Job::Log(mut record) => {
                    if locked(&shared.redactor).log_record(&mut record) {
                        log_batch.push(record);
                    } else {
                        shared.report_log(Cause::SecretIdentity, || {
                            "a daemon log entry contains a credential in a structured identity".to_owned()
                        });
                    }
                }
                Job::Plans(plans) => {
                    // A plan is kept for a task whose run the history has: the records handed in before it go first.
                    write_batch(&mut store, shared, std::mem::take(&mut batch));
                    write_plans(&mut store, shared, &plans);
                    continue;
                }
                Job::Retain(request) => {
                    write_batch(&mut store, shared, std::mem::take(&mut batch));
                    write_log_batch(&mut store, shared, std::mem::take(&mut log_batch));
                    if let Err(error) = store.retain(&request) {
                        // History cleanup must not stop active tasks or later writes.
                        shared.report(Cause::Retention, || error_chain(&error));
                    }
                }
                Job::Flush(answer) => flushes.push(answer),
                Job::Close => closing = true,
            }
            if closing || batch.len().saturating_add(log_batch.len()) >= MAX_BATCH {
                break;
            }
            match jobs.try_recv() {
                Ok(job) => next = Some(job),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
            }
        }
        write_batch(&mut store, shared, batch);
        write_log_batch(&mut store, shared, log_batch);
        for answer in flushes {
            // The one who flushes may have given up waiting; there is nobody to tell then.
            if answer.send(()).is_err() {
                tracing::debug!("a flush of the history was not waited for");
            }
        }
    }
    count_what_is_left(jobs, shared);
}

/// Counts as lost the records still in the queue when the writer stops, and answers the flushes that wait.
fn count_what_is_left(jobs: &Receiver<Job>, shared: &Shared) {
    while let Ok(job) = jobs.try_recv() {
        match job {
            Job::Record(_) => shared.lose(1),
            Job::Flush(answer) => {
                if answer.send(()).is_err() {
                    tracing::debug!("a flush of the history was not waited for");
                }
            }
            Job::Retain(_) => shared.report(Cause::Retention, || "writer stopped before retention".to_owned()),
            Job::Plans(plans) => shared.lose(count(plans.len())),
            Job::Log(_) | Job::Close => {}
        }
    }
}

/// How many records `len` planned rows are, for the count of what is lost: a count that does not fit `u64` (a 128-bit
/// target) is the largest one there is, so that the loss stays visible.
#[expect(
    clippy::manual_unwrap_or,
    reason = "the fallback is the largest count, not a discarded failure: a usize that does not fit u64 reports every record as lost"
)]
fn count(len: usize) -> u64 {
    match u64::try_from(len) {
        Ok(count) => count,
        Err(_) => u64::MAX,
    }
}

/// Writes `plans`; when they cannot be written, each that failed is counted as lost.
fn write_plans(store: &mut impl Store, shared: &Shared, plans: &[super::plans::PlanRecord]) {
    if plans.is_empty() {
        return;
    }
    if let Err(error) = store.write_plans(plans) {
        shared.lose(count(plans.len()));
        shared.report(Cause::Write(error.kind()), || error_chain(&error));
    }
}

/// Writes `batch`; when the batch as a whole is refused, writes its records one by one, so that one that cannot be
/// written loses itself and nothing else.
fn write_batch(store: &mut impl Store, shared: &Shared, batch: Vec<Record>) {
    if batch.is_empty() {
        return;
    }
    let Err(error) = store.write(&batch) else {
        return;
    };
    if batch.len() == 1 {
        shared.lose_to(1, &error);
        return;
    }
    for record in batch {
        if let Err(error) = store.write(std::slice::from_ref(&record)) {
            shared.lose_to(1, &error);
        }
    }
}

/// Writes daemon log `batch`; when the batch as a whole fails, writes its entries one by one and reports failures directly
/// to `stderr` without incrementing [`Shared::lost`].
fn write_log_batch(store: &mut impl Store, shared: &Shared, batch: Vec<LogRecord>) {
    if batch.is_empty() {
        return;
    }
    let Err(error) = store.write_logs(&batch) else {
        return;
    };
    if batch.len() == 1 {
        shared.report_log(Cause::Write(error.kind()), || error_chain(&error));
        return;
    }
    for record in batch {
        if let Err(error) = store.write_logs(std::slice::from_ref(&record)) {
            shared.report_log(Cause::Write(error.kind()), || error_chain(&error));
        }
    }
}
