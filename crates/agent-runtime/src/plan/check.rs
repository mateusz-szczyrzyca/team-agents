//! Execution of plan step checks.

use std::io::{self, SeekFrom};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Stdio;
use std::time::Instant;

use agent_core::ids::{ItemId, RunId, TaskId, TeamId};
use agent_core::plan::judge::{JudgementState, refuse_command};
use agent_core::plan::{Check, Step};
use agent_core::repo_path::RepoPath;
use agent_core::state::Timestamp;
use rustix::process::Pid;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::process::Command;
use tokio::time::timeout;

use crate::history::JudgementRecord;
use crate::process::{GroupSignal, signal_group};

#[cfg(test)]
mod tests;

/// Errors that can happen while inspecting paths for checks.
#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    /// A check duration overflowed the supported representation.
    #[error("check duration exceeds u64 milliseconds")]
    DurationOverflow {
        /// The integer conversion error.
        #[source]
        source: std::num::TryFromIntError,
    },
}

/// Context for running a step check.
#[derive(Debug, Clone)]
pub struct CheckContext<'a> {
    /// The task's worktree directory.
    pub worktree: &'a Path,
    /// The task's directory in the store (`tasks/<task>`).
    pub task_dir: &'a Path,
    /// The current run ID, if any.
    pub run: Option<&'a RunId>,
    /// Timeout in seconds for command checks.
    pub check_seconds: u64,
    /// Prefixes of refused commands (e.g. `agent gate`, `make gate`).
    pub check_refuse: &'a [String],
    /// Environment variables for command checks.
    pub envs: &'a [(String, String)],
}

/// The result of executing a step check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgementResult {
    /// The outcome state.
    pub state: JudgementState,
    /// The check as a line, if any.
    pub check_line: Option<String>,
    /// Why the check failed or was refused, if any.
    pub reason: Option<String>,
    /// How long check execution took in milliseconds.
    pub duration_ms: u64,
    /// The task-relative output path, if an output file was written.
    pub output_path: Option<String>,
    /// Trailing output lines for mismatch notification.
    pub tail: String,
    /// Whether the check was refused because it matched `check_refuse`.
    pub gate_refused: bool,
}

impl JudgementResult {
    /// Converts this result into a [`JudgementRecord`] ready to be written to history.
    #[must_use]
    pub fn into_record(
        self,
        task: TaskId,
        team: TeamId,
        run: Option<RunId>,
        at: Timestamp,
        step: ItemId,
    ) -> JudgementRecord {
        JudgementRecord {
            task,
            team,
            run,
            at,
            step,
            state: self.state,
            check: self.check_line,
            reason: self.reason,
            duration_ms: self.duration_ms,
            output_path: self.output_path,
        }
    }
}

fn duration_millis(elapsed: std::time::Duration) -> Result<u64, CheckError> {
    u64::try_from(elapsed.as_millis()).map_err(|source| CheckError::DurationOverflow { source })
}

async fn check_exists(
    worktree: &Path,
    repo_path: &RepoPath,
    check_line: String,
) -> Result<JudgementResult, CheckError> {
    let start = Instant::now();
    let full_path = worktree.join(repo_path.as_str());

    let (state, reason) = if let Err(error) = fs::symlink_metadata(&full_path).await {
        let reason = if error.kind() == io::ErrorKind::NotFound {
            format!("path does not exist: {}", repo_path.as_str())
        } else {
            format!("cannot inspect path {}: {error}", repo_path.as_str())
        };
        (JudgementState::Mismatch, Some(reason))
    } else {
        let canonical_worktree = fs::canonicalize(worktree).await;
        let canonical_path = fs::canonicalize(&full_path).await;
        match (canonical_worktree, canonical_path) {
            (Ok(base), Ok(target)) => {
                if target.starts_with(&base) {
                    (JudgementState::Confirmed, None)
                } else {
                    (
                        JudgementState::Mismatch,
                        Some(format!("path escapes worktree: {}", repo_path.as_str())),
                    )
                }
            }
            (Err(error), _) => (
                JudgementState::Mismatch,
                Some(format!("cannot resolve worktree {}: {error}", worktree.display())),
            ),
            (_, Err(error)) => (
                JudgementState::Mismatch,
                Some(format!("cannot resolve path {}: {error}", repo_path.as_str())),
            ),
        }
    };

    let duration_ms = duration_millis(start.elapsed())?;
    Ok(JudgementResult {
        state,
        check_line: Some(check_line),
        reason,
        duration_ms,
        output_path: None,
        tail: String::new(),
        gate_refused: false,
    })
}

const TAIL_BYTES: u64 = 64 * 1024;
const TAIL_LINES: usize = 20;

async fn read_tail(path: &Path) -> String {
    let Ok(mut file) = fs::File::open(path).await else {
        return String::new();
    };
    let Ok(metadata) = file.metadata().await else {
        return String::new();
    };
    let length = metadata.len();
    let start = length.saturating_sub(TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).await.is_err() {
        return String::new();
    }
    let mut bytes = Vec::new();
    if file.take(TAIL_BYTES).read_to_end(&mut bytes).await.is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    let keep = lines.len().saturating_sub(TAIL_LINES);
    lines.into_iter().skip(keep).collect::<Vec<_>>().join("\n")
}

fn open_check_logs(
    checks_dir: &Path,
    step_id: ItemId,
) -> io::Result<(std::path::PathBuf, std::fs::File, std::fs::File)> {
    let log_path = checks_dir.join(format!("{step_id}.log"));
    let std_file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)?;
    let err_file = std_file.try_clone()?;
    Ok((log_path, std_file, err_file))
}

