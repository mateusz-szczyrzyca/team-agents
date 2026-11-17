//! The daemon: the single owner of the state and of every model run.
//!
//! One daemon runs per home (the lock of [`StateStore`]); every other command and the dashboard are clients of its
//! socket (`docs/architecture.md`, "Process model"). The daemon is a small group of tasks that talk through channels:
//!
//! - the **owner** holds the state and is the only one that changes it: it answers requests, starts a run for every
//!   queued task, applies the end of each run, wakes paused tasks and publishes events;
//! - the **server** accepts connections on the socket and passes their requests to the owner;
//! - the **notifier** delivers the notifications the owner asks for;
//! - the **gate service** runs the repository's gate for the tasks that ask, one at a time on the machine.
//!
//! Cancelling the shutdown token stops them in order: the server stops accepting, the owner stops its runs and records
//! them as interrupted, the notifier delivers what is queued, and the socket is removed.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use agent_core::config::{AbsolutePath, OpenQuestions};
use agent_core::log::LogEvent;
use agent_runtime::agentbus::AgentBus;
use agent_runtime::autopilot_prompts::{AutopilotPrompts, AutopilotPromptsError};
use agent_runtime::checks_prompts::{ChecksPrompts, ChecksPromptsError};
use agent_runtime::clock::SharedClock;
use agent_runtime::editor::{SocketDirs, VscodeOpener};
use agent_runtime::harness_prompts::{HarnessPrompts, HarnessPromptsError};
use agent_runtime::history::{HistoryLayer, Scoped};
use agent_runtime::notify::Notifier;
use agent_runtime::process::Supervisor;
use agent_runtime::process::error_chain;
use agent_runtime::prompts::{PromptLoadError, PromptLoader};
use agent_runtime::questions_prompts::{QuestionsPrompts, QuestionsPromptsError};
use agent_runtime::run::{EngineParts, RunEngine, RunTimings};
use agent_runtime::settings::{Settings, SettingsError};
use agent_runtime::store::{Home, StateStore, StoreError};
use agent_runtime::team_prompts::{TeamPrompts, TeamPromptsError};
use rand::SeedableRng;
use rand::rngs::{StdRng, SysError, SysRng};
use time::UtcOffset;
use tokio::sync::{broadcast, mpsc};
use tokio::task::{JoinError, JoinSet};
use tokio_util::sync::CancellationToken;
use tracing::instrument::WithSubscriber;
use tracing_subscriber::layer::SubscriberExt;

use self::owner::{EVENT_BACKLOG, INBOX, MAX_RUNS, Owner, OwnerError, OwnerParts, Pulls, PullsError};
use self::server::ServerError;

mod checks;
mod control;
mod gate;
mod history;
mod merge;
mod notice;
mod notifier;
mod opener;
mod owner;
mod pr;
mod questions;
mod recovery;
mod review;
#[cfg(test)]
#[path = "tests/round_fixture.rs"]
mod round_fixture;
mod server;
mod settle;
mod team;

#[cfg(test)]
mod tests;

/// How often the daemon looks for paused tasks that are due, in production.
pub(crate) const TICK: Duration = Duration::from_secs(5);

/// How often and how long the daemon waits, apart from what the configuration says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Timings {
    /// The time between two looks for paused tasks that are due.
    pub(crate) tick: Duration,
    /// How the engine watches its runs.
    pub(crate) run: RunTimings,
}

/// What a daemon needs to start.
#[derive(Debug, Clone)]
pub(crate) struct DaemonSetup {
    /// The home.
    pub(crate) home: Home,
    /// The directory of the prompt files.
    pub(crate) prompts_dir: PathBuf,
    /// The clock that stamps everything.
    pub(crate) clock: SharedClock,
    /// The machine's offset from UTC.
    pub(crate) local_offset: UtcOffset,
    /// The timings, or `None` for those the configuration gives.
    pub(crate) timings: Option<Timings>,
    /// The directory containing the daemon's own executable, or `None` to resolve `std::env::current_exe()`.
    pub(crate) agent_binary_dir: Option<AbsolutePath>,
}

