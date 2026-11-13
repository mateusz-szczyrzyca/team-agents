//! The gate service: the repository's gate command, one run at a time on the machine, in the order it was asked for.
//!
//! `agent gate` asks the owner, which checks the task and puts a [`GateJob`] on the queue; this service takes the jobs
//! one by one. A job runs the gate command in the task's worktree under a timeout, records the commit, whether the
//! tree was clean and how the command ended, and answers the client that waits. A run that waits for its gate holds
//! the watchdog off (`WatchdogHold`), so that its silence is not taken for a hang.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::num::{NonZeroI32, NonZeroU32, NonZeroUsize};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use agent_core::cli::ActivityEvent;
use agent_core::config::{AbsolutePath, CommandLine, EnvName, InvalidValue};
use agent_core::ids::{TaskId, TeamId};
use agent_core::protocol::{Event, EventKind, Failure, FailureKind, Reply};
use agent_core::state::{CommitId, GateOutcome, GateRecord, GateResult, GateTree, InvalidTimestamp, Timestamp};
use agent_runtime::clock::SharedClock;
use agent_runtime::git::{CommitIdentity, Git, GitError, MergeState, Revision, TreeState};
use agent_runtime::process::{ExitReport, ProcessError, ProcessSpec, Supervisor, error_chain};
use agent_runtime::run::HoldGuard;
use agent_runtime::store::{StateStore, StoreError};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// How many gates may wait for their turn; a gate asked for beyond that is refused.
pub(super) const QUEUE: usize = 16;

/// What the service is made of.
pub(super) struct GateParts {
    /// The store, for the layout of the home, the activity logs and the records.
    pub(super) store: Arc<StateStore>,
    /// The clock that stamps the records.
    pub(super) clock: SharedClock,
    /// Where the events go.
    pub(super) events: broadcast::Sender<Event>,
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
    /// Cancelled when the task is stopped or the daemon shuts down: a gate that waits is dropped, one that runs is
    /// stopped, and neither is recorded.
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
    // The failure that stands in for an exit code of zero that a signal would make: a signal number is positive, so the
    // sum is never zero; if it were, the gate failed all the same.
    let failed = |code: i32| GateOutcome::Failed {
        exit_code: NonZeroI32::new(code).unwrap_or(NonZeroI32::MIN),
    };
    match exit {
        ExitReport::Exited { code: 0 } => GateOutcome::Passed,
        ExitReport::Exited { code } => failed(code),
        ExitReport::Signaled { signal } => failed(SIGNAL_EXIT_BASE.saturating_add(signal)),
    }
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
    /// Runs the gate commands; it has one slot, which is the machine's one gate at a time.
    supervisor: Supervisor,
    /// The number the next log of each task is tried with.
    next_log: HashMap<TaskId, NonZeroU32>,
}

impl Service {
    /// A service made of `parts`.
    fn new(parts: GateParts) -> Self {
        Self {
            parts,
            supervisor: Supervisor::new(NonZeroUsize::MIN),
            next_log: HashMap::new(),
        }
    }

    /// Runs `job` and answers its client.
    async fn serve(&mut self, job: GateJob) {
        let outcome = self.execute(&job).await;
        let GateJob { task, reply, hold, .. } = job;
        // The run's silence is counted from here on: whatever the client does with the answer is the run's own.
        drop(hold);
        let answer = match outcome {
            Ok(record) => Ok(Reply::GateRecorded { record }),
            Err(error) => {
                tracing::warn!(task = %task, error = %error_chain(&error), "a gate was not run or not recorded");
                Err(error.failure())
            }
        };
        if reply.send(answer).is_err() {
            // The client went away while it waited: nobody is left to tell. The record, if any, is kept all the same.
            tracing::debug!(task = %task, "the client of a gate went away before its answer");
        }
    }