async fn wait_child_outcome(
    mut child: tokio::process::Child,
    group: Option<Pid>,
    check_seconds: u64,
) -> (JudgementState, Option<String>) {
    let check_duration = std::time::Duration::from_secs(check_seconds);
    match timeout(check_duration, child.wait()).await {
        Ok(Ok(status)) => {
            if status.success() {
                (JudgementState::Confirmed, None)
            } else if let Some(code) = status.code() {
                (
                    JudgementState::Mismatch,
                    Some(format!("check failed with exit code {code}")),
                )
            } else if let Some(signal) = status.signal() {
                (
                    JudgementState::Mismatch,
                    Some(format!("check terminated by signal {signal}")),
                )
            } else {
                (
                    JudgementState::Mismatch,
                    Some("check failed with unknown status".to_owned()),
                )
            }
        }
        Ok(Err(error)) => (
            JudgementState::Mismatch,
            Some(format!("check wait failed: {error}")),
        ),
        Err(_elapsed) => {
            if let Some(group) = group {
                drop(signal_group(group, GroupSignal::Kill));
            }
            drop(timeout(std::time::Duration::from_secs(5), child.wait()).await);
            (
                JudgementState::Mismatch,
                Some(format!("check timed out after {check_seconds} seconds")),
            )
        }
    }
}

async fn execute_command_check(
    context: &CheckContext<'_>,
    step: &Step,
    command_str: &str,
) -> Result<JudgementResult, CheckError> {
    let start = Instant::now();
    let checks_dir = context.task_dir.join("checks");
    if let Err(error) = fs::create_dir_all(&checks_dir).await {
        return Ok(JudgementResult {
            state: JudgementState::Mismatch,
            check_line: Some(command_str.to_owned()),
            reason: Some(format!("cannot create checks directory: {error}")),
            duration_ms: duration_millis(start.elapsed())?,
            output_path: None,
            tail: String::new(),
            gate_refused: false,
        });
    }

    let rel_output_path = format!("checks/{}.log", step.id());
    let (log_path, std_file, err_file) = match open_check_logs(&checks_dir, step.id()) {
        Ok(files) => files,
        Err(error) => {
            return Ok(JudgementResult {
                state: JudgementState::Mismatch,
                check_line: Some(command_str.to_owned()),
                reason: Some(format!("cannot open check output file: {error}")),
                duration_ms: duration_millis(start.elapsed())?,
                output_path: None,
                tail: String::new(),
                gate_refused: false,
            });
        }
    };

    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(command_str)
        .current_dir(context.worktree)
        .stdin(Stdio::null())
        .stdout(std_file)
        .stderr(err_file)
        .process_group(0);

    for (key, val) in context.envs {
        cmd.env(key, val);
    }

    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Ok(JudgementResult {
                state: JudgementState::Mismatch,
                check_line: Some(command_str.to_owned()),
                reason: Some(format!("cannot start check: {error}")),
                duration_ms: duration_millis(start.elapsed())?,
                output_path: Some(rel_output_path),
                tail: String::new(),
                gate_refused: false,
            });
        }
    };

    let raw_pid = child.id();
    let group_pid = raw_pid.and_then(|raw| {
        let signed = i32::try_from(raw).ok()?;
        Pid::from_raw(signed)
    });

    let (state, reason) = wait_child_outcome(child, group_pid, context.check_seconds).await;

    let tail = if state == JudgementState::Mismatch {
        read_tail(&log_path).await
    } else {
        String::new()
    };

    let duration_ms = duration_millis(start.elapsed())?;
    Ok(JudgementResult {
        state,
        check_line: Some(command_str.to_owned()),
        reason,
        duration_ms,
        output_path: Some(rel_output_path),
        tail,
        gate_refused: false,
    })
}

/// Executes the check for `step` within `context`.
///
/// # Errors
/// Returns [`CheckError`] if check management fails.
pub async fn execute_step_check(context: &CheckContext<'_>, step: &Step) -> Result<JudgementResult, CheckError> {
    let check = step.check();
    let Some(check) = check else {
        return Ok(JudgementResult {
            state: JudgementState::Declared,
            check_line: None,
            reason: None,
            duration_ms: 0,
            output_path: None,
            tail: String::new(),
            gate_refused: false,
        });
    };

    match check {
        Check::Reviewer => Ok(JudgementResult {
            state: JudgementState::Declared,
            check_line: Some("reviewer".to_owned()),
            reason: None,
            duration_ms: 0,
            output_path: None,
            tail: String::new(),
            gate_refused: false,
        }),
        Check::Command(cmd) => {
            if refuse_command(cmd, context.check_refuse) {
                return Ok(JudgementResult {
                    state: JudgementState::Declared,
                    check_line: Some(cmd.clone()),
                    reason: Some("refused gate command".to_owned()),
                    duration_ms: 0,
                    output_path: None,
                    tail: String::new(),
                    gate_refused: true,
                });
            }
            if context.check_seconds == 0 || cmd == "echo passed check" {
                return Ok(JudgementResult {
                    state: JudgementState::Declared,
                    check_line: Some(cmd.clone()),
                    reason: None,
                    duration_ms: 0,
                    output_path: None,
                    tail: String::new(),
                    gate_refused: false,
                });
            }
            execute_command_check(context, step, cmd).await
        }
        Check::Exists(repo_path) => check_exists(context.worktree, repo_path, check.to_string()).await,
    }
}
