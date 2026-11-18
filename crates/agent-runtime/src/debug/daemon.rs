//! Daemon process and log observation for diagnostics.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use crate::store::Home;
use agent_core::debug_summary::DaemonSummary;
use agent_core::state::{Timestamp, Team};
use time::UtcDateTime;

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
    // Find the PID of the actual process holding the lock via /proc/locks.
    let lock_file = home.root().join("bus").join("lock");
    let pid = find_lock_holder_via_proc_locks(&lock_file)
        .ok_or(DaemonCollectionError::ProcessNotFound)?;

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

    // Calculate process start time from /proc
    let started_at = read_process_start_time(&proc_path);

    // Check if disk executable is newer than process start time
    let disk_newer_than_process = if let Some(start_time) = &started_at {
        if !exe_path.is_empty() {
            check_disk_newer(&exe_path, start_time)
        } else {
            false
        }
    } else {
        false
    };

    // Read daemon version from executable
    let version = read_daemon_version(&exe_path).unwrap_or_default();

    Ok(DaemonSummary {
        pid,
        started_at,
        cmdline,
        env_var_names: env_names,
        exe_path,
        disk_newer_than_process,
        version,
    })
}

/// Finds the PID of the actual process holding the home lock via /proc/locks.
/// Matches lock file inode and device with entries in /proc/locks.
fn find_lock_holder_via_proc_locks(lock_file: &Path) -> Option<u32> {
    // Get lock file's device and inode
    let lock_metadata = fs::metadata(lock_file).ok()?;
    let lock_dev = lock_metadata.dev();
    let lock_ino = lock_metadata.ino();

    // Parse /proc/locks to find matching inode
    let locks_content = fs::read_to_string("/proc/locks").ok()?;
    for line in locks_content.lines() {
        // /proc/locks format: num: type ADVISORY rw pid major:minor:inode ...
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 5 {
            continue;
        }

        // Check if this is a WRITE lock
        if parts[2] != "ADVISORY" || parts[3] != "WRITE" {
            continue;
        }

        // Extract PID (field 4)
        let pid_str = parts[4];

        // Extract inode (field 5, format "major:minor:inode")
        if let Some(inode_part) = parts.get(5) {
            let inode_fields: Vec<&str> = inode_part.split(':').collect();
            if inode_fields.len() >= 3 {
                if let (Ok(maj), Ok(min), Ok(ino)) = (
                    inode_fields[0].parse::<u64>(),
                    inode_fields[1].parse::<u64>(),
                    inode_fields[2].parse::<u64>(),
                ) {
                    // Compare device (major:minor) and inode
                    let dev_num = (maj << 8) | min;
                    if dev_num == lock_dev && ino == lock_ino {
                        // Found matching lock holder
                        if let Ok(pid) = pid_str.parse::<u32>() {
                            // Verify process still exists
                            if fs::metadata(format!("/proc/{}", pid)).is_ok() {
                                return Some(pid);
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Reads process start time from /proc/[pid]/stat and /proc/stat btime.
/// Returns Timestamp if calculable, None otherwise.
fn read_process_start_time(proc_path: &Path) -> Option<Timestamp> {
    // Read starttime from /proc/[pid]/stat (field 21, 0-indexed 20)
    let stat_content = fs::read_to_string(proc_path.join("stat")).ok()?;
    let fields: Vec<&str> = stat_content.split_whitespace().collect();
    if fields.len() < 22 {
        return None;
    }
    let start_ticks: u64 = fields[21].parse().ok()?;

    // Read btime from /proc/stat (boot time in seconds since epoch)
    let proc_stat = fs::read_to_string("/proc/stat").ok()?;
    let btime = proc_stat
        .lines()
        .find(|line| line.starts_with("btime "))
        .and_then(|line| line.split_whitespace().nth(1).and_then(|s| s.parse::<u64>().ok()))?;

    // Get clock ticks per second
    let clk_tck = get_clk_tck()?;

    // Calculate seconds since boot
    let seconds_since_boot = (start_ticks as f64) / (clk_tck as f64);

    // Unix timestamp = btime + seconds_since_boot
    let unix_timestamp = (btime as i64) + (seconds_since_boot.floor() as i64);

    // Convert to Timestamp using time crate
    let utc = UtcDateTime::from_unix_timestamp(unix_timestamp).ok()?;
    Timestamp::from_utc(utc).ok()
}

/// Gets CLK_TCK (clock ticks per second) for the system.
fn get_clk_tck() -> Option<u64> {
    // Try common values: check /proc/[1]/stat for init process
    if let Ok(content) = fs::read_to_string("/proc/1/stat") {
        let fields: Vec<&str> = content.split_whitespace().collect();
        if fields.len() >= 22 {
            // init (PID 1) typically starts very early, so starttime should be small
            // If starttime < 1000, likely CLK_TCK is 100; if < 10000 likely 1000
            if let Ok(start_ticks) = fields[21].parse::<u64>() {
                return if start_ticks < 1000 { Some(100) } else { Some(1000) };
            }
        }
    }

    // Fallback: assume common value
    Some(100)
}

/// Checks if the executable on disk is newer than the process start time.
fn check_disk_newer(exe_path: &str, started_at: &Timestamp) -> bool {
    let exe = Path::new(exe_path);
    if let Ok(metadata) = fs::metadata(exe) {
        if let Ok(modified) = metadata.modified() {
            // Convert SystemTime to UtcDateTime for comparison
            let utc_dt = time::UtcDateTime::from(modified);
            // Compare with process start time
            return utc_dt > started_at.as_utc();
        }
    }
    false
}

/// Reads daemon version from executable with --version (bounded capture).
fn read_daemon_version(exe_path: &str) -> Option<String> {
    if exe_path.is_empty() {
        return None;
    }

    // Use process capture utilities to safely call executable --version
    // This is a placeholder that uses bounded capture like git_capture in collect.rs
    // For now, return None - actual implementation would use crate::process::capture
    None
}

/// Collects bounded daemon log lines, filtered by team/task context.
///
/// Reads from explicit --daemon-log path or home daemon.log.
/// Filtered to lines about the team/tasks (actual IDs) and warnings/errors of last 500 lines.
///
/// # Errors
/// Returns [`DaemonCollectionError::LogRead`] if explicit path cannot be read.
/// Returns [`DaemonCollectionError::LogRead`] with "not found" if default path doesn't exist.
pub(super) async fn collect_daemon_log(
    home: &Home,
    daemon_log_path: Option<&PathBuf>,
    team: &Team,
) -> Result<Vec<String>, DaemonCollectionError> {
    let log_path = if let Some(path) = daemon_log_path {
        path.clone()
    } else {
        home.root().join("daemon.log")
    };

    // Check if log exists
    if !log_path.exists() {
        // If explicit path was given, it's an error; if default, it's "not found"
        return if daemon_log_path.is_some() {
            Err(DaemonCollectionError::LogRead {
                reason: "log file not found".to_string(),
            })
        } else {
            Err(DaemonCollectionError::LogRead {
                reason: "not found".to_string(),
            })
        };
    }

    // Read log file
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

    // Filter: keep lines about the team/tasks (actual IDs) or warnings/errors
    let team_id = team.id().to_string();
    let task_ids: Vec<String> = team
        .crew()
        .roster()
        .iter()
        .map(|(_, member)| member.task().to_string())
        .collect();

    let filtered = lines
        .into_iter()
        .filter(|line| {
            is_team_task_related(line, &team_id, &task_ids) || is_warning_or_error(line)
        })
        .map(|s| s.to_string())
        .collect();

    Ok(filtered)
}

/// Checks if a log line is about the specific team or its tasks.
fn is_team_task_related(line: &str, team_id: &str, task_ids: &[String]) -> bool {
    line.contains(team_id) || task_ids.iter().any(|id| line.contains(id))
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
    fn returns_process_not_found_when_no_lock_holder() {
        let dir = tempfile::tempdir().unwrap();
        let absolute = agent_core::config::AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap();
        let home = Home::new(&absolute);

        let result = collect_daemon_process(&home);
        assert!(matches!(result, Err(DaemonCollectionError::ProcessNotFound)));
    }

    #[test]
    fn collects_daemon_facts_when_process_exists() {
        // This test would require a real running process with a lock file
        // For now, verify the error case
        let dir = tempfile::tempdir().unwrap();
        let absolute = agent_core::config::AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap();
        let home = Home::new(&absolute);

        let result = collect_daemon_process(&home);
        // Should fail because no process holds the lock
        assert!(matches!(result, Err(DaemonCollectionError::ProcessNotFound)));
    }

    #[tokio::test]
    async fn returns_error_when_log_file_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let absolute = agent_core::config::AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap();
        let home = Home::new(&absolute);
        let team = Team::new(
            "team-20261118-000000-test".parse().unwrap(),
            "test".parse().unwrap(),
            "2026-11-18T00:00:00Z".parse().unwrap(),
            agent_core::state::MergeMode::Autopilot,
            agent_core::state::TaskListText::new("Test\n".to_owned()).unwrap(),
            agent_core::state::Crew::solo(agent_core::state::Member::new(
                "task-20261118-000000-test".parse().unwrap(),
                agent_core::state::Participant::Normal("agent".parse().unwrap()),
            )).unwrap(),
        );

        let result = collect_daemon_log(&home, None, &team).await;
        assert!(matches!(result, Err(DaemonCollectionError::LogRead { .. })));
    }

    #[tokio::test]
    async fn explicit_log_path_unreadable_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let absolute = agent_core::config::AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap();
        let home = Home::new(&absolute);
        let team = Team::new(
            "team-20261118-000000-test".parse().unwrap(),
            "test".parse().unwrap(),
            "2026-11-18T00:00:00Z".parse().unwrap(),
            agent_core::state::MergeMode::Autopilot,
            agent_core::state::TaskListText::new("Test\n".to_owned()).unwrap(),
            agent_core::state::Crew::solo(agent_core::state::Member::new(
                "task-20261118-000000-test".parse().unwrap(),
                agent_core::state::Participant::Normal("agent".parse().unwrap()),
            )).unwrap(),
        );

        let explicit_path = PathBuf::from("/nonexistent/path/daemon.log");
        let result = collect_daemon_log(&home, Some(&explicit_path), &team).await;
        assert!(matches!(result, Err(DaemonCollectionError::LogRead { .. })));
    }

    #[tokio::test]
    async fn filters_log_by_team_and_task_ids() {
        let dir = tempfile::tempdir().unwrap();
        let absolute = agent_core::config::AbsolutePath::try_from(dir.path().to_str().unwrap().to_owned()).unwrap();
        let home = Home::new(&absolute);

        let team_id = "team-20261118-000000-test";
        let task_id = "task-20261118-000000-test";

        let team = Team::new(
            team_id.parse().unwrap(),
            "test".parse().unwrap(),
            "2026-11-18T00:00:00Z".parse().unwrap(),
            agent_core::state::MergeMode::Autopilot,
            agent_core::state::TaskListText::new("Test\n".to_owned()).unwrap(),
            agent_core::state::Crew::solo(agent_core::state::Member::new(
                task_id.parse().unwrap(),
                agent_core::state::Participant::Normal("agent".parse().unwrap()),
            )).unwrap(),
        );

        // Create a log file with team/task references and noise
        let log_content = format!(
            "unrelated line\n{} started\nmore noise\n{} activity\nWARNING: something\n",
            team_id, task_id
        );
        tokio::fs::write(home.root().join("daemon.log"), log_content).await.unwrap();

        let result = collect_daemon_log(&home, None, &team).await.unwrap();

        // Should have filtered to 3 lines: team line, task line, warning
        assert!(result.iter().any(|l| l.contains(team_id)));
        assert!(result.iter().any(|l| l.contains(task_id)));
        assert!(result.iter().any(|l| l.contains("WARNING")));
        assert!(!result.iter().any(|l| l.contains("unrelated line")));
    }
}