/// Why the daemon stopped with an error.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DaemonError {
    /// Another daemon owns the home.
    #[error("the agent daemon is already running: {path:?} is locked by it; use another AGENTS_HOME for a second one")]
    AlreadyRunning {
        /// The lock file.
        path: PathBuf,
    },
    /// The home cannot be opened or its state read or saved.
    #[error("the state store of the daemon failed")]
    Store(#[from] StoreError),
    /// The settings cannot be read.
    #[error("cannot read the settings of the home")]
    Settings(#[from] SettingsError),
    /// The prompts of the harness cannot be loaded.
    #[error("cannot load the texts of the harness")]
    HarnessPrompts(#[from] HarnessPromptsError),
    /// The texts of the teams cannot be loaded.
    #[error("cannot load the texts of the teams")]
    TeamPrompts(#[from] TeamPromptsError),
    /// The texts of the questions cannot be loaded.
    #[error("cannot load the texts of the questions")]
    QuestionsPrompts(#[from] QuestionsPromptsError),
    /// The texts of the pull requests and the reviews cannot be loaded.
    #[error("cannot load the texts of the pull requests and the reviews")]
    AutopilotPrompts(#[from] AutopilotPromptsError),
    /// A text of the checks before a review cannot be used.
    #[error("the texts of the checks before a review cannot be used")]
    ChecksPrompts(#[from] ChecksPromptsError),
    /// The autopilot cannot be set up from the settings.
    #[error("cannot set up the pull requests of the autopilot")]
    Pulls(#[from] PullsError),
    /// The rules cannot be loaded.
    #[error("cannot load the rules")]
    Prompts(#[from] PromptLoadError),
    /// The operating system gives no entropy for the IDs.
    #[error("cannot seed the generator of IDs from the operating system")]
    Entropy(#[source] SysError),
    /// The socket cannot be made.
    #[error("cannot make the socket of the daemon")]
    Socket(#[from] ServerError),
    /// The socket cannot be removed.
    #[error("cannot remove the socket of the daemon")]
    RemoveSocket(#[source] std::io::Error),
    /// The owner of the state stopped with an error.
    #[error("the owner of the state stopped")]
    Owner(#[from] OwnerError),
    /// The socket server or the notifier failed.
    #[error("a task of the daemon (the socket server or the notifier) failed")]
    Join(#[source] JoinError),
    /// The daemon's executable cannot be resolved.
    #[error("cannot resolve the daemon executable")]
    Executable(#[from] ExecutableError),
}

/// Why the executable directory of the daemon cannot be determined.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ExecutableError {
    /// The current executable cannot be determined.
    #[error("cannot determine the executable of the daemon")]
    Current(#[source] std::io::Error),
    /// The executable path cannot be canonicalized.
    #[error("cannot canonicalize the daemon executable path {path:?}")]
    Canonicalize {
        /// The executable path returned by `current_exe` before canonicalization.
        path: PathBuf,
        /// The I/O error encountered while canonicalizing.
        #[source]
        source: std::io::Error,
    },
    /// The executable path has no parent directory.
    #[error("the daemon executable path {path:?} has no parent directory")]
    NoParent {
        /// The canonicalized executable path without a parent directory.
        path: PathBuf,
    },
    /// The directory path is not a valid absolute path.
    #[error("the daemon executable directory {path:?} is not valid")]
    InvalidPath {
        /// The parent directory path of the canonical executable.
        path: PathBuf,
        /// The rule violation if the path is not valid UTF-8 or not an absolute path.
        source: Option<agent_core::config::InvalidValue>,
    },
}

/// Resolves the canonical directory of the current executable.
///
/// # Errors
/// [`ExecutableError`] if the executable cannot be found, canonicalized, or its parent is not a valid absolute path.
pub(crate) fn resolve_agent_binary_dir() -> Result<AbsolutePath, ExecutableError> {
    resolve_agent_binary_dir_from(std::env::current_exe)
}

/// Resolves the canonical directory of the executable returned by `current_exe`.
pub(crate) fn resolve_agent_binary_dir_from(
    current_exe: impl FnOnce() -> std::io::Result<PathBuf>,
) -> Result<AbsolutePath, ExecutableError> {
    let exe = current_exe().map_err(ExecutableError::Current)?;
    let canonical = exe.canonicalize().map_err(|source| ExecutableError::Canonicalize {
        path: exe.clone(),
        source,
    })?;
    let parent = canonical.parent().ok_or_else(|| ExecutableError::NoParent {
        path: canonical.clone(),
    })?;
    let text = parent.to_str().ok_or_else(|| ExecutableError::InvalidPath {
        path: parent.to_path_buf(),
        source: None,
    })?;
    AbsolutePath::from_str(text).map_err(|source| ExecutableError::InvalidPath {
        path: parent.to_path_buf(),
        source: Some(source),
    })
}

/// Where the daemon looks for what belongs to the user it runs as, read from the environment once at its start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Surroundings {
    /// The home directory of the user, where the VS Code servers keep their directories; `None` when it is not known.
    pub(crate) user_home: Option<PathBuf>,
    /// The runtime directory of the user's session, where VS Code puts the sockets of its windows.
    pub(crate) runtime_dir: PathBuf,
    /// Another directory that may hold sockets of windows.
    pub(crate) fallback_dir: PathBuf,
}

impl Surroundings {
    /// The surroundings of this process: `HOME`, and `XDG_RUNTIME_DIR` or else `/run/user/<uid>`, then `/tmp`.
    pub(crate) fn from_process() -> Self {
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(
            || PathBuf::from(format!("/run/user/{}", rustix::process::getuid().as_raw())),
            PathBuf::from,
        );
        Self {
            user_home: std::env::var_os("HOME").map(PathBuf::from),
            runtime_dir,
            fallback_dir: PathBuf::from("/tmp"),
        }
    }
}

/// Runs the daemon until `shutdown` is cancelled, in the surroundings of this process.
///
/// # Errors
/// [`DaemonError`].
#[cfg(test)]
pub(crate) async fn run(setup: DaemonSetup, shutdown: CancellationToken) -> Result<(), DaemonError> {
    run_in(setup, Surroundings::from_process(), shutdown).await
}

/// Runs the daemon with the explicitly installed process logging layer.
pub(crate) async fn run_with_log_layer(
    setup: DaemonSetup,
    shutdown: CancellationToken,
    log_layer: HistoryLayer,
) -> Result<(), DaemonError> {
    let rng = StdRng::try_from_rng(&mut SysRng).map_err(DaemonError::Entropy)?;
    run_with_rng_and_log_layer(setup, Surroundings::from_process(), rng, shutdown, Some(log_layer)).await
}

/// Runs the daemon until `shutdown` is cancelled, in `surroundings`.
///
/// # Errors
/// [`DaemonError`].
#[cfg(test)]
pub(crate) async fn run_in(
    setup: DaemonSetup,
    surroundings: Surroundings,
    shutdown: CancellationToken,
) -> Result<(), DaemonError> {
    let rng = StdRng::try_from_rng(&mut SysRng).map_err(DaemonError::Entropy)?;
    run_with_rng_and_log_layer(setup, surroundings, rng, shutdown, None).await
}

/// Runs the daemon with the caller's random source for team selection and identifiers.
///
/// # Errors
/// [`DaemonError`].
#[cfg(test)]
pub(crate) async fn run_with_rng(
    setup: DaemonSetup,
    surroundings: Surroundings,
    rng: StdRng,
    shutdown: CancellationToken,
) -> Result<(), DaemonError> {
    run_with_rng_and_log_layer(setup, surroundings, rng, shutdown, None).await
}

/// The prompt catalogs, rules, and pull requests of the daemon.
struct DaemonTexts {
    /// The harness prompts for run instructions and lifecycle.
    texts: Arc<HarnessPrompts>,
    /// The team prompts for lead and member coordination.
    team_texts: Arc<TeamPrompts>,
    /// The question prompts for human interactions.
    question_texts: Arc<QuestionsPrompts>,
    /// The pull request review and autopilot prompts, if enabled.
    pulls: Option<Pulls>,
    /// The prompt loader for developer and reviewer rules.
    rules: Arc<PromptLoader>,
}

/// Loads the prompt catalogs, rules, and pull request reviewer prompts for the daemon.
///
/// # Errors
/// Returns [`DaemonError::Prompts`] if loading prompt catalogs or rules fails.
/// Returns [`DaemonError::Pulls`] if initializing pull request prompts fails.
async fn load_daemon_texts(setup: &DaemonSetup, settings: &Settings) -> Result<DaemonTexts, DaemonError> {
    let language = settings.config.language().clone();
    let texts = Arc::new(HarnessPrompts::load(&setup.prompts_dir, &language).await?);
    let team_texts = Arc::new(TeamPrompts::load(&setup.prompts_dir, &language).await?);
    let question_texts = Arc::new(QuestionsPrompts::load(&setup.prompts_dir, &language).await?);
    let autopilot_texts = Arc::new(AutopilotPrompts::load(&setup.prompts_dir, &language).await?);
    let checks_texts = Arc::new(ChecksPrompts::load(&setup.prompts_dir, &language).await?);
    let pulls = Pulls::new(autopilot_texts, checks_texts, settings)?;
    let rules = Arc::new(PromptLoader::load(&setup.prompts_dir, &language).await?);
    Ok(DaemonTexts {
        texts,
        team_texts,
        question_texts,
        pulls,
        rules,
    })
}

/// Runs the daemon with either the process-wide logging layer supplied by the command or a scoped test layer.
async fn run_with_rng_and_log_layer(
    setup: DaemonSetup,
    surroundings: Surroundings,
    rng: StdRng,
    shutdown: CancellationToken,
    installed_log_layer: Option<HistoryLayer>,
) -> Result<(), DaemonError> {
    let store = match StateStore::open(setup.home.clone()).await {
        Ok(store) => Arc::new(store),
        Err(StoreError::HomeLocked { path }) => return Err(DaemonError::AlreadyRunning { path }),
        Err(error) => return Err(error.into()),
    };
    let settings = Arc::new(Settings::load(&setup.home).await?);
    let log_layer = daemon_log_layer(&settings, &setup, installed_log_layer);
    let dispatch: tracing::Dispatch = tracing_subscriber::registry().with(log_layer.clone()).into();
    let daemon_texts = load_daemon_texts(&setup, &settings).await?;
    let bus = make_bus(&setup.home, &settings);
    let state = store.load().await?;
    let timings = setup.timings.unwrap_or_else(|| Timings {
        tick: TICK,
        run: RunTimings::from_config(settings.config.watchdog()),
    });
    let opened = history::open(&settings, &setup.home)
        .with_subscriber(dispatch.clone())
        .await;
    log_layer.attach_history(opened.history.clone());
    let agent_binary_dir = match setup.agent_binary_dir {
        Some(dir) => dir,
        None => resolve_agent_binary_dir()?,
    };
    let engine = RunEngine::new(EngineParts {
        store: Arc::clone(&store),
        texts: Arc::clone(&daemon_texts.texts),
        config: Arc::new(settings.config.clone()),
        secrets: Arc::new(settings.secrets.clone()),
        supervisor: Supervisor::new(MAX_RUNS),
        clock: setup.clock.clone(),
        local_offset: setup.local_offset,
        timings: timings.run,
        agent_binary_dir: agent_binary_dir.clone(),
    })
    .with_history(opened.history.clone());
    let notifier = Notifier::from_config(settings.config.notifications(), store.home(), &settings.secrets);
    let listener = server::bind(&setup.home).await?;

    let (inbox, requests) = mpsc::channel(INBOX);
    let (notices, deliveries) = mpsc::channel(notifier::QUEUE);
    let (events, _unused) = broadcast::channel(EVENT_BACKLOG);
    let (gates, gate_jobs) = gate::GateQueue::new();
    let mut sides: JoinSet<()> = JoinSet::new();
    let serving = shutdown.clone();
    // The side tasks are scoped, not only polled with the subscriber: one that is still going when the daemon ends is
    // dropped with it, and its spans close in the registry that made them.
    sides.spawn(Scoped::new(server::serve(listener, inbox, serving), dispatch.clone()));
    sides.spawn(Scoped::new(
        notifier::run(deliveries, notifier, opened.history.clone()),
        dispatch.clone(),
    ));
    let opens = start_opener(&mut sides, &settings, &surroundings, &dispatch);
    let gate_parts = gate::GateParts {
        store: Arc::clone(&store),
        clock: setup.clock.clone(),
        events: events.clone(),
        agent_binary_dir,
        agent_path: settings.config.binaries().agent_path.clone(),
        stop_grace: timings.run.stop_grace,
    };
    sides.spawn(Scoped::new(
        gate::run_recorded(gate_jobs, gate_parts, opened.history.clone(), shutdown.clone()),
        dispatch.clone(),
    ));

    let owner = Owner::new(
        OwnerParts {
            store,
            state,
            settings,
            texts: daemon_texts.texts,
            team_texts: daemon_texts.team_texts,
            question_texts: daemon_texts.question_texts,
            rules: daemon_texts.rules,
            bus,
            opens,
            engine,
            clock: setup.clock.clone(),
            rng,
            notices,
            events,
            local_offset: setup.local_offset,
            tick: timings.tick,
        },
        shutdown.clone(),
    )
    .with_gate(gates)
    .with_history(opened.history.clone());
    let owner = attach_pulls(owner, daemon_texts.pulls, &dispatch);
    log_start(&setup.home, &dispatch);
    let owned_end = Scoped::new(owner.run(requests), dispatch.clone()).await;
    // Whatever ended the owner, the rest of the daemon ends with it.
    shutdown.cancel();
    let failure = join_sides(&mut sides, &dispatch).await;
    log_stop(owned_end.as_ref().err(), failure.as_ref(), &dispatch);
    // Everything that records has ended.
    log_layer.detach_history();
    history::close(opened.writer).await;
    server::remove(&setup.home).await.map_err(DaemonError::RemoveSocket)?;
    owned_end?;
    failure.map_or(Ok(()), Err)
}

/// Configures the installed process layer, or creates the isolated layer used by daemon tests.
fn daemon_log_layer(settings: &Settings, setup: &DaemonSetup, installed: Option<HistoryLayer>) -> HistoryLayer {
    let layer = installed.unwrap_or_else(|| {
        HistoryLayer::deferred(
            settings.config.log().level,
            setup.clock.clone(),
            settings.secrets.clone(),
        )
    });
    layer.configure(
        settings.config.log().level,
        setup.clock.clone(),
        settings.secrets.clone(),
    );
    layer
}

/// Attaches `pulls` to `owner`, or logs under `dispatch` that the home has no GitHub token.
fn attach_pulls(owner: Owner, pulls: Option<Pulls>, dispatch: &tracing::Dispatch) -> Owner {
    if let Some(pulls) = pulls {
        owner.with_pulls(pulls)
    } else {
        tracing::dispatcher::with_default(dispatch, || {
            tracing::warn!(
                event = agent_core::log::LogEvent::GithubCallFailed.as_str(),
                "the home has no GitHub token (github_token in secrets.toml): no pull request is opened"
            );
        });
        owner
    }
}

/// Waits for every background task in `sides` to finish and returns the first join error, if any.
async fn join_sides(sides: &mut JoinSet<()>, dispatch: &tracing::Dispatch) -> Option<DaemonError> {
    let mut failure = None;
    while let Some(joined) = sides.join_next().await {
        if let Err(source) = joined {
            tracing::dispatcher::with_default(dispatch, || {
                tracing::error!(
                    event = LogEvent::DaemonTaskFailed.as_str(),
                    error = %error_chain(&source),
                    "a task of the daemon failed"
                );
            });
            failure.get_or_insert(DaemonError::Join(source));
        }
    }
    failure
}

/// Logs under `dispatch` that the daemon of `home` starts: its state, its history and its socket are open.
fn log_start(home: &Home, dispatch: &tracing::Dispatch) {
    tracing::dispatcher::with_default(dispatch, || {
        tracing::info!(
            event = LogEvent::DaemonStarted.as_str(),
            home = %home.root().display(),
            "the daemon starts"
        );
    });
}

/// Logs under `dispatch` how the daemon stops: the error that ended its owner, if one did, and then `daemon_stopped` with
/// its outcome, the last record of the history. The errors are returned all the same, for the exit status.
fn log_stop(owner: Option<&OwnerError>, side: Option<&DaemonError>, dispatch: &tracing::Dispatch) {
    tracing::dispatcher::with_default(dispatch, || {
        if let Some(error) = owner {
            tracing::error!(
                event = LogEvent::DaemonTaskFailed.as_str(),
                error = %error_chain(error),
                "the owner of the state failed: the daemon stops"
            );
        }
        let outcome = if owner.is_some() || side.is_some() {
            "failed"
        } else {
            "requested"
        };
        tracing::info!(event = LogEvent::DaemonStopped.as_str(), outcome, "the daemon stops");
    });
}

/// How long one command of the bus may take before the owner gives up on it: the bus holds its lock for the length of
/// one operation, so a command that takes longer is stuck.
const BUS_TIMEOUT: Duration = Duration::from_secs(15);

/// The bus of the home, run with the program the configuration names.
fn make_bus(home: &Home, settings: &Settings) -> AgentBus {
    AgentBus::new(
        settings.config.binaries().agentbus.clone(),
        home.absolute_root().clone(),
    )
    .with_timeout(BUS_TIMEOUT)
}

/// Starts the opener of questions files among `sides` when the configuration asks for it; the channel to it is returned.
fn start_opener(
    sides: &mut JoinSet<()>,
    settings: &Settings,
    surroundings: &Surroundings,
    dispatch: &tracing::Dispatch,
) -> Option<mpsc::Sender<PathBuf>> {
    match settings.config.questions().open {
        OpenQuestions::Off => None,
        OpenQuestions::Vscode => {
            let (opens, files) = mpsc::channel(opener::QUEUE);
            sides.spawn(Scoped::new(
                opener::run(files, vscode_opener(settings, surroundings)),
                dispatch.clone(),
            ));
            Some(opens)
        }
    }
}

/// The directories of the VS Code servers a home user has, under the home directory: the usual three.
const VSCODE_SERVER_DIRS: [&str; 3] = [".vscode-server", ".vscode-server-insiders", ".cursor-server"];

/// The opener of questions files: the servers the configuration names, else the usual ones in the home of the user.
fn vscode_opener(settings: &Settings, surroundings: &Surroundings) -> VscodeOpener {
    let configured = &settings.config.questions().vscode_server_dirs;
    let server_dirs: Vec<PathBuf> = if configured.is_empty() {
        surroundings
            .user_home
            .iter()
            .flat_map(|home| VSCODE_SERVER_DIRS.iter().map(move |dir| home.join(dir)))
            .collect()
    } else {
        configured.iter().map(|dir| dir.as_path().to_path_buf()).collect()
    };
    VscodeOpener::new(
        server_dirs,
        SocketDirs {
            runtime: surroundings.runtime_dir.clone(),
            fallback: surroundings.fallback_dir.clone(),
        },
    )
}
