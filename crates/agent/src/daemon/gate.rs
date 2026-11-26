//! The gate service: the repository's gate command, one run at a time on the machine, in the order it was asked for.
//!
//! `agent gate` asks the owner, which checks the task and puts a [`GateJob`] on the queue; this service takes the jobs
//! one by one. A job runs the gate command in the task's worktree under a timeout, records the commit, whether the
//! tree was clean and how the command ended, and answers the client that waits. A run that waits for its gate holds
//! the watchdog off (`WatchdogHold`), so that its silence is not taken for a hang.
//!
//! A gate asked again for what it has an answer for is not run: when the worktree is clean at the commit and the command
//! of the task's last gate that gated its commit, and that gate passed, or failed there [`FAILED_RUNS_PER_COMMIT`]
//! times in a row, the client gets that record with a log of its own: the output of that gate and a note that it was
//! not run again. The service remembers this while it runs; a restarted daemon runs the next gate of every task.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::num::{NonZeroI32, NonZeroU32, NonZeroUsize};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use agent_core::config::{AbsolutePath, Build, CommandLine, EnvName, InvalidValue};
use agent_core::history::{EventRecord, HistoryEvent, LostCause, LostKind, Record};
use agent_core::ids::{TaskId, TeamId};
use agent_core::log::LogEvent;
use agent_core::protocol::{Event, EventKind, Failure, FailureKind, Reply};
use agent_core::state::{CommitId, GateOutcome, GateRecord, GateResult, GateTree, InvalidTimestamp, Timestamp};
use agent_runtime::clock::SharedClock;
use agent_runtime::git::{CommitIdentity, Git, GitError, MergeState, Revision, TreeState};
use agent_runtime::history::{CaptureKind, History, log_outside_call};
use agent_runtime::process::{ExitReport, ProcessError, ProcessSpec, Supervisor, error_chain};
use agent_runtime::run::HoldGuard;
use agent_runtime::store::{StateStore, StoreError};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

/// How many gates may wait for their turn; a gate asked for beyond that is refused.
pub(super) const QUEUE: usize = 16;

/// How many times in a row a gate runs and fails on one clean commit with one command: one failure may be a flaky
/// test and is run once more, the gate asked for after the second is answered with that failure without running.
const FAILED_RUNS_PER_COMMIT: u32 = 2;

/// The exit code of a gate command that `tools/test-lock.sh` refused with "another tests are running now - please wait
/// before you try again": it ran no check, so it tells nothing about the commit and does not count as a failure.
const LOCK_BUSY_EXIT: i32 = 75;

/// What the service is made of.
pub(super) struct GateParts {
    /// The store, for the layout of the home, the activity logs and the records.
    pub(super) store: Arc<StateStore>,
    /// The clock that stamps the records.
    pub(super) clock: SharedClock,
    /// Where the events go.
    pub(super) events: broadcast::Sender<Event>,
    /// The directory of the daemon executable, prepended to `PATH`.
    pub(super) agent_binary_dir: AbsolutePath,
    /// The directories of the gate command's `PATH`: the agents' own.
    pub(super) agent_path: Vec<AbsolutePath>,
    /// How long a stopped gate has to end after `SIGTERM` before `SIGKILL`.
    pub(super) stop_grace: Duration,
}

/// One gate to run.
#[derive(Debug)]
pub(super) struct GateJob {
    /// The task whose worktree the gate runs in.
    pub(super) task: TaskId,
    /// The task's team.
    pub(super) team: TeamId,
    /// The worktree: where the command runs.
    pub(super) worktree: AbsolutePath,
    /// The repository's gate command.
    pub(super) command: CommandLine,
    /// How long the command may run before it is stopped and the gate counts as failed.
    pub(super) timeout: Duration,
    /// Cancelled when the task is stopped or paused or the daemon shuts down: a gate that runs is stopped, and one that
    /// waits is answered without running when its turn comes (it keeps its place in the queue until then); neither is
    /// recorded.
    pub(super) cancel: CancellationToken,
    /// Holds the watchdog off the task's run while the gate waits and runs; `None` when the task has no run going.
    pub(super) hold: Option<HoldGuard>,
    /// Where the answer goes: the client waits for it.
    pub(super) reply: oneshot::Sender<Result<Reply, Failure>>,
}

