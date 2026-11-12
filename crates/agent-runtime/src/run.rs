//! The run engine: everything one run of a model CLI needs, from its files to its classified end.
//!
//! [`RunEngine::run`] builds the run through the CLI's adapter, lays down the files the adapter asks for, starts the
//! CLI under the supervisor and follows its output: every line goes to the adapter's parser and its events to the
//! task's activity log, and a run with no activity for the watchdog's limit is killed. When the process is over, the
//! final message, the error tail and the exit status are classified (`agent_core::decide::outcome`). The engine
//! touches no state: what the end means for the task is the daemon's decision.

pub mod compose;
mod launch;
mod output;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use agent_core::cli::{ActivityEvent, BuildRunError, McpServerKind, OutputError, OutputParser, RunMode, SessionId};
use agent_core::config::{AbsolutePath, CliArgs, CommandLine, Config, InvalidValue, ModelName, Secrets, Watchdog};
use agent_core::decide::outcome::{
    Classified, ClassifyError, ExitStatus, LimitContext, RunFinish, StoppedBy, classify_outcome,
};
use agent_core::ids::{RunId, TaskId};
use agent_core::kinds::CliKind;
use agent_core::prompt::VariablesError;
use agent_core::state::{InvalidTimestamp, Timestamp};
use time::UtcOffset;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::clock::SharedClock;
use crate::git::GitError;
use crate::harness_prompts::{HarnessPrompts, RenderTextError};
use crate::process::{ExitReport, ProcessError, Supervisor, error_chain};
use crate::run::output::{OutputTail, read_end};
use crate::store::{StateStore, StoreError};

/// How long a prompt may be to travel as one argument of a command line; longer ones go through a file. Linux refuses
/// an argument over 128 KiB, and a CLI adds its own flags around it.
pub const MAX_PROMPT_ARGUMENT_BYTES: usize = 100_000;

/// How often a run's output is looked at, unless the timings say otherwise.
const DEFAULT_POLL: Duration = Duration::from_secs(1);

/// How long a stopped run has to end by itself, unless the timings say otherwise.
const DEFAULT_STOP_GRACE: Duration = Duration::from_secs(10);

/// The shortest time between two looks: a zero would make the engine spin.
const MIN_POLL: Duration = Duration::from_millis(1);

/// How often the engine looks at a run's output, and how long it waits for a stopped one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunTimings {
    /// The time between two looks at the output of a run: for new lines and for the watchdog.
    pub poll: Duration,
    /// The silence after which a run is hung.
    pub hang: Duration,
    /// How long a stopped run has to end after `SIGTERM` before `SIGKILL`.
    pub stop_grace: Duration,
}

impl RunTimings {
    /// The timings of the configuration: its hang limit, a look every second and a grace of ten seconds.
    #[must_use]
    pub fn from_config(watchdog: &Watchdog) -> Self {
        Self {
            poll: DEFAULT_POLL,
            hang: watchdog.hang_minutes.as_duration(),
            stop_grace: DEFAULT_STOP_GRACE,
        }
    }
}

/// What a [`RunEngine`] is made of.
#[derive(Debug, Clone)]
pub struct EngineParts {
    /// The store, for the activity logs and the layout of the home.
    pub store: Arc<StateStore>,
    /// The texts of the harness, for the prompt file.
    pub texts: Arc<HarnessPrompts>,
    /// The configuration: the programs, the `PATH` of the agents and the limits.
    pub config: Arc<Config>,
    /// The secrets: the environment of every run.
    pub secrets: Arc<Secrets>,
    /// The runs the engine may have going at once.
    pub supervisor: Supervisor,
    /// The clock that stamps the activity and the end of a run.
    pub clock: SharedClock,
    /// The machine's offset from UTC, with which a reset time without a zone is read.
    pub local_offset: UtcOffset,
    /// How often and how long.
    pub timings: RunTimings,
}

