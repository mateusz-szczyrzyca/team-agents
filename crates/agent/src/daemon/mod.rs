//! The daemon: the single owner of the state and of every model run.
//!
//! One daemon runs per home (the lock of [`StateStore`]); every other command and the dashboard are clients of its
//! socket (`docs/architecture.md`, "Process model"). The daemon is a small group of tasks that talk through channels:
//!
//! - the **owner** holds the state and is the only one that changes it: it answers requests, starts a run for every
//!   queued task, applies the end of each run, wakes paused tasks and publishes events;
//! - the **server** accepts connections on the socket and passes their requests to the owner;
//! - the **notifier** delivers the notifications the owner asks for.
//!
//! Cancelling the shutdown token stops them in order: the server stops accepting, the owner stops its runs and records
//! them as interrupted, the notifier delivers what is queued, and the socket is removed.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_core::config::OpenQuestions;
use agent_runtime::agentbus::AgentBus;
use agent_runtime::clock::SharedClock;
use agent_runtime::editor::{SocketDirs, VscodeOpener};
use agent_runtime::harness_prompts::{HarnessPrompts, HarnessPromptsError};
use agent_runtime::notify::Notifier;
use agent_runtime::process::Supervisor;
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

use self::owner::{EVENT_BACKLOG, INBOX, MAX_RUNS, Owner, OwnerError, OwnerParts};
use self::server::ServerError;

mod control;
mod notice;
mod notifier;
mod opener;
mod owner;
mod questions;
mod recovery;
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
pub(crate) async fn run(setup: DaemonSetup, shutdown: CancellationToken) -> Result<(), DaemonError> {
    run_in(setup, Surroundings::from_process(), shutdown).await
}

/// Runs the daemon until `shutdown` is cancelled, in `surroundings`.
///
/// # Errors
/// [`DaemonError`].
pub(crate) async fn run_in(
    setup: DaemonSetup,
    surroundings: Surroundings,
    shutdown: CancellationToken,
) -> Result<(), DaemonError> {
    let store = match StateStore::open(setup.home.clone()).await {
        Ok(store) => Arc::new(store),
        Err(StoreError::HomeLocked { path }) => return Err(DaemonError::AlreadyRunning { path }),
        Err(error) => return Err(error.into()),
    };
    let settings = Arc::new(Settings::load(&setup.home).await?);
    let language = settings.config.language().clone();
    let texts = Arc::new(HarnessPrompts::load(&setup.prompts_dir, &language).await?);
    let team_texts = Arc::new(TeamPrompts::load(&setup.prompts_dir, &language).await?);
    let question_texts = Arc::new(QuestionsPrompts::load(&setup.prompts_dir, &language).await?);
    let rules = Arc::new(PromptLoader::load(&setup.prompts_dir, &language).await?);
    let bus = make_bus(&setup.home, &settings);
    let state = store.load().await?;
    let timings = setup.timings.unwrap_or_else(|| Timings {
        tick: TICK,
        run: RunTimings::from_config(settings.config.watchdog()),
    });
    let rng = StdRng::try_from_rng(&mut SysRng).map_err(DaemonError::Entropy)?;
    let engine = RunEngine::new(EngineParts {
        store: Arc::clone(&store),
        texts: Arc::clone(&texts),
        config: Arc::new(settings.config.clone()),
        secrets: Arc::new(settings.secrets.clone()),
        supervisor: Supervisor::new(MAX_RUNS),
        clock: setup.clock.clone(),
        local_offset: setup.local_offset,
        timings: timings.run,
    });
    let notifier = Notifier::from_config(settings.config.notifications(), store.home(), &settings.secrets);
    let listener = server::bind(&setup.home).await?;

    let (inbox, requests) = mpsc::channel(INBOX);
    let (notices, deliveries) = mpsc::channel(notifier::QUEUE);
    let (events, _unused) = broadcast::channel(EVENT_BACKLOG);
    let mut sides: JoinSet<()> = JoinSet::new();
    let serving = shutdown.clone();
    sides.spawn(server::serve(listener, inbox, serving));
    let delivering = Arc::clone(&texts);
    sides.spawn(notifier::run(deliveries, delivering, notifier));
    let opens = start_opener(&mut sides, &settings, &surroundings);

    let owner = Owner::new(
        OwnerParts {
            store,
            state,
            settings,
            texts,
            team_texts,
            question_texts,
            rules,
            bus,
            opens,
            engine,
            clock: setup.clock.clone(),
            rng,
            notices,
            events,
            tick: timings.tick,
        },
        shutdown.clone(),
    );
    let owned_end = owner.run(requests).await;
    // Whatever ended the owner, the rest of the daemon ends with it.
    shutdown.cancel();
    let mut failure = None;
    while let Some(joined) = sides.join_next().await {
        if let Err(source) = joined {
            tracing::error!(%source, "a task of the daemon failed");
            failure.get_or_insert(DaemonError::Join(source));
        }
    }
    server::remove(&setup.home).await.map_err(DaemonError::RemoveSocket)?;
    owned_end?;
    failure.map_or(Ok(()), Err)
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
) -> Option<mpsc::Sender<PathBuf>> {
    match settings.config.questions().open {
        OpenQuestions::Off => None,
        OpenQuestions::Vscode => {
            let (opens, files) = mpsc::channel(opener::QUEUE);
            sides.spawn(opener::run(files, vscode_opener(settings, surroundings)));
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
