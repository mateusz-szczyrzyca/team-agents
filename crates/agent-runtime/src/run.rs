//! The run engine: everything one run of a model CLI needs, from its files to its classified end.
//!
//! [`RunEngine::run`] builds the run through the CLI's adapter, lays down the files the adapter asks for, starts the
//! CLI under the supervisor and follows its output: every line goes to the adapter's parser and its events to the
//! task's activity log, and a run with no activity for the watchdog's limit is killed. When the process is over, the
//! final message, the error tail and the exit status are classified (`agent_core::decide::outcome`). The engine
//! touches no state: what the end means for the task is the daemon's decision.

pub mod activity;
pub mod compose;
mod hold;
mod launch;
pub mod outcome;
pub mod serena;
pub use launch::check_environment;
mod output;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use agent_core::cli::{
    ActivityEvent, BuildRunError, McpServerKind, OutputError, OutputParser, RunMode, RunOutput, RunUsage, SessionId,
};
use agent_core::config::{AbsolutePath, CliArgs, CommandLine, Config, InvalidValue, ModelName, Secrets, Watchdog};
use agent_core::decide::outcome::{
    Classified, ClassifyError, ExitStatus, LimitContext, RunFinish, StoppedBy, classify_outcome,
};
use agent_core::history::{Launch, Record, RunChanges, StepKind};
use agent_core::ids::{RunId, TaskId};
use agent_core::kinds::CliKind;
use agent_core::prompt::VariablesError;
use agent_core::serena::{RemindAfter, SessionTally};
use agent_core::state::{CommitId, InvalidTimestamp, Timestamp};
use serde_json::json;
use time::UtcOffset;
use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use crate::clock::SharedClock;
use crate::git::{CommitIdentity, GitError, Revision};
use crate::harness_prompts::{HarnessPrompts, RenderTextError};
use crate::history::{CaptureKind, History, RunUsageRecord, log_decision};
use crate::process::{ExitReport, ProcessError, RunningProcess, Supervisor, error_chain};
use crate::run::output::{OutputTail, read_end};
pub use crate::run::serena::DueReminder;
use crate::run::serena::{SessionWatch, Watched as WatchStep};

use crate::store::StateStore;
pub use hold::{HoldGuard, WatchdogHold};
pub use launch::{agent_path_directories, keep_out_of_git};

/// One best-effort history boundary may spend two seconds collecting Git data, plus bounded child cleanup.
const HISTORY_CAPTURE_BUDGET: Duration = Duration::from_secs(2);

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
    /// The directory of the agent/daemon executable, prepended to `PATH` for agent runs.
    pub agent_binary_dir: AbsolutePath,
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
    /// Whose the commits the agent makes in the run are: the agent's, never the human's.
    pub identity: CommitIdentity,
    /// The MCP servers the run gets, each with its whole command line.
    pub mcp_servers: BTreeMap<McpServerKind, CommandLine>,
    /// The model's context window in tokens, the agent's `context`; `None` leaves the CLI what it knows.
    pub context: Option<u64>,
    /// The most tokens the model writes in one answer, the agent's `output`; `None` leaves the CLI its own fallback.
    pub output: Option<u64>,
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
    /// Whether the run recorded a step that calls a tool: a bus call, a tool call, a command or a change of a file.
    /// A `STATUS: FAILED` run that called no tool is started once in a new session before its task fails.
    pub used_tool: bool,
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

/// The callsite for directly constructed model CLI operation spans.
static MODEL_CLI_CALLSITE: tracing::callsite::DefaultCallsite =
    tracing::callsite::DefaultCallsite::new(&MODEL_CLI_METADATA);

/// The stable metadata and scope fields of a model CLI operation span.
static MODEL_CLI_METADATA: tracing::Metadata<'static> = tracing::Metadata::new(
    "model_cli",
    module_path!(),
    tracing::Level::DEBUG,
    Some(file!()),
    Some(line!()),
    Some(module_path!()),
    tracing::field::FieldSet::new(
        &["task", "run", "cli"],
        tracing::callsite::Identifier(&MODEL_CLI_CALLSITE),
    ),
    tracing::metadata::Kind::SPAN,
);