/// The way to the queue of the service.
#[derive(Debug, Clone)]
pub(super) struct GateQueue {
    /// The jobs waiting, oldest first.
    jobs: mpsc::Sender<GateJob>,
}

/// A job that could not be queued, given back so that its client can be answered.
#[derive(Debug)]
pub(super) enum NotQueued {
    /// [`QUEUE`] gates are waiting already.
    Full(Box<GateJob>),
    /// The service has ended: the daemon is shutting down.
    Stopped(Box<GateJob>),
}

impl GateQueue {
    /// A queue and the end the service takes jobs from.
    pub(super) fn new() -> (Self, mpsc::Receiver<GateJob>) {
        let (jobs, waiting) = mpsc::channel(QUEUE);
        (Self { jobs }, waiting)
    }

    /// Puts `job` behind the ones waiting.
    ///
    /// # Errors
    /// [`NotQueued`], with the job.
    pub(super) fn enqueue(&self, job: GateJob) -> Result<(), NotQueued> {
        self.jobs.try_send(job).map_err(|refused| match refused {
            mpsc::error::TrySendError::Full(job) => NotQueued::Full(Box::new(job)),
            mpsc::error::TrySendError::Closed(job) => NotQueued::Stopped(Box::new(job)),
        })
    }
}

/// The signal numbers are added to this to make the exit code a shell shows for a command that a signal ended.
const SIGNAL_EXIT_BASE: i32 = 128;

/// The name of the program git is run with for the questions about a worktree: no commit is made, but a handle has one.
const LOOKUP_NAME: &str = "team-agents";

/// The e-mail of [`LOOKUP_NAME`].
const LOOKUP_EMAIL: &str = "agent@localhost.invalid";

/// The environment variable the gate command's programs are looked up through.
const PATH_VARIABLE: &str = "PATH";

/// Why a gate could not be run or recorded.
#[derive(Debug, thiserror::Error)]
enum GateError {
    /// The task was stopped, or the daemon is shutting down, before the gate was over.
    #[error("the gate was cancelled: its task was stopped or the daemon is shutting down")]
    Cancelled,
    /// Git cannot say what the worktree has out.
    #[error("the gate cannot run in {worktree:?}: git cannot read the worktree")]
    Git {
        /// The worktree.
        worktree: PathBuf,
        /// What git said.
        source: GitError,
    },
    /// The gate ran, and git cannot say what its worktree has out afterwards.
    #[error(
        "the gate ran, but git cannot read the worktree {worktree:?} after it, so nothing is recorded; its output is in {log:?}"
    )]
    GitAfter {
        /// The worktree.
        worktree: PathBuf,
        /// The file with the output of the gate that ran.
        log: PathBuf,
        /// What git said.
        source: GitError,
    },
    /// The file for the gate's output cannot be made.
    #[error("the gate cannot run: cannot make its log {path:?}")]
    Log {
        /// The log.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// Every number of a task's gate logs is taken.
    #[error("the gate cannot run: task {task} has no gate log number left")]
    LogNumbers {
        /// The task.
        task: TaskId,
    },
    /// A path is not usable text.
    #[error("the gate cannot run: the path {path:?} is not valid")]
    Path {
        /// The path.
        path: PathBuf,
        /// What is wrong with it, when it is text.
        source: Option<InvalidValue>,
    },
    /// The `PATH` of the gate command cannot be made.
    #[error("the gate cannot run: the directories of agent_path cannot make a PATH")]
    AgentPath {
        /// What the standard library refused.
        source: std::env::JoinPathsError,
    },
    /// The name of an environment variable is not valid.
    #[error("the gate cannot run: {name:?} is not a valid name of an environment variable")]
    EnvName {
        /// The name.
        name: &'static str,
        /// What is wrong with it.
        source: InvalidValue,
    },
    /// The gate command cannot be started.
    #[error("the gate command cannot be started: check `gate` of the repository in agents.toml")]
    Start {
        /// What the supervisor said.
        source: ProcessError,
    },
    /// The gate command cannot be waited for or stopped.
    #[error("the gate command cannot be supervised")]
    Supervise {
        /// What the supervisor said.
        source: ProcessError,
    },
    /// The clock gives a time that cannot be recorded.
    #[error("the clock gives a time that cannot be recorded")]
    Clock(#[from] InvalidTimestamp),
    /// The record of a gate that ran cannot be saved.
    #[error("the gate ran but its record for task {task} cannot be saved")]
    Record {
        /// The task.
        task: TaskId,
        /// What the store said.
        source: StoreError,
    },
}

impl GateError {
    /// What the client is told: a refusal when the gate itself or its configuration is wrong, a conflict when it was
    /// cancelled, an internal failure for the rest.
    fn failure(&self) -> Failure {
        let kind = match self {
            Self::Cancelled => FailureKind::Conflict,
            Self::Git { .. } | Self::Start { .. } => FailureKind::Refused,
            Self::Log { .. }
            | Self::LogNumbers { .. }
            | Self::Path { .. }
            | Self::AgentPath { .. }
            | Self::EnvName { .. }
            | Self::GitAfter { .. }
            | Self::Supervise { .. }
            | Self::Clock(_)
            | Self::Record { .. } => FailureKind::Internal,
        };
        Failure {
            kind,
            message: error_chain(self),
        }
    }
}

/// How a gate command's end is recorded: a zero exit is a pass, any other code a failure with that code, and a signal
/// a failure with the code a shell shows for it.
fn outcome_of(exit: ExitReport) -> GateOutcome {
    match exit {
        ExitReport::Exited { code } => {
            NonZeroI32::new(code).map_or(GateOutcome::Passed, |exit_code| GateOutcome::Failed { exit_code })
        }
        ExitReport::Signaled { signal } => GateOutcome::Failed {
            // A signal number is positive, so the sum is never zero; a sum that were zero would still be a failure.
            exit_code: NonZeroI32::new(SIGNAL_EXIT_BASE.saturating_add(signal)).unwrap_or(NonZeroI32::MIN),
        },
    }
}

/// The name a log record gives `outcome`.
fn outcome_name(outcome: GateOutcome) -> &'static str {
    match outcome {
        GateOutcome::Passed => "passed",
        GateOutcome::Failed { .. } => "failed",
        GateOutcome::TimedOut => "timed_out",
    }
}

