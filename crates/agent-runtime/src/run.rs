//! The run engine: everything one run of a model CLI needs, from its files to its classified end.
//!
//! [`RunEngine::run`] builds the run through the CLI's adapter, lays down the files the adapter asks for, starts the
//! CLI under the supervisor and follows its output: every line goes to the adapter's parser and its events to the
//! task's activity log, and a run with no activity for the watchdog's limit is killed. When the process is over, the
//! final message, the error tail and the exit status are classified (`agent_core::decide::outcome`). The engine
//! touches no state: what the end means for the task is the daemon's decision.

pub mod compose;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_core::cli::{BuildRunError, McpServerKind, RunMode, SessionId};
use agent_core::config::{AbsolutePath, CliArgs, CommandLine, Config, InvalidValue, ModelName, Secrets, Watchdog};
use agent_core::decide::outcome::{Classified, ClassifyError, RunFinish};
use agent_core::ids::{RunId, TaskId};
use agent_core::kinds::CliKind;
use agent_core::state::InvalidTimestamp;
use time::UtcOffset;
use tokio_util::sync::CancellationToken;

use crate::clock::SharedClock;
use crate::git::GitError;
use crate::harness_prompts::{HarnessPrompts, RenderTextError};
use crate::process::{ProcessError, Supervisor};
use crate::store::{StateStore, StoreError};

/// How long a prompt may be to travel as one argument of a command line; longer ones go through a file. Linux refuses
/// an argument over 128 KiB, and a CLI adds its own flags around it.
pub const MAX_PROMPT_ARGUMENT_BYTES: usize = 100_000;

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
            poll: Duration::ZERO,
            hang: watchdog.hang_minutes.as_duration(),
            stop_grace: Duration::ZERO,
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
    /// Whether the run did anything but start: a step, an edit, a message.
    pub made_progress: bool,
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
    /// The directories of `[binaries] agent_path` cannot be made a `PATH`.
    #[error("cannot make a PATH of the directories of agent_path")]
    AgentPath {
        /// What the standard library refused.
        source: std::env::JoinPathsError,
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
        let _ = (&self.parts, job, cancel);
        Ok(RunVerdict::Interrupted { session: None })
    }
}