/// One run to carry out.
#[derive(Debug, Clone)]
pub struct RunJob {
    /// The task the run belongs to.
    pub task: TaskId,
    /// The run's ID: the name of its directory.
    pub run: RunId,
    /// The model CLI.
    pub cli: CliKind,
    /// The model, or the CLI's default.
    pub model: Option<ModelName>,
    /// The agent's extra arguments for the CLI.
    pub extra_args: CliArgs,
    /// A new session, or the one to resume.
    pub mode: RunMode,
    /// The whole prompt.
    pub prompt: String,
    /// The agent's worktree: where the CLI runs.
    pub working_dir: AbsolutePath,
    /// The MCP servers the run gets, each with its whole command line.
    pub mcp_servers: BTreeMap<McpServerKind, CommandLine>,
}

/// A run that went to its end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedRun {
    /// How it ended, and the pause that implies.
    pub classified: Classified,
    /// How the process ended and who ended it.
    pub finish: RunFinish,
    /// The session the CLI named last: the one a later run resumes.
    pub session: Option<SessionId>,
    /// The agent's final message.
    pub final_message: Option<String>,
    /// Whether the run did anything but start.
    pub progress: Progress,
}

/// Whether a run did anything but start: what tells a run that hangs after some work from one that hangs at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// It started, and printed nothing more than information or errors.
    Nothing,
    /// It made a step, an edit or a message.
    Made,
}

/// What became of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunVerdict {
    /// The process ended, by itself or stopped by the watchdog.
    Finished(Box<FinishedRun>),
    /// The engine was told to stop (the daemon shuts down): the process was stopped and nothing is classified.
    Interrupted {
        /// The session the CLI had named by then.
        session: Option<SessionId>,
    },
}