/// Whether `outcome` is the refusal of the busy test lock: the command exited with [`LOCK_BUSY_EXIT`].
fn refused_by_busy_lock(outcome: &GateOutcome) -> bool {
    matches!(outcome, GateOutcome::Failed { exit_code } if exit_code.get() == LOCK_BUSY_EXIT)
}

/// The last gate of a task that gated its commit (the tree was clean before and after it), as far as a gate asked again
/// is concerned.
struct LastGate {
    /// The commit it gated.
    head: CommitId,
    /// The command it ran: a gate asked with another command (`agents.toml` changed) is run.
    command: CommandLine,
    /// Its record, with the log of the gate that ran.
    record: GateRecord,
    /// How many gates ran and failed in a row on `head` with `command`, this one included; 0 when it passed.
    failures: u32,
}

/// What a wait for the gate command saw.
enum Ended {
    /// The command ended.
    Exited(Result<ExitReport, ProcessError>),
    /// It ran over its timeout.
    TimedOut,
    /// The job was cancelled.
    Cancelled,
}

/// The service: the state of the loop that runs the jobs.
struct Service {
    /// What it is made of.
    parts: GateParts,
    /// Cargo cache policy for every command this service launches.
    build: Build,
    /// Where the gates that ran are recorded.
    history: History,
    /// Runs the gate commands; it has one slot, which is the machine's one gate at a time.
    supervisor: Supervisor,
    /// The number the next log of each task is tried with.
    next_log: HashMap<TaskId, NonZeroU32>,
    /// The last gate of each task that gated its commit: one entry a task, kept while the service runs.
    last: HashMap<TaskId, LastGate>,
}

impl Service {
    /// A service made of `parts`.
    fn new(parts: GateParts) -> Self {
        Self {
            parts,
            build: Build::default(),
            history: History::disabled(),
            supervisor: Supervisor::new(NonZeroUsize::MIN),
            next_log: HashMap::new(),
            last: HashMap::new(),
        }
    }

    /// The same service with the configured Cargo cache policy.
    fn with_build(mut self, build: Build) -> Self {
        self.build = build;
        self
    }

    /// The same service, recording the gates that run in `history`.
    fn with_history(mut self, history: History) -> Self {
        self.history = history;
        self
    }

