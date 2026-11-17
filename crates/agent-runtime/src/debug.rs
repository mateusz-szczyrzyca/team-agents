//! Debug diagnostic collection: gather facts from home and render a bounded summary.

use std::io::Write;
use std::path::PathBuf;

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
mod tests {
    use super::*;

    #[test]
    fn debug_target_round_eq() {
        let round = RoundId::parse("round-20261117-230000-xxxx").unwrap();
        let target1 = DebugTarget::Round(round.clone());
        let target2 = DebugTarget::Round(round);
        assert_eq!(target1, target2);
    }

    #[test]
    fn debug_target_team_eq() {
        let team = TeamId::parse("team-20261117-230000-xxxx").unwrap();
        let target1 = DebugTarget::Team(team.clone());
        let target2 = DebugTarget::Team(team);
        assert_eq!(target1, target2);
    }

    #[test]
    fn debug_target_task_eq() {
        let task = TaskId::parse("task-20261117-230000-xxxx").unwrap();
        let target1 = DebugTarget::Task(task.clone());
        let target2 = DebugTarget::Task(task);
        assert_eq!(target1, target2);
    }
}
