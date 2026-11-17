//! `agent debug`: collects diagnostics for a round, team or task and writes a summary into a bundle directory.

use std::io::Write;
use std::path::PathBuf;

use agent_core::ids::{RoundId, TaskId, TeamId};
use agent_core::protocol::{Client, Command, Reply, StatusRequest, Target};
use agent_runtime::debug::{DebugArgs as RuntimeDebugArgs, DebugRuntimeError, DebugTarget, execute_debug};
use agent_runtime::store::Home;

use crate::client::{ClientError, Connection};

/// Why `agent debug` failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DebugError {
    /// Runtime collection failed.
    #[error(transparent)]
    Runtime(#[from] DebugRuntimeError),
    /// The output cannot be written.
    #[error("cannot write output")]
    Output(#[source] std::io::Error),
    /// The client could not ask the daemon.
    #[error("cannot query daemon status")]
    Client(#[from] ClientError),
}

impl DebugError {
    /// The exit status of the command for this failure: 3 for an unknown ID, 1 otherwise.
    pub(crate) const fn exit_code(&self) -> u8 {
        match self {
            Self::Runtime(DebugRuntimeError::UnknownTarget { .. }) => 3,
            Self::Runtime(_) | Self::Output(_) | Self::Client(_) => 1,
        }
    }
}

/// Arguments for `agent debug` from the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DebugCliArgs {
    /// The round to debug.
    pub(crate) round: Option<RoundId>,
    /// The team to debug.
    pub(crate) team: Option<TeamId>,
    /// The task to debug.
    pub(crate) task: Option<TaskId>,
    /// Parent directory for the debug bundle.
    pub(crate) out: Option<PathBuf>,
    /// Stale threshold in minutes.
    pub(crate) stale_minutes: Option<u64>,
    /// Daemon log file (accepted, deferred).
    pub(crate) daemon_log: Option<PathBuf>,
}

/// Queries daemon status if the daemon is online, returning status text or the connection/query error.
pub(crate) async fn query_daemon_status(home: &Home, target: &Target) -> Option<Result<String, String>> {
    let socket = home.socket_file();
    match Connection::connect(&socket).await {
        Ok(mut connection) => {
            let request = Command::Status(StatusRequest {
                target: Some(target.clone()),
            });
            match connection.request(Client::Cli, request).await {
                Ok(Reply::Overview { teams, history }) => Some(Ok(format!(
                    "{}{}",
                    crate::commands::status::render_active(&teams, &|_| None),
                    crate::commands::status::render_history(Some(&history)),
                ))),
                Ok(Reply::Teams { teams }) => Some(Ok(crate::commands::status::render_active(&teams, &|_| None))),
                Ok(other) => Some(Err(format!("unexpected daemon reply: {other:?}"))),
                Err(err) => Some(Err(err.to_string())),
            }
        }
        Err(err) => Some(Err(err.to_string())),
    }
}

/// Carries out `agent debug`.
///
/// # Errors
/// [`DebugError`].
pub(crate) async fn debug(home: &Home, args: DebugCliArgs, out: &mut impl Write) -> Result<(), DebugError> {
    let (target, protocol_target) = match (&args.round, &args.team, &args.task) {
        (Some(round), None, None) => (DebugTarget::Round(round.clone()), Target::Round(round.clone())),
        (None, Some(team), None) => (DebugTarget::Team(team.clone()), Target::Team(team.clone())),
        (None, None, Some(task)) => (DebugTarget::Task(task.clone()), Target::Task(task.clone())),
        _ => {
            return Err(DebugError::Runtime(DebugRuntimeError::UnknownTarget {
                id: "missing or conflicting target".to_string(),
            }));
        }
    };
    let status = query_daemon_status(home, &protocol_target).await;
    let runtime_args = RuntimeDebugArgs {
        target,
        out: args.out,
        stale_minutes: args.stale_minutes,
        daemon_log: args.daemon_log,
    };
    let result = execute_debug(home, runtime_args, status, out).await?;
    writeln!(out, "Debug summary: {}", result.summary_path.display()).map_err(DebugError::Output)?;
    writeln!(out, "Debug directory: {}", result.bundle_dir.display()).map_err(DebugError::Output)?;
    Ok(())
}