/// Why a run could not be carried out.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The adapter cannot build the run.
    #[error("cannot build the run of {task}")]
    Build {
        /// The task.
        task: TaskId,
        /// What the adapter refused.
        source: BuildRunError,
    },
    /// A path is not UTF-8 text, which the adapters need.
    #[error("the path {path:?} is not UTF-8 text")]
    PathNotUtf8 {
        /// The path.
        path: PathBuf,
    },
    /// A path is not a valid absolute path.
    #[error("the path {path:?} is not a valid absolute path")]
    Path {
        /// The path.
        path: PathBuf,
        /// What is wrong with it.
        source: InvalidValue,
    },
    /// A file the adapter asked for cannot be written.
    #[error("cannot write the file {path:?} of the run")]
    LayDown {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
    /// The worktree's `info/exclude` cannot be found or written.
    #[error("cannot keep files of the run out of git in {worktree:?}")]
    Exclude {
        /// The worktree.
        worktree: PathBuf,
        /// What went wrong.
        source: ExcludeCause,
    },
    /// A name of an environment variable of the program is not valid.
    #[error("{name:?} is not a valid name of an environment variable")]
    EnvName {
        /// The name.
        name: &'static str,
        /// What is wrong with it.
        source: InvalidValue,
    },
    /// The directories of `[binaries] agent_path` cannot be made a `PATH`.
    #[error("cannot make a PATH of the directories of agent_path")]
    AgentPath {
        /// What the standard library refused.
        source: std::env::JoinPathsError,
    },
    /// A variable of the text that points to the prompt file is not valid.
    #[error("a variable of the text that points to the prompt file is not valid")]
    PromptFileVariable {
        /// What is wrong with the name.
        source: VariablesError,
    },
    /// The text that sends the agent to its prompt file cannot be rendered.
    #[error("cannot render the text that points to the prompt file")]
    PromptFile {
        /// What went wrong.
        source: RenderTextError,
    },
    /// The process could not be started.
    #[error("cannot start the {cli} run of {task}")]
    Spawn {
        /// The task.
        task: TaskId,
        /// The CLI.
        cli: CliKind,
        /// What the supervisor said.
        source: ProcessError,
    },
    /// The process could not be waited for, watched or stopped.
    #[error("cannot supervise the run of {task}")]
    Supervise {
        /// The task.
        task: TaskId,
        /// What the supervisor said.
        source: ProcessError,
    },
    /// The output of the run cannot be read.
    #[error("cannot read the output file {path:?} of the run")]
    Output {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
    /// The activity of the run cannot be logged.
    #[error("cannot log the activity of the run of {task}")]
    Activity {
        /// The task.
        task: TaskId,
        /// What the store said.
        source: StoreError,
    },
    /// The end of the run cannot be classified.
    #[error("cannot classify the end of the run of {task}")]
    Classify {
        /// The task.
        task: TaskId,
        /// What the classifier refused.
        source: ClassifyError,
    },
    /// The time of the clock cannot be a timestamp.
    #[error("the clock gives a time that cannot be recorded")]
    Clock(#[from] InvalidTimestamp),
}

/// Why the file `info/exclude` of a worktree cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum ExcludeCause {
    /// Git cannot say where the file is.
    #[error("git cannot say where info/exclude is")]
    Git(#[from] GitError),
    /// The file cannot be read or written.
    #[error("the file {path:?} cannot be read or written")]
    Io {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
}

/// Carries out runs of model CLIs.
#[derive(Debug, Clone)]
pub struct RunEngine {
    /// What the engine is made of.
    parts: EngineParts,
}

impl RunEngine {
    /// An engine made of `parts`.
    #[must_use]
    pub const fn new(parts: EngineParts) -> Self {
        Self { parts }
    }

    /// Carries out `job` and returns how it ended.
    ///
    /// Cancelling `cancel` stops the process (its whole group) and returns [`RunVerdict::Interrupted`].
    ///
    /// # Errors
    /// [`RunError`]: the run could not be built, started, followed or classified. A process that was started is
    /// stopped when the error is returned.
    pub async fn run(&self, job: RunJob, cancel: &CancellationToken) -> Result<RunVerdict, RunError> {
        let launch = launch::prepare(&self.parts, &job).await?;
        let stdout = launch.process.stdout_file.clone();
        let stderr = launch.process.stderr_file.clone();
        let mut process = self
            .parts
            .supervisor
            .spawn(&launch.process)
            .await
            .map_err(|source| RunError::Spawn {
                task: job.task.clone(),
                cli: job.cli,
                source,
            })?;
        let mut follow = Follow {
            engine: self,
            task: &job.task,
            tail: OutputTail::new(stdout),
            parser: launch.parser,
            progress: Progress::Nothing,
            unreadable: Unreadable::default(),
        };
        let supervise = |source| RunError::Supervise {
            task: job.task.clone(),
            source,
        };
        let mut tick = tokio::time::interval(self.parts.timings.poll.max(MIN_POLL));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let stopped_by = loop {
            let event = tokio::select! {
                exit = process.wait() => Watched::Exited(exit),
                () = cancel.cancelled() => Watched::Cancelled,
                _ = tick.tick() => Watched::Tick,
            };
            match event {
                Watched::Exited(exit) => {
                    let exit = exit.map_err(supervise)?;
                    // What the leader started may outlive it; the run is over, so the rest of its group goes too.
                    process.stop(self.parts.timings.stop_grace).await.map_err(supervise)?;
                    break Ended::By(StoppedBy::Nobody, exit);
                }
                Watched::Cancelled => {
                    process.stop(self.parts.timings.stop_grace).await.map_err(supervise)?;
                    break Ended::Cancelled;
                }
                Watched::Tick => {
                    follow.read_new().await?;
                    let last = process.last_activity().await.map_err(supervise)?;
                    if self.is_silent(last) {
                        let report = process.stop(self.parts.timings.stop_grace).await.map_err(supervise)?;
                        break Ended::By(StoppedBy::Watchdog, report.exit);
                    }
                }
            }
        };
        follow.drain().await?;
        let stderr = read_end(&stderr).await?;
        follow.unreadable.report(&job);
        let output = follow.parser.finish(&stderr, self.parts.config.limits().tail_lines);
        let (stopped_by, exit) = match stopped_by {
            Ended::Cancelled => {
                return Ok(RunVerdict::Interrupted {
                    session: output.session_id,
                });
            }
            Ended::By(stopped_by, exit) => (stopped_by, exit),
        };
        let finish = RunFinish {
            exit: match exit {
                ExitReport::Exited { code } => ExitStatus::Code(code),
                ExitReport::Signaled { .. } => ExitStatus::Signal,
            },
            stopped_by,
        };
        let context = LimitContext {
            limits: self.parts.config.limits(),
            now: Timestamp::now(&self.parts.clock)?,
            local_offset: self.parts.local_offset,
        };
        let classified = classify_outcome(&output, finish, &context).map_err(|source| RunError::Classify {
            task: job.task.clone(),
            source,
        })?;
        Ok(RunVerdict::Finished(Box::new(FinishedRun {
            classified,
            finish,
            session: output.session_id,
            final_message: output.final_message,
            progress: follow.progress,
        })))
    }

    /// Whether a process that last showed life at `last` has been silent for the hang limit.
    fn is_silent(&self, last: SystemTime) -> bool {
        // A modification time in the future (the clock was set back) is no silence at all.
        let silent_for = SystemTime::now().duration_since(last).unwrap_or(Duration::ZERO);
        silent_for > self.parts.timings.hang
    }
}

/// What the engine's wait for a run saw.
enum Watched {
    /// The process ended.
    Exited(Result<ExitReport, ProcessError>),
    /// The engine was told to stop.
    Cancelled,
    /// It is time to look at the output.
    Tick,
}

/// How the supervised process came to its end.
enum Ended {
    /// By itself or by the watchdog.
    By(StoppedBy, ExitReport),
    /// Because the engine was cancelled.
    Cancelled,
}

/// The lines of output that could not be read, which are logged once at the end of the run.
#[derive(Debug, Default)]
struct Unreadable {
    /// How many.
    count: u64,
    /// Why the first could not be.
    first: Option<String>,
}

impl Unreadable {
    /// Counts one more.
    fn note(&mut self, error: &OutputError) {
        self.count = self.count.saturating_add(1);
        if self.first.is_none() {
            self.first = Some(error_chain(error));
        }
    }

    /// Logs what was not readable in the run of `job`, if anything.
    fn report(&self, job: &RunJob) {
        if let Some(first) = &self.first {
            tracing::warn!(
                task = %job.task,
                run = %job.run,
                count = self.count,
                first = %first,
                "lines of the output of a run could not be read",
            );
        }
    }
}

/// What the engine does with the output of one run as it grows.
struct Follow<'a> {
    /// The engine, for the store and the clock.
    engine: &'a RunEngine,
    /// The task.
    task: &'a TaskId,
    /// The standard output file.
    tail: OutputTail,
    /// The adapter's parser.
    parser: Box<dyn OutputParser>,
    /// Whether an event that is more than the start or an error was seen.
    progress: Progress,
    /// What the parser refused.
    unreadable: Unreadable,
}

impl Follow<'_> {
    /// Feeds the lines written since the last call to the parser and its events to the activity log.
    async fn read_new(&mut self) -> Result<(), RunError> {
        let lines = self.tail.read_new().await?;
        self.feed(&lines).await
    }

    /// Reads what is left of the output after the process ended, the last line without a newline included.
    async fn drain(&mut self) -> Result<(), RunError> {
        // One look reads a bounded part: the end reads, part by part, all that was written when the process was over.
        let end = self.tail.len().await?;
        loop {
            let before = self.tail.offset();
            self.read_new().await?;
            if self.tail.offset() >= end || self.tail.offset() == before {
                break;
            }
        }
        if let Some(last) = self.tail.finish() {
            self.feed(&[last]).await?;
        }
        if self.tail.dropped() > 0 {
            tracing::warn!(task = %self.task, dropped = self.tail.dropped(), "lines of the output were too long or not text");
        }
        Ok(())
    }

    /// Gives `lines` to the parser and logs the events.
    async fn feed(&mut self, lines: &[String]) -> Result<(), RunError> {
        for line in lines {
            let events = match self.parser.push_line(line) {
                Ok(events) => events,
                Err(error) => {
                    self.unreadable.note(&error);
                    continue;
                }
            };
            for event in events {
                if !matches!(event, ActivityEvent::Info(_) | ActivityEvent::Error(_)) {
                    self.progress = Progress::Made;
                }
                let at = Timestamp::now(&self.engine.parts.clock)?;
                self.engine
                    .parts
                    .store
                    .append_activity(self.task, at, &event)
                    .await
                    .map_err(|source| RunError::Activity {
                        task: self.task.clone(),
                        source,
                    })?;
            }
        }
        Ok(())
    }
}