    /// Runs `job` and answers its client.
    async fn serve(&mut self, job: GateJob) {
        let span = tracing::error_span!("gate", team = %job.team, task = %job.task);
        let outcome = self.execute(&job).instrument(span.clone()).await;
        let GateJob { task, reply, hold, .. } = job;
        // The run's silence is counted from here on: whatever the client does with the answer is the run's own.
        drop(hold);
        let answer = match outcome {
            Ok(record) => Ok(Reply::GateRecorded { record }),
            Err(error) => {
                let _entered = span.enter();
                if matches!(error, GateError::Cancelled) {
                    tracing::warn!(
                        event = LogEvent::RequestRefused.as_str(),
                        task = %task,
                        error = %error_chain(&error),
                        "a gate was not run or not recorded"
                    );
                } else {
                    tracing::error!(
                        event = LogEvent::GateCannotStart.as_str(),
                        task = %task,
                        error = %error_chain(&error),
                        "a gate was not run or not recorded"
                    );
                }
                Err(error.failure())
            }
        };
        if reply.send(answer).is_err() {
            // The client went away while it waited: nobody is left to tell. The record, if any, is kept all the same.
            tracing::debug!(
                event = LogEvent::PeerGone.as_str(),
                task = %task,
                "the client of a gate went away before its answer"
            );
        }
    }

    /// Runs the gate of `job`: looks at the worktree, runs the command and records the result.
    async fn execute(&mut self, job: &GateJob) -> Result<GateRecord, GateError> {
        if job.cancel.is_cancelled() {
            return Err(GateError::Cancelled);
        }
        let started_at = Timestamp::now(&self.parts.clock);
        let before = inspect(&job.worktree)
            .await
            .map_err(|source| GateError::Git {
                worktree: job.worktree.as_path().to_path_buf(),
                source,
            })
            .inspect_err(|error| self.record_capture_loss_or_clock(job, started_at.as_ref().ok().copied(), error))?;
        if let Some(last) = self.reusable(job, &before) {
            let (record, failures) = (last.record.clone(), last.failures);
            return self.reuse(job, record, failures).await;
        }
        let (log, log_text) = self.open_log(&job.task).await?;
        let outcome = self.run_command(job, &log).await?;
        if matches!(outcome, GateOutcome::TimedOut) {
            tracing::warn!(
                event = LogEvent::GateTimedOut.as_str(),
                task = %job.task,
                timeout_ms = u64::try_from(job.timeout.as_millis()).unwrap_or(u64::MAX),
                "a gate timed out"
            );
        }
        let finished_at = Timestamp::now(&self.parts.clock)
            .inspect_err(|error| self.record_capture_loss_or_clock(job, started_at.as_ref().ok().copied(), error))?;
        // The command has ended with its whole group: what it left in the worktree is final. A gate that changed the
        // tree or moved `HEAD` did not gate what the commit holds.
        let after = inspect(&job.worktree)
            .await
            .map_err(|source| GateError::GitAfter {
                worktree: job.worktree.as_path().to_path_buf(),
                log: log.clone(),
                source,
            })
            .inspect_err(|error| {
                self.record_capture_loss(job, finished_at, error);
            })?;
        let head = before.head.clone();
        let record = GateRecord {
            tree: before.merged_with(&after),
            result: GateResult {
                head,
                finished_at,
                outcome,
                log: log_text,
            },
        };
        if matches!(record.tree, GateTree::NotClean) {
            tracing::warn!(
                event = LogEvent::GateDirtyTree.as_str(),
                task = %job.task,
                "a gate left a dirty worktree or moved HEAD"
            );
        }
        match started_at {
            Ok(started_at) => self.record(job, started_at, &record),
            Err(ref error) => self.record_capture_loss(job, finished_at, error),
        }
        self.parts
            .store
            .save_gate_record(&job.task, &record)
            .await
            .map_err(|source| GateError::Record {
                task: job.task.clone(),
                source,
            })?;
        self.remember(job, &record);
        self.announce(job, &record);
        Ok(record)
    }

    /// The last gate of `job`'s task, when `job` is answered with it without running: the worktree is clean at the
    /// commit that gate gated, the command is the same, and the gate passed or failed there [`FAILED_RUNS_PER_COMMIT`]
    /// times in a row.
    fn reusable(&self, job: &GateJob, before: &Inspected) -> Option<&LastGate> {
        let last = self.last.get(&job.task)?;
        let same = before.tree == GateTree::Clean && last.head == before.head && last.command == job.command;
        let settled = last.record.result.outcome == GateOutcome::Passed || last.failures >= FAILED_RUNS_PER_COMMIT;
        (same && settled).then_some(last)
    }