    /// Runs the gate of `job`: looks at the worktree, runs the command and records the result.
    async fn execute(&mut self, job: &GateJob) -> Result<GateRecord, GateError> {
        if job.cancel.is_cancelled() {
            return Err(GateError::Cancelled);
        }
        let (head, tree) = inspect(&job.worktree).await?;
        let (log, log_text) = self.open_log(&job.task).await?;
        let outcome = self.run_command(job, &log).await?;
        let record = GateRecord {
            tree,
            result: GateResult {
                head,
                finished_at: Timestamp::now(&self.parts.clock)?,
                outcome,
                log: log_text,
            },
        };
        self.parts
            .store
            .save_gate_record(&job.task, &record)
            .await
            .map_err(|source| GateError::Record {
                task: job.task.clone(),
                source,
            })?;
        self.announce(job, &record).await;
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
                Ok(_file) => {
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
        let directories = self.parts.agent_path.iter().map(AbsolutePath::as_path);
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
        Ok(ProcessSpec {
            program: job.command.program().clone(),
            args: job.command.args().as_slice().to_vec(),
            env: BTreeMap::from([(name, path)]),
            working_dir: job.worktree.clone(),
            // One file for both streams, as `> log 2>&1` makes it: both are opened for appending.
            stdout_file: log.to_path_buf(),
            stderr_file: log.to_path_buf(),
        })
    }

    /// Runs the command of `job` to its end, its timeout or its cancellation.
    async fn run_command(&self, job: &GateJob, log: &Path) -> Result<GateOutcome, GateError> {
        let spec = self.spec(job, log)?;
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
        let supervise = |source| GateError::Supervise { source };
        let grace = self.parts.stop_grace;
        match ended {
            Ended::Exited(exit) => {
                let exit = exit.map_err(supervise)?;
                // What the command started may outlive it; the gate is over, so the rest of its group goes too.
                process.stop(grace).await.map_err(supervise)?;
                Ok(outcome_of(exit))
            }
            Ended::TimedOut => {
                process.stop(grace).await.map_err(supervise)?;
                Ok(GateOutcome::TimedOut)
            }
            Ended::Cancelled => {
                process.stop(grace).await.map_err(supervise)?;
                Err(GateError::Cancelled)
            }
        }
    }

    /// Tells the subscribers and the activity log of the task that the gate ended.
    async fn announce(&self, job: &GateJob, record: &GateRecord) {
        let at = record.result.finished_at;
        let text = format!(
            "gate {}; output: {}",
            record.summary(),
            record.result.log.as_path().display()
        );
        if let Err(error) = self
            .parts
            .store
            .append_activity(&job.task, at, &ActivityEvent::Info(text))
            .await
        {
            // The log explains to a person what happened; the record does not depend on it.
            tracing::warn!(task = %job.task, error = %error_chain(&error), "the activity log cannot be written");
        }
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

/// What git says about the worktree: the commit it has out, and whether the tree is clean.
///
/// A merge that waits to be committed makes the tree not clean even when it changed nothing: what the gate sees is not
/// what the commit holds.
async fn inspect(worktree: &AbsolutePath) -> Result<(CommitId, GateTree), GateError> {
    let git = Git::new(
        worktree.clone(),
        CommitIdentity::new(LOOKUP_NAME.to_owned(), LOOKUP_EMAIL.to_owned(), None),
    );
    let failed = |source| GateError::Git {
        worktree: worktree.as_path().to_path_buf(),
        source,
    };
    let head = git.rev_parse(&Revision::Head).await.map_err(failed)?;
    let changes = git.tree_state().await.map_err(failed)?;
    let merge = git.merge_state().await.map_err(failed)?;
    let tree = match (changes, merge) {
        (TreeState::Clean, MergeState::NotMerging) => GateTree::Clean,
        (TreeState::Clean, MergeState::InProgress) | (TreeState::Dirty(_), _) => GateTree::NotClean,
    };
    Ok((head, tree))
}

/// Runs the jobs of `jobs`, one at a time in the order they arrive, until `shutdown` is cancelled or the queue is closed.
///
/// A job waits as long as the ones before it run; its `cancel` token drops it from the queue when its task is stopped.
pub(super) async fn run(mut jobs: mpsc::Receiver<GateJob>, parts: GateParts, shutdown: CancellationToken) {
    let mut service = Service::new(parts);
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

#[cfg(test)]
mod tests;