/// Opens a model operation span without consulting the process-wide macro-interest cache.
fn model_cli_span(task: &TaskId, run: &RunId, cli: &str) -> tracing::Span {
    crate::history::ensure_scoped_callsites();
    let task = task.to_string();
    let run = run.to_string();
    let task_text = task.as_str();
    let run_text = run.as_str();
    let fields = MODEL_CLI_METADATA.fields();
    let field_list = fields.iter().collect::<Vec<_>>();
    let [task_field, run_field, cli_field] = field_list.as_slice() else {
        // The static metadata and this destructuring must change together.
        return tracing::Span::none();
    };
    let entries = [
        (task_field, Some(&task_text as &dyn tracing::field::Value)),
        (run_field, Some(&run_text as &dyn tracing::field::Value)),
        (cli_field, Some(&cli as &dyn tracing::field::Value)),
    ];
    tracing::Span::new(&MODEL_CLI_METADATA, &fields.value_set(&entries))
}

/// Classifies a completed stream and records bounded projections, never its raw output or pause evidence.
fn classify_and_log(
    output: &RunOutput,
    finish: RunFinish,
    context: &LimitContext<'_>,
) -> Result<Classified, ClassifyError> {
    let classified = classify_outcome(output, finish, context)?;
    let input = json!({
        "stream": match output.stream {
            agent_core::cli::StreamOutcome::Succeeded => "succeeded",
            agent_core::cli::StreamOutcome::Failed => "failed",
            agent_core::cli::StreamOutcome::Incomplete => "incomplete",
            agent_core::cli::StreamOutcome::TurnLimit => "turn_limit",
        },
        "exit": match finish.exit {
            ExitStatus::Code(code) => json!({"code": code}),
            ExitStatus::Signal => json!("signal"),
        },
        "stopped_by": match finish.stopped_by {
            StoppedBy::Nobody => "nobody",
            StoppedBy::Watchdog => "watchdog",
            StoppedBy::Human => "human",
        },
        "now": context.now,
        "local_offset_seconds": context.local_offset.whole_seconds(),
    });
    let result = json!({
        "end": &classified.end,
        "pause": classified.pause.as_ref().map(|pause| json!({
            "until": &pause.until,
            "reason": pause.reason.to_string(),
        })),
    });
    log_decision("classify_outcome", &input, &result);
    Ok(classified)
}

/// Carries out runs of model CLIs.
#[derive(Debug, Clone)]
pub struct RunEngine {
    /// What the engine is made of.
    parts: EngineParts,
    /// Where the launch, the steps and the changes of every run are recorded; off unless [`RunEngine::with_history`].
    history: History,
    /// The count of the sessions its runs made of code without Serena, by session: a session that goes on in a resumed
    /// run keeps its count, and is reminded no more than once.
    serena_sessions: Arc<Mutex<BTreeMap<String, SessionTally>>>,
    /// The reminders that came due while runs went on and that the daemon has not taken yet, oldest first. A reminder is
    /// put on the bus while its run goes on, so it is handed over at once, in memory, and not read back from the history.
    due_reminders: Arc<Mutex<Vec<DueReminder>>>,
}

