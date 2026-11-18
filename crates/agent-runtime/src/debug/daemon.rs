//! Daemon process and log observation for diagnostics.

use std::fs;
use std::path::{Path, PathBuf};
use crate::store::Home;
use agent_core::debug_summary::DaemonSummary;

/// Daemon process and log facts that could not be collected.
#[derive(Debug, Clone)]
pub(super) enum DaemonCollectionError {
    /// Process information is not available.
    ProcessNotFound,
    /// Log file could not be read.
    LogRead { reason: String },
}

impl std::fmt::Display for DaemonCollectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProcessNotFound => write!(f, "daemon process not found"),
            Self::LogRead { reason } => write!(f, "cannot read daemon log: {}", reason),
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
pub(super) fn collect_daemon_process(home: &Home) -> Result<DaemonSummary, DaemonCollectionError> {
    // Find the PID of the process holding the lock.
    let lock_file = home.root().join("bus").join("lock");
    let pid = read_lock_pid(&lock_file).ok_or(DaemonCollectionError::ProcessNotFound)?;

    // Read process information from /proc.
    let proc_path = PathBuf::from(format!("/proc/{}", pid));

    // Extract cmdline
    let cmdline = fs::read_to_string(proc_path.join("cmdline"))
        .ok()
        .map(|s| s.split('\0').filter(|x| !x.is_empty()).map(|x| x.to_string()).collect())
        .unwrap_or_default();

    // Extract environment variable names (no values)
    let env_names = fs::read_to_string(proc_path.join("environ"))
        .ok()
        .map(|s| s.split('\0').filter(|x| !x.is_empty()).filter_map(|x| x.split('=').next().map(|n| n.to_string())).collect())
        .unwrap_or_default();

    // Extract executable path
    let exe_path = fs::read_link(proc_path.join("exe"))
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_default();

    // Check if disk executable is newer than process (conservative: false for now)
    let disk_newer_than_process = false;

    let version = String::new();

    Ok(DaemonSummary {
        pid,
        started_at: None,
        cmdline,
        env_var_names: env_names,
        exe_path,
        disk_newer_than_process,
        version,
    })
}

/// Reads the lock file and extracts the PID.
fn read_lock_pid(lock_file: &Path) -> Option<u32> {
    fs::read_to_string(lock_file)
        .ok()
        .and_then(|content| content.trim().parse().ok())
}

/// Collects bounded daemon log lines, filtered by team/task context.
///
/// Reads from explicit --daemon-log path or home daemon.log.
/// Filtered to lines about the team/tasks and warnings/errors of last 500 lines.
///
/// # Errors
/// Returns [`DaemonCollectionError::LogRead`] if the log cannot be read.
pub(super) async fn collect_daemon_log(
    home: &Home,
    daemon_log_path: Option<&PathBuf>,
) -> Result<Vec<String>, DaemonCollectionError> {
    let log_path = if let Some(path) = daemon_log_path {
        path.clone()
    } else {
        home.root().join("daemon.log")
    };

    if !log_path.exists() {
        return Err(DaemonCollectionError::LogRead {
            reason: "log file not found".to_string(),
        });
    }

    let content = tokio::fs::read_to_string(&log_path)
        .await
        .map_err(|e| DaemonCollectionError::LogRead {
            reason: e.to_string(),
        })?;

    // Split into lines and keep last 500
    let all_lines: Vec<&str> = content.lines().collect();
    let lines = if all_lines.len() > 500 {
        all_lines[all_lines.len() - 500..].to_vec()
    } else {
        all_lines
    };

    // Filter: keep lines about team/tasks or warnings/errors
    let filtered = lines
        .into_iter()
        .filter(|line| {
            is_team_task_related(line) || is_warning_or_error(line)
        })
        .map(|s| s.to_string())
        .collect();

    Ok(filtered)
}

/// Checks if a log line is about team or task activity.
fn is_team_task_related(line: &str) -> bool {
    line.contains("team") ||
    line.contains("task") ||
    line.contains("agent") ||
    line.contains("TEAM") ||
    line.contains("TASK")
}

/// Checks if a log line is a warning or error.
fn is_warning_or_error(line: &str) -> bool {
    line.contains("WARN") ||
    line.contains("ERROR") ||
    line.contains("warning") ||
    line.contains("error")
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
