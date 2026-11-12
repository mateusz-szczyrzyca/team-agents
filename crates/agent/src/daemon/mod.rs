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
use std::time::Duration;

use agent_core::config::Language;
use agent_runtime::clock::SharedClock;
use agent_runtime::harness_prompts::HarnessPromptsError;
use agent_runtime::prompts::PromptLoadError;
use agent_runtime::run::RunTimings;
use agent_runtime::settings::SettingsError;
use agent_runtime::store::{Home, StoreError};
use time::UtcOffset;
use tokio_util::sync::CancellationToken;

mod notice;
mod recovery;
mod settle;

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
    /// The rules cannot be loaded.
    #[error("cannot load the rules")]
    Prompts(#[from] PromptLoadError),
}

/// Runs the daemon until `shutdown` is cancelled.
///
/// # Errors
/// [`DaemonError`].
pub(crate) async fn run(setup: DaemonSetup, shutdown: CancellationToken) -> Result<(), DaemonError> {
    let _ = (setup, shutdown, Language::default());
    Ok(())
}
