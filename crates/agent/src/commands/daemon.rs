//! `agent daemon`: the daemon, in this process, until SIGTERM or SIGINT.

use std::io;
use std::path::PathBuf;

use tokio::signal::unix::{SignalKind, signal};
use tokio::task::JoinError;
use tokio_util::sync::CancellationToken;

use crate::daemon::{self, DaemonError, DaemonSetup};
use crate::environment::{Environment, EnvironmentError};
use agent_runtime::clock::{SharedClock, SystemClock};

/// Why `agent daemon` failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DaemonCommandError {
    /// `AGENTS_HOME` cannot be used.
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    /// The signals cannot be watched.
    #[error("cannot watch SIGTERM and SIGINT")]
    Signals(#[source] io::Error),
    /// The daemon stopped with an error.
    #[error(transparent)]
    Daemon(#[from] DaemonError),
    /// The daemon's structured logging subscriber could not be installed.
    #[error("cannot install the daemon logging subscriber")]
    LogSubscriber(#[source] tracing::subscriber::SetGlobalDefaultError),
    /// The task that watches the signals failed.
    #[error("the task that watches the signals failed")]
    Watcher(#[source] JoinError),
}

/// Runs the daemon of the home of `environment` until it is told to stop, logging to standard error.
///
/// # Errors
/// [`DaemonCommandError`].
pub(crate) async fn daemon(environment: &Environment, prompts_dir: Option<PathBuf>) -> Result<(), DaemonCommandError> {
    let home = environment.home()?;
    let log_layer =
        agent_runtime::history::HistoryLayer::install_global().map_err(DaemonCommandError::LogSubscriber)?;
    if let Some(error) = environment.offset_error() {
        tracing::warn!(%error, "the time zone of the machine is unknown: UTC is used to read the reset times of providers");
    }
    // The handlers are installed before the daemon starts, so that no signal is missed while it starts.
    let mut terminate = signal(SignalKind::terminate()).map_err(DaemonCommandError::Signals)?;
    let mut interrupt = signal(SignalKind::interrupt()).map_err(DaemonCommandError::Signals)?;
    let shutdown = CancellationToken::new();
    let stop = shutdown.clone();
    let watcher = tokio::spawn(async move {
        tokio::select! {
            _ = terminate.recv() => tracing::info!("SIGTERM received: the daemon stops"),
            _ = interrupt.recv() => tracing::info!("SIGINT received: the daemon stops"),
            () = stop.cancelled() => {}
        }
        stop.cancel();
    });
    let setup = DaemonSetup {
        prompts_dir: prompts_dir.unwrap_or_else(|| home.prompts_dir()),
        home,
        clock: SharedClock::new(SystemClock),
        local_offset: environment.local_offset(),
        timings: None,
    };
    tracing::info!(home = %setup.home.root().display(), "the daemon starts");
    let ended = daemon::run_with_log_layer(setup, shutdown.clone(), log_layer).await;
    shutdown.cancel();
    watcher.await.map_err(DaemonCommandError::Watcher)?;
    ended?;
    tracing::info!("the daemon has stopped");
    Ok(())
}