impl RunEngine {
    /// An engine made of `parts`.
    #[must_use]
    pub fn new(parts: EngineParts) -> Self {
        Self {
            parts,
            history: History::disabled(),
            serena_sessions: Arc::new(Mutex::new(BTreeMap::new())),
            due_reminders: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// The reminders that came due since the last call, oldest first, each once: the run of each is going on, and the
    /// caller puts the reminder on the bus for it.
    #[must_use]
    pub fn take_due_reminders(&self) -> Vec<DueReminder> {
        // A poisoned lock still holds consistent data: the list is only ever appended to and emptied whole.
        std::mem::take(
            &mut *self
                .due_reminders
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// The same engine, recording what its runs do in `history`: how each process was started, every step, and what the
    /// run changed.
    #[must_use]
    pub fn with_history(mut self, history: History) -> Self {
        self.history = history;
        self
    }

    /// The watch of the session `job` is of: fresh for a new session, and for a resumed one the tally its earlier runs
    /// kept in this engine, or, after a restart of the daemon, the count the history holds for the session. `None` when
    /// the run has no Serena or the reminder is off.
    async fn serena_watch(&self, job: &RunJob) -> Option<SessionWatch> {
        let after = remind_after(job, &self.parts.config)?;
        if !matches!(job.mode, RunMode::Resume { .. }) {
            return SessionWatch::new(Some(after));
        }
        let key = session_key(&job.mode);
        if let Some(tally) = self.serena_tally(&key) {
            return Some(SessionWatch::resumed(tally));
        }
        if self.history.is_disabled() {
            return SessionWatch::new(Some(after));
        }
        let path = self.parts.store.home().history_file();
        let task = job.task.clone();
        let session = key.clone();
        let read = tokio::task::spawn_blocking(move || SessionWatch::from_history(&path, &task, &session)).await;
        match read {
            Ok(Ok(watch)) => Some(watch),
            // A session whose count cannot be read counts anew: it may be reminded once more, which is told.
            Ok(Err(error)) => {
                tracing::warn!(
                    event = agent_core::log::LogEvent::HistoryRecordIncomplete.as_str(),
                    task = %job.task,
                    run = %job.run,
                    error = %error_chain(&error),
                    "the count of the resumed session cannot be read from the history: it counts anew"
                );
                SessionWatch::new(Some(after))
            }
            Err(error) => {
                tracing::warn!(
                    event = agent_core::log::LogEvent::HistoryRecordIncomplete.as_str(),
                    task = %job.task,
                    run = %job.run,
                    error = %error_chain(&error),
                    "the read of the count of the resumed session failed: it counts anew"
                );
                SessionWatch::new(Some(after))
            }
        }
    }

    /// The tally of the session `key` the engine has kept so far, if any.
    fn serena_tally(&self, key: &str) -> Option<SessionTally> {
        // A poisoned lock still holds consistent data: the map is only ever replaced whole.
        self.serena_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .copied()
    }

    /// Keeps `tally` as the count of the session `key`.
    fn keep_tally(&self, key: &str, tally: SessionTally) {
        // A poisoned lock still holds consistent data: the map is only ever replaced whole.
        self.serena_sessions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key.to_owned(), tally);
    }

    /// Carries out `job` and returns how it ended.
    ///
    /// Cancelling `cancel` stops the process (its whole group) and returns [`RunVerdict::Interrupted`].
    ///
    /// # Errors
    /// [`RunError`]: the run could not be built, started, followed or classified. A process that was started is
    /// stopped when the error is returned.
    pub async fn run(&self, job: RunJob, cancel: &CancellationToken) -> Result<RunVerdict, RunError> {
        self.run_held(job, cancel, &WatchdogHold::new()).await
    }

    /// Carries out `job` like [`RunEngine::run`], with the watchdog held off while `hold` is held: the run is never
    /// taken for hung then, and its silence is counted from the moment the hold ended.
    ///
    /// # Errors
    /// [`RunError`], as for [`RunEngine::run`].
    pub async fn run_held(
        &self,
        job: RunJob,
        cancel: &CancellationToken,
        hold: &WatchdogHold,
    ) -> Result<RunVerdict, RunError> {
        let started = std::time::Instant::now();
        let tool = job.cli.as_str();
        let span = model_cli_span(&job.task, &job.run, tool);
        let operation = async {
            let result = self.run_held_lifecycle(job, cancel, hold).await;
            let (exit_code, outcome) = match &result {
                Ok(RunVerdict::Finished(run)) => {
                    let code = match run.finish.exit {
                        ExitStatus::Code(code) => Some(code),
                        ExitStatus::Signal => None,
                    };
                    let outcome = if code == Some(0) { "ok" } else { "failed" };
                    (code, outcome)
                }
                Ok(RunVerdict::Interrupted { .. }) => (None, "cancelled"),
                Err(_) => (None, "error"),
            };
            crate::history::log_outside_call(tool, "run", started.elapsed(), exit_code, outcome);
            result
        };
        tracing::Instrument::instrument(operation, span).await
    }

    /// Carries out the process lifecycle whose completion is logged by [`Self::run_held`].
    async fn run_held_lifecycle(
        &self,
        job: RunJob,
        cancel: &CancellationToken,
        hold: &WatchdogHold,
    ) -> Result<RunVerdict, RunError> {
        let launch = launch::prepare(&self.parts, &job).await?;
        let stdout = launch.process.stdout_file.clone();
        let stderr = launch.process.stderr_file.clone();
        let head_before = self.head_of_worktree(&job).await;
        let process = self
            .parts
            .supervisor
            .spawn(&launch.process)
            .await
            .map_err(|source| RunError::Spawn {
                task: job.task.clone(),
                cli: job.cli,
                source,
            })?;
        self.record_launch(&job, &launch, head_before.clone());
        let mut follow = Follow {
            engine: self,
            task: &job.task,
            run: &job.run,
            tail: OutputTail::new(stdout),
            parser: launch.parser,
            known_session: match &job.mode {
                RunMode::Resume { session_id } => Some(session_id.clone()),
                RunMode::New { session_id } if job.cli == CliKind::Claude => Some(session_id.clone()),
                RunMode::New { .. } => None,
            },
            watch: self.serena_watch(&job).await,
            session_key: session_key(&job.mode),
            after: remind_after(&job, &self.parts.config),
            cli: job.cli,
            steps: 0,
            progress: Progress::Nothing,
            used_tool: false,
            unreadable: Unreadable::default(),
        };
        let stopped_by = self.supervise(&job, process, &mut follow, cancel, hold).await?;
        follow.drain().await?;
        self.record_process_end(&job, head_before, follow.parser.usage()).await;
        let stderr = read_end(&stderr).await?;
        follow.unreadable.report(&job);
        let usage = follow.parser.usage();
        let output = follow.parser.finish(&stderr, self.parts.config.limits().tail_lines);
        self.record_outcome(&job, usage, output.final_message.clone()).await;
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
        let classified = classify_and_log(&output, finish, &context).map_err(|source| RunError::Classify {
            task: job.task.clone(),
            source,
        })?;
        Ok(RunVerdict::Finished(Box::new(FinishedRun {
            classified,
            finish,
            session: output.session_id,
            final_message: output.final_message,
            progress: follow.progress,
            used_tool: follow.used_tool,
        })))
    }

    /// Waits for the run of `job` to end — it goes silent, is cancelled, or exits — feeding its output to `follow`,
    /// and returns how it ended. The rest of the leader's process group is stopped once the run is over, consuming
    /// the process.
    async fn supervise(
        &self,
        job: &RunJob,
        mut process: RunningProcess,
        follow: &mut Follow<'_>,
        cancel: &CancellationToken,
        hold: &WatchdogHold,
    ) -> Result<Ended, RunError> {
        let supervise = |source| RunError::Supervise {
            task: job.task.clone(),
            source,
        };
        let mut tick = tokio::time::interval(self.parts.timings.poll.max(MIN_POLL));
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            let event = tokio::select! {
                exit = process.wait() => Watched::Exited(exit),
                () = cancel.cancelled() => Watched::Cancelled,
                _ = tick.tick() => Watched::Tick,
            };
            match event {
                Watched::Exited(exit) => {
                    let exit = exit.map_err(supervise)?;
                    process.stop(self.parts.timings.stop_grace).await.map_err(supervise)?;
                    return Ok(Ended::By(StoppedBy::Nobody, exit));
                }
                Watched::Cancelled => {
                    process.stop(self.parts.timings.stop_grace).await.map_err(supervise)?;
                    return Ok(Ended::Cancelled);
                }
                Watched::Tick => {
                    follow.read_new().await?;
                    let last = process.last_activity().await.map_err(supervise)?;
                    // A run that waits for something the daemon does for it is not silent: its limit counts from
                    // the later of its last output and the last time it was held.
                    let last = hold.last_held().map_or(last, |held| last.max(held));
                    if self.is_silent(last) {
                        let report = process.stop(self.parts.timings.stop_grace).await.map_err(supervise)?;
                        return Ok(Ended::By(StoppedBy::Watchdog, report.exit));
                    }
                }
            }
        }
    }

    /// The commit the worktree of `job` is at, for the history; `None` when it is off or git cannot say.
    async fn head_of_worktree(&self, job: &RunJob) -> Option<CommitId> {
        if !self.history.is_writing() {
            return None;
        }
        match launch::lookup_git(&job.working_dir)
            .with_deadline(tokio::time::Instant::now() + HISTORY_CAPTURE_BUDGET)
            .rev_parse(&Revision::Head)
            .await
        {
            Ok(head) => Some(head),
            Err(error) => {
                self.history
                    .capture_run_failed(&job.run, CaptureKind::Head, &self.parts.clock, &error);
                None
            }
        }
    }

    /// Records how the process of `job` was started: the command line the CLI saw and the names — never the values — of the
    /// environment it was given.
    fn record_launch(&self, job: &RunJob, launch: &launch::Launch, head: Option<CommitId>) {
        if self.history.is_disabled() {
            return;
        }
        let process = &launch.process;
        let at = match Timestamp::now(&self.parts.clock) {
            Ok(at) => at,
            Err(error) => {
                tracing::warn!(
                    event = agent_core::log::LogEvent::HistoryRecordIncomplete.as_str(),
                    task = %job.task,
                    run = %job.run,
                    error = %error_chain(&error),
                    "the launch of a run cannot be stamped: the history has none for it"
                );
                self.history.record_lost();
                return;
            }
        };
        let Some(program) = process.program.as_path().to_str() else {
            tracing::warn!(
                event = agent_core::log::LogEvent::HistoryRecordIncomplete.as_str(),
                task = %job.task,
                run = %job.run,
                "the program of a run is not text: the history has no launch for it"
            );
            self.history.record_lost();
            return;
        };
        self.history.record(Record::Launch(Box::new(Launch {
            run: job.run.clone(),
            at,
            program: program.to_owned(),
            args: process.args.clone(),
            working_dir: process.working_dir.clone(),
            head,
            env_names: process.env.keys().cloned().collect(),
            prompt_in_file: launch.prompt_in_file,
        })));
    }

    /// What a run leaves in the history once its process is gone, also for a run that was cancelled: what it changed,
    /// and what its stream told of its usage.
    async fn record_process_end(&self, job: &RunJob, head_before: Option<CommitId>, usage: RunUsage) {
        self.record_changes(job, head_before).await;
        self.record_usage(job, usage);
    }

    /// Writes the run's outcome to its own directory. Best-effort: a refused write is logged with its cause, and the owner
    /// then reads the size of the session from the history, as it did before the file existed.
    async fn record_outcome(&self, job: &RunJob, usage: RunUsage, final_message: Option<String>) {
        let dir = self.parts.store.home().run_dir(&job.task, &job.run);
        let outcome = outcome::RunOutcome {
            last_context: usage.last_context,
            final_message,
        };
        if let Err(error) = outcome::write_outcome(dir.as_path(), &outcome).await {
            tracing::warn!(
                event = agent_core::log::LogEvent::HistoryRecordIncomplete.as_str(),
                task = %job.task,
                run = %job.run,
                error = %error_chain(&error),
                "the outcome of a run was not written: the size of its session comes from the history"
            );
        }
    }

    /// Writes what the run's stream told of its usage to the history. Best-effort: a refused write is logged with its cause, and the run goes on.
    fn record_usage(&self, job: &RunJob, usage: RunUsage) {
        // A stream that told nothing of the usage writes no row: a run without a row is "not told" all the same.
        if self.history.is_disabled() || usage == RunUsage::default() {
            return;
        }
        let records = vec![RunUsageRecord {
            run: job.run.clone(),
            usage,
        }];
        if let Err(error) = self.history.persist_usage(records) {
            tracing::warn!(
                event = agent_core::log::LogEvent::HistoryRecordIncomplete.as_str(),
                task = %job.task,
                run = %job.run,
                error = %error_chain(&error),
                "the usage of a run was not written to the history"
            );
        }
    }

    /// Records what the run changed in its worktree, now that its process is gone: the commits and files between the commit
    /// it started at and the one it ended at, and what it left uncommitted. A worktree git cannot read has none.
    async fn record_changes(&self, job: &RunJob, head_before: Option<CommitId>) {
        self.history
            .record_captured(&job.run, CaptureKind::Changes, &self.parts.clock, async {
                let changes = launch::lookup_git(&job.working_dir)
                    .with_deadline(tokio::time::Instant::now() + HISTORY_CAPTURE_BUDGET)
                    .work_changes(head_before.as_ref())
                    .await?;
                Ok::<_, crate::git::GitError>(Record::Changes(Box::new(RunChanges {
                    run: job.run.clone(),
                    head_before,
                    head_after: Some(changes.head_after),
                    commits: changes.commits,
                    files: changes.files,
                    uncommitted: changes.uncommitted,
                })))
            })
            .await;
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
                event = agent_core::log::LogEvent::CliOutputUnreadable.as_str(),
                task = %job.task,
                run = %job.run,
                unreadable_lines = self.count,
                first = %first,
                "lines of the output of a run could not be read",
            );
        }
    }
}