    /// Keeps `record` of a gate of `job` that ran, when it gated its commit: a gate that did not (the tree was not
    /// clean, or the busy test lock refused it with [`LOCK_BUSY_EXIT`]) tells nothing about the commit, and the gate
    /// before it is kept.
    fn remember(&mut self, job: &GateJob, record: &GateRecord) {
        if record.tree != GateTree::Clean || refused_by_busy_lock(&record.result.outcome) {
            return;
        }
        let failures = match record.result.outcome {
            GateOutcome::Passed => 0,
            GateOutcome::Failed { .. } | GateOutcome::TimedOut => match self.last.get(&job.task) {
                Some(last) if last.head == record.result.head && last.command == job.command => {
                    last.failures.saturating_add(1)
                }
                Some(_) | None => 1,
            },
        };
        self.last.insert(
            job.task.clone(),
            LastGate {
                head: record.result.head.clone(),
                command: job.command.clone(),
                record: record.clone(),
                failures,
            },
        );
    }

    /// Answers `job` with `record` of the gate that ran, without running it: the answer gets a log of its own with the
    /// output of that gate and, as its last line, why it was not run again and what to do. The answer is saved as the
    /// task's record and told to the subscribers, as a gate that ran would be; the history keeps the gate that ran.
    async fn reuse(&mut self, job: &GateJob, mut record: GateRecord, failures: u32) -> Result<GateRecord, GateError> {
        let (log, log_text) = self.open_log(&job.task).await?;
        let head = record.result.head.as_str().to_owned();
        let note = if failures == 0 {
            format!(
                "gate: not run again: it passed on commit {head} at {} with the same command, and the tree has not changed since; the output above is that gate's. A new commit is gated again.",
                record.result.finished_at
            )
        } else {
            format!(
                "gate: not run again: it failed {failures} times in a row on commit {head} with the same command, and the tree has not changed since; the output above is the last one's. Fix the cause and commit, or, when the failure is not caused by your change (a flaky test, a broken tool), say so in your questions file and wait for the answer."
            )
        };
        let earlier = record.result.log.as_path();
        let mut text = match tokio::fs::read(earlier).await {
            Ok(text) => text,
            // The note matters more than the output: it is told without it, and the reader is told why.
            Err(error) => format!(
                "gate: the output of that gate cannot be read from {}: {error}\n",
                earlier.display()
            )
            .into_bytes(),
        };
        if !text.is_empty() && !text.ends_with(b"\n") {
            text.push(b'\n');
        }
        text.extend_from_slice(note.as_bytes());
        text.push(b'\n');
        tokio::fs::write(&log, text).await.map_err(|source| GateError::Log {
            path: log.clone(),
            source,
        })?;
        record.result.log = log_text;
        self.parts
            .store
            .save_gate_record(&job.task, &record)
            .await
            .map_err(|source| GateError::Record {
                task: job.task.clone(),
                source,
            })?;
        // The gates that ran are in the history; this is the one place a gate answered without running is told.
        tracing::info!(
            event = LogEvent::ChecksDecided.as_str(),
            outcome = "gate_reused",
            task = %job.task,
            head = head.as_str(),
            gate = outcome_name(record.result.outcome),
            failures,
            "a gate was answered with the gate that ran on the same commit, without running"
        );
        self.announce(job, &record);
        Ok(record)
    }

