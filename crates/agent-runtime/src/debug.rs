//! Debug diagnostic collection: gather facts from home and render a bounded summary.

use std::io::Write;
use std::path::PathBuf;

use agent_core::config::Secrets;
use agent_core::ids::{RoundId, TeamId, TaskId};
use agent_core::state::State;

use crate::store::Home;

/// A debug target: round, team or task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugTarget {
    /// A round ID.
    Round(RoundId),
    /// A team ID.
    Team(TeamId),
    /// A task ID.
    Task(TaskId),
}

/// Arguments for debug collection.
#[derive(Debug, Clone)]
pub struct DebugArgs {
    /// What to collect diagnostics for.
    pub target: DebugTarget,
    /// Directory to write debug bundle to (creates subdirectory); defaults to home debug dir.
    pub out: Option<PathBuf>,
    /// How long without activity a task is considered stale, in minutes (default 120).
    pub stale_minutes: Option<u64>,
    /// Daemon log file to read when daemon does not answer (deferred).
    pub daemon_log: Option<PathBuf>,
}

/// Result of successful debug collection.
#[derive(Debug, Clone)]
pub struct DebugResult {
    /// Directory containing collected facts.
    pub bundle_dir: PathBuf,
    /// Path to SUMMARY.md file.
    pub summary_path: PathBuf,
}

/// Why debug collection failed.
#[derive(Debug, thiserror::Error)]
pub enum DebugRuntimeError {
    /// The home directory cannot be used.
    #[error("cannot read home")]
    Home(#[source] std::io::Error),
    /// The secrets file is not readable; collection cannot proceed.
    #[error("secrets file not readable: {reason}")]
    SecretsUnreadable {
        /// Why the secrets file could not be read.
        reason: String,
    },
    /// The ID provided does not resolve to a known team.
    #[error("unknown or unresolvable ID: {id}")]
    UnknownTarget {
        /// The ID that was not found.
        id: String,
    },
    /// The state cannot be loaded.
    #[error("cannot load state")]
    StateRead(#[source] std::io::Error),
    /// Output cannot be written.
    #[error("cannot write output")]
    Output(#[source] std::io::Error),
}

/// Resolves a debug target to its team ID.
///
/// # Errors
/// [`DebugRuntimeError::UnknownTarget`] when the ID does not resolve to a known team.
pub fn resolve_debug_target(state: &State, target: &DebugTarget) -> Result<TeamId, DebugRuntimeError> {
    match target {
        DebugTarget::Round(round_id) => {
            state.find_round(round_id)
                .map(|(team, _round)| team.id().clone())
                .ok_or_else(|| DebugRuntimeError::UnknownTarget {
                    id: round_id.to_string(),
                })
        }
        DebugTarget::Team(team_id) => {
            state.team(team_id)
                .map(|_team| team_id.clone())
                .ok_or_else(|| DebugRuntimeError::UnknownTarget {
                    id: team_id.to_string(),
                })
        }
        DebugTarget::Task(task_id) => {
            state.find_task(task_id)
                .map(|task_ref| task_ref.team.id().clone())
                .ok_or_else(|| DebugRuntimeError::UnknownTarget {
                    id: task_id.to_string(),
                })
        }
    }
}

/// Loads and validates that secrets file is readable before collection begins.
///
/// # Errors
/// [`DebugRuntimeError::SecretsUnreadable`] when secrets file cannot be read or parsed.
pub async fn load_debug_secrets(home: &Home) -> Result<Secrets, DebugRuntimeError> {
    let secrets_path = home.secrets_file();
    let text = tokio::fs::read_to_string(&secrets_path)
        .await
        .map_err(|e| DebugRuntimeError::SecretsUnreadable {
            reason: format!("cannot read {}: {}", secrets_path.display(), e),
        })?;

    let _metadata = tokio::fs::metadata(&secrets_path)
        .await
        .map_err(|e| DebugRuntimeError::SecretsUnreadable {
            reason: format!("cannot stat {}: {}", secrets_path.display(), e),
        })?;

    Secrets::parse(&text, 0o600)
        .map_err(|_| DebugRuntimeError::SecretsUnreadable {
            reason: "secrets file is malformed".to_string(),
        })
}

/// Collects diagnostic facts and renders a summary.
///
/// # Errors
/// [`DebugRuntimeError`] when facts cannot be collected, secrets are unreadable, or output cannot be written.
pub async fn execute_debug(
    home: &Home,
    args: DebugArgs,
    _status: Option<Result<String, String>>,
    out: &mut impl Write,
) -> Result<DebugResult, DebugRuntimeError> {
    // Placeholder implementation
    writeln!(out, "execute_debug placeholder").map_err(DebugRuntimeError::Output)?;
    Err(DebugRuntimeError::UnknownTarget {
        id: "not implemented".to_string(),
    })
}

#[cfg(test)]
pub mod tests;