/// The count at which the session of `job` is reminded, when the run has Serena and the reminder is not off.
fn remind_after(job: &RunJob, config: &Config) -> Option<RemindAfter> {
    given_serena(job)
        .then(|| agent_core::config::remind_after(config))
        .flatten()
}

/// Whether the run `job` is given Serena, the MCP server that finds code through language servers.
fn given_serena(job: &RunJob) -> bool {
    job.mcp_servers.contains_key(&McpServerKind::Serena)
}

/// The key of the session a run is of, for the counts the engine keeps across a session's runs.
fn session_key(mode: &RunMode) -> String {
    match mode {
        RunMode::New { session_id } | RunMode::Resume { session_id } => session_id.as_str().to_owned(),
    }
}

/// What the engine does with the output of one run as it grows.
struct Follow<'a> {
    /// The engine, for the store and the clock.
    engine: &'a RunEngine,
    /// The task, included in diagnostics for unreadable output.
    task: &'a TaskId,
    /// The run, which its steps are recorded for.
    run: &'a RunId,
    /// The standard output file.
    tail: OutputTail,
    /// The adapter's parser.
    parser: Box<dyn OutputParser>,
    /// The session supplied at launch or most recently observed; echoes are not new observations.
    known_session: Option<SessionId>,
    /// The count of the session's searches of code without Serena; `None` for a run without Serena and for a run whose
    /// session was reminded before, which counts no step again.
    watch: Option<SessionWatch>,
    /// The key of the session, under which the engine keeps the count that a resumed run goes on from.
    session_key: String,
    /// The count at which the session is reminded, when the run has Serena and the reminder is not off.
    after: Option<RemindAfter>,
    /// The model CLI, which the log of a reminder names.
    cli: CliKind,
    /// The number of the step being fed: the steps of the run before it, then this one.
    steps: u32,
    /// Whether an event that is more than the start or an error was seen.
    progress: Progress,
    /// Whether a step that calls a tool was recorded in this run.
    used_tool: bool,
    /// What the parser refused.
    unreadable: Unreadable,
}