    /// Makes the file for the output of the next gate of `task`: the first number nobody used.
    async fn open_log(&mut self, task: &TaskId) -> Result<(PathBuf, AbsolutePath), GateError> {
        let home = self.parts.store.home();
        let dir = home.task_dir(task);
        tokio::fs::create_dir_all(&dir).await.map_err(|source| GateError::Log {
            path: dir.clone(),
            source,
        })?;
        let mut number = self.next_log.get(task).copied().unwrap_or(NonZeroU32::MIN);
        loop {
            let path = home.gate_log(task, number);
            let made = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .await;
            match made {
                // The file is made only to claim its number: the gate command writes it.
                Ok(_claimed) => {
                    let next = number
                        .checked_add(1)
                        .ok_or_else(|| GateError::LogNumbers { task: task.clone() })?;
                    self.next_log.insert(task.clone(), next);
                    let text = path.to_str().ok_or_else(|| GateError::Path {
                        path: path.clone(),
                        source: None,
                    })?;
                    let absolute = AbsolutePath::from_str(text).map_err(|source| GateError::Path {
                        path: path.clone(),
                        source: Some(source),
                    })?;
                    return Ok((path, absolute));
                }
                // A log of an earlier gate (the daemon was restarted since): the next number is tried.
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                    number = number
                        .checked_add(1)
                        .ok_or_else(|| GateError::LogNumbers { task: task.clone() })?;
                }
                Err(source) => return Err(GateError::Log { path, source }),
            }
        }
    }

    /// The process of `job`, its output going to `log`.
    fn spec(&self, job: &GateJob, log: &Path) -> Result<ProcessSpec, GateError> {
        let directories =
            agent_runtime::run::agent_path_directories(self.parts.agent_binary_dir.as_path(), &self.parts.agent_path);
        let path = std::env::join_paths(directories)
            .map_err(|source| GateError::AgentPath { source })?
            .into_string()
            .map_err(|path| GateError::Path {
                path: path.into(),
                source: None,
            })?;
        let name = EnvName::from_str(PATH_VARIABLE).map_err(|source| GateError::EnvName {
            name: PATH_VARIABLE,
            source,
        })?;
        let incremental = EnvName::from_str("CARGO_INCREMENTAL").map_err(|source| GateError::EnvName {
            name: "CARGO_INCREMENTAL",
            source,
        })?;
        let value = if self.build.incremental { "1" } else { "0" };
        Ok(ProcessSpec {
            program: job.command.program().clone(),
            args: job.command.args().as_slice().to_vec(),
            env: BTreeMap::from([(name, path), (incremental, value.to_owned())]),
            working_dir: job.worktree.clone(),
            // One file for both streams, as `> log 2>&1` makes it: both are opened for appending.
            stdout_file: log.to_path_buf(),
            stderr_file: log.to_path_buf(),
        })
    }

    /// Runs the command of `job` to its end, its timeout or its cancellation.
    async fn run_command(&self, job: &GateJob, log: &Path) -> Result<GateOutcome, GateError> {
        let spec = self.spec(job, log)?;
        let started = std::time::Instant::now();
        let command_args: String = std::iter::once(spec.program.as_path().display().to_string())
            .chain(spec.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        let mut process = self
            .supervisor
            .spawn(&spec)
            .await
            .map_err(|source| GateError::Start { source })?;
        let ended = tokio::select! {
            exit = process.wait() => Ended::Exited(exit),
            () = tokio::time::sleep(job.timeout) => Ended::TimedOut,
            () = job.cancel.cancelled() => Ended::Cancelled,
        };
        let duration = started.elapsed();
        let supervise = |source| GateError::Supervise { source };
        let grace = self.parts.stop_grace;
        match ended {
            Ended::Exited(exit) => {
                let exit = exit.map_err(supervise)?;
                // What the command started may outlive it; the gate is over, so the rest of its group goes too.
                process.stop(grace).await.map_err(supervise)?;
                let code = match exit {
                    ExitReport::Exited { code } => Some(code),
                    ExitReport::Signaled { .. } => None,
                };
                log_outside_call("gate", &command_args, duration, code, "exited");
                Ok(outcome_of(exit))
            }
            Ended::TimedOut => {
                process.stop(grace).await.map_err(supervise)?;
                log_outside_call("gate", &command_args, duration, None, "timed_out");
                Ok(GateOutcome::TimedOut)
            }
            Ended::Cancelled => {
                process.stop(grace).await.map_err(supervise)?;
                log_outside_call("gate", &command_args, duration, None, "cancelled");
                Err(GateError::Cancelled)
            }
        }
    }

    /// Hands the history the gate that ran. The rounds are left to the ledger, which knows the task's runs: this service
    /// holds no state of rounds.
    fn record(&self, job: &GateJob, started_at: Timestamp, record: &GateRecord) {
        self.history.record(Record::Event(Box::new(EventRecord {
            at: record.result.finished_at,
            team: job.team.clone(),
            rounds: Vec::new(),
            event: HistoryEvent::Gate {
                task: job.task.clone(),
                started_at,
                record: record.clone(),
            },
        })));
    }

    /// Counts a gate capture failure at `at`, or without a note when `started_at` itself failed.
    fn record_capture_loss_or_clock(&self, job: &GateJob, at: Option<Timestamp>, error: &impl std::error::Error) {
        if let Some(at) = at {
            self.record_capture_loss(job, at, error);
        } else {
            self.history.capture_failed(CaptureKind::Gate, error);
        }
    }

    /// Counts a gate whose worktree state could not be captured, and notes the loss for its round when the history is writing.
    fn record_capture_loss(&self, job: &GateJob, at: Timestamp, error: &impl std::error::Error) {
        self.history.capture_failed(CaptureKind::Gate, error);
        if self.history.is_writing() {
            self.history.record(Record::Event(Box::new(EventRecord {
                at,
                team: job.team.clone(),
                rounds: Vec::new(),
                event: HistoryEvent::Lost {
                    task: Some(job.task.clone()),
                    what: LostKind::Event(agent_core::history::EventKind::Gate),
                    cause: LostCause::CaptureGate,
                    detail: error_chain(error),
                },
            })));
        }
    }

    /// Tells subscribers that the gate ended; its record is already in history.
    fn announce(&self, job: &GateJob, record: &GateRecord) {
        let at = record.result.finished_at;
        let finished = EventKind::GateFinished {
            team: job.team.clone(),
            task: job.task.clone(),
            outcome: record.result.outcome,
        };
        if self.parts.events.send(Event::new(at, finished)).is_err() {
            // `send` fails when nobody is subscribed: the event is for nobody.
            tracing::trace!("an event had no subscriber");
        }
    }
}

