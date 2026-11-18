//! Daemon process and log observation for diagnostics.

use std::path::PathBuf;
use crate::store::Home;
use agent_core::debug_summary::DaemonSummary;

/// Daemon process and log facts that could not be collected.
#[derive(Debug, Clone)]
pub(super) enum DaemonCollectionError {
    /// Process information is not available.
    ProcessNotFound,
    /// Log file could not be read.
    LogRead { reason: String },
    /// Process start time could not be determined.
    StartTimeUnknown,
}

impl std::fmt::Display for DaemonCollectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProcessNotFound => write!(f, "daemon process not found"),
            Self::LogRead { reason } => write!(f, "cannot read daemon log: {}", reason),
            Self::StartTimeUnknown => write!(f, "daemon start time not available"),
        }
    }
}

impl std::error::Error for DaemonCollectionError {}

/// Collects daemon process facts chosen by the home lock owner.
///
/// Returns PID, start time, cmdline, environment variable names (no values),
/// executable path, whether disk is newer than process, and version string.
///
/// # Errors
/// Returns [`DaemonCollectionError::ProcessNotFound`] if no process holds the home.
pub(super) fn collect_daemon_process(_home: &Home) -> Result<DaemonSummary, DaemonCollectionError> {
    // Stub: to be implemented.
    Err(DaemonCollectionError::ProcessNotFound)
}

/// Collects bounded daemon log lines, filtered by team/task context.
///
/// Reads from explicit --daemon-log path or home daemon.log.
/// Filtered to lines about the team/tasks and warnings/errors of last 500 lines.
///
/// # Errors
/// Returns [`DaemonCollectionError::LogRead`] if the log cannot be read.
pub(super) async fn collect_daemon_log(
    _home: &Home,
    _daemon_log_path: Option<&PathBuf>,
) -> Result<Vec<String>, DaemonCollectionError> {
    // Stub: to be implemented.
    Err(DaemonCollectionError::LogRead {
        reason: "not implemented".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_process_not_found_when_no_daemon_lock() {
        let dir = tempfile::tempdir().unwrap();
        let absolute = agent_core::config::AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap();
        let home = Home::new(&absolute);

        let result = collect_daemon_process(&home);
        assert!(matches!(result, Err(DaemonCollectionError::ProcessNotFound)));
    }

    #[tokio::test]
    async fn returns_error_when_log_file_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let absolute = agent_core::config::AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap();
        let home = Home::new(&absolute);

        let result = collect_daemon_log(&home, None).await;
        assert!(matches!(result, Err(DaemonCollectionError::LogRead { .. })));
    }
}