impl Follow<'_> {
    /// Gives the step `event` to the count of the session, if the run counts one: the searches of the session when the
    /// reminder came due, or `None`.
    fn note_reminder(&mut self, event: &ActivityEvent) -> Option<u64> {
        let (Some(watch), Some(after)) = (self.watch.as_mut(), self.after) else {
            return None;
        };
        let watched = watch.note(event, after);
        if let Some(tally) = watch.tally() {
            self.engine.keep_tally(&self.session_key, tally);
        }
        let WatchStep::Due { searches } = watched else {
            return None;
        };
        Some(searches)
    }

    /// Logs the reminder of the session at the step `number`, after `searches` searches of code without Serena, and
    /// records it in the history of the run's task.
    fn log_reminder(&self, number: u32, searches: u64) {
        tracing::info!(
            event = agent_core::log::LogEvent::SerenaReminded.as_str(),
            task = %self.task,
            run = %self.run,
            step = number,
            searches,
            cli = self.cli.as_str(),
            "the run was told once to find code with Serena"
        );
        self.engine.history.serena_reminded(
            self.run,
            self.task,
            self.cli,
            searches,
            number,
            &self.engine.parts.clock,
        );
        self.engine
            .due_reminders
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(DueReminder {
                task: self.task.clone(),
                run: self.run.clone(),
                cli: self.cli,
                searches,
            });
    }

    /// Feeds the lines written since the last call to the parser and its events to the history.
    async fn read_new(&mut self) -> Result<(), RunError> {
        let lines = self.tail.read_new().await?;
        self.feed(&lines)
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
            self.feed(&[last])?;
        }
        if self.tail.dropped() > 0 {
            tracing::warn!(
                event = agent_core::log::LogEvent::CliOutputUnreadable.as_str(),
                task = %self.task,
                run = %self.run,
                dropped = self.tail.dropped(),
                "lines of the output were too long or not text"
            );
        }
        Ok(())
    }

    /// Gives `lines` to the parser and logs the events.
    fn feed(&mut self, lines: &[String]) -> Result<(), RunError> {
        for line in lines {
            let events = match self.parser.push_line(line) {
                Ok(events) => events,
                Err(error) => {
                    self.unreadable.note(&error);
                    continue;
                }
            };
            if let Some(session) = self
                .parser
                .session()
                .filter(|session| Some(*session) != self.known_session.as_ref())
            {
                self.engine.history.session(self.run, session, &self.engine.parts.clock);
                self.known_session = Some(session.clone());
            }
            for event in events {
                if !matches!(event, ActivityEvent::Info(_) | ActivityEvent::Error(_)) {
                    self.progress = Progress::Made;
                }
                if matches!(
                    StepKind::of(&event),
                    StepKind::BusCall | StepKind::ToolCall | StepKind::Command | StepKind::FileEdit
                ) {
                    self.used_tool = true;
                }
                let at = Timestamp::now(&self.engine.parts.clock)?;
                self.steps = self.steps.saturating_add(1);
                let due = self.note_reminder(&event);
                let step_number = self.steps;
                self.engine.history.step(self.run, at, event);
                if let Some(searches) = due {
                    self.log_reminder(step_number, searches);
                }
            }
        }
        Ok(())
    }
}