/// What git says about the worktree at one moment: the commit it has out, and whether the tree is clean.
struct Inspected {
    /// The commit the worktree has out.
    head: CommitId,
    /// Whether the tree holds exactly that commit.
    tree: GateTree,
}

impl Inspected {
    /// Whether a gate that began at `self` and ended at `after` gated its commit: the tree was clean at both ends and
    /// `HEAD` did not move.
    fn merged_with(&self, after: &Self) -> GateTree {
        if self.tree == GateTree::Clean && after.tree == GateTree::Clean && self.head == after.head {
            GateTree::Clean
        } else {
            GateTree::NotClean
        }
    }
}

/// What git says about the worktree: the commit it has out, and whether the tree is clean.
///
/// A merge that waits to be committed makes the tree not clean even when it changed nothing: what the gate sees is not
/// what the commit holds.
async fn inspect(worktree: &AbsolutePath) -> Result<Inspected, GitError> {
    let git = Git::new(
        worktree.clone(),
        CommitIdentity::new(LOOKUP_NAME.to_owned(), LOOKUP_EMAIL.to_owned(), None),
    );
    let head = git.rev_parse(&Revision::Head).await?;
    let changes = git.tree_state().await?;
    let merge = git.merge_state().await?;
    let tree = match (changes, merge) {
        (TreeState::Clean, MergeState::NotMerging) => GateTree::Clean,
        (TreeState::Clean, MergeState::InProgress) | (TreeState::Dirty(_), _) => GateTree::NotClean,
    };
    Ok(Inspected { head, tree })
}

/// Runs the jobs of `jobs`, one at a time in the order they arrive, until `shutdown` is cancelled or the queue is closed.
///
/// A job waits as long as the ones before it run; when its task is stopped it keeps its place, and is answered
/// `conflict` without running when its turn comes.
pub(super) async fn run_configured(
    mut jobs: mpsc::Receiver<GateJob>,
    parts: GateParts,
    history: History,
    build: Build,
    shutdown: CancellationToken,
) {
    let mut service = Service::new(parts).with_history(history).with_build(build);
    loop {
        let job = tokio::select! {
            () = shutdown.cancelled() => break,
            job = jobs.recv() => match job {
                Some(job) => job,
                None => break,
            },
        };
        service.serve(job).await;
    }
}

/// Runs a service with the default build policy for callers that do not supply configuration.
#[cfg(test)]
pub(super) async fn run_recorded(
    jobs: mpsc::Receiver<GateJob>,
    parts: GateParts,
    history: History,
    shutdown: CancellationToken,
) {
    run_configured(jobs, parts, history, Build::default(), shutdown).await;
}

/// [`run_recorded`] with a history that records nothing: what the tests of the service run.
#[cfg(test)]
pub(super) async fn run(jobs: mpsc::Receiver<GateJob>, parts: GateParts, shutdown: CancellationToken) {
    run_recorded(jobs, parts, History::disabled(), shutdown).await;
}

#[cfg(test)]
mod tests;
