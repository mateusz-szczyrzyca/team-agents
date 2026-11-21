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
const TAIL_LINES: usize = 40;

async fn read_tail(path: &Path) -> Result<String, io::Error> {
    let mut file = match fs::File::open(path).await {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(String::new()),
        Err(err) => return Err(err),
    };
    let metadata = file.metadata().await?;
    let length = metadata.len();
    let start = length.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).await?;
    let mut bytes = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut bytes).await?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    let keep = lines.len().saturating_sub(TAIL_LINES);
    Ok(lines.into_iter().skip(keep).collect::<Vec<_>>().join("\n"))
}

async fn open_check_logs(
    checks_dir: &Path,
    run_id: Option<&RunId>,
    step_id: ItemId,
) -> io::Result<(std::path::PathBuf, String, std::fs::File, std::fs::File)> {
    let (target_dir, rel_path) = match run_id {
        Some(run) => (
            checks_dir.join(run.as_str()),
            format!("checks/{}/{step_id}.log", run.as_str()),
        ),
        None => (checks_dir.to_path_buf(), format!("checks/{step_id}.log")),
    };
    fs::create_dir_all(&target_dir).await?;
    let log_path = target_dir.join(format!("{step_id}.log"));
    let std_file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .await?
        .into_std()
        .await;
    let err_file = std_file.try_clone()?;
    Ok((log_path, rel_path, std_file, err_file))
}

/// Guard ensuring a process group is terminated when dropped prematurely.
struct GroupGuard {
    group: Pid,
    active: bool,
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        if self.active
            && let Err(error) = signal_group(self.group, GroupSignal::Kill)
        {
            tracing::warn!(%error, "cannot kill process group on check guard drop");
        }
    }
}

async fn wait_child_outcome(
    mut child: tokio::process::Child,
    group: Pid,
    check_seconds: u64,
) -> (JudgementState, Option<String>) {
    let mut guard = GroupGuard { group, active: true };
    let check_duration = std::time::Duration::from_secs(check_seconds);
    match timeout(check_duration, child.wait()).await {
        Ok(Ok(status)) => {
            guard.active = false;
            if let Err(error) = signal_group(group, GroupSignal::Kill) {
                tracing::warn!(%error, "cannot kill process group after check completion");
            }
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
        Ok(Err(error)) => {
            guard.active = false;
            if let Err(err) = signal_group(group, GroupSignal::Kill) {
                tracing::warn!(%err, "cannot kill process group after check wait error");
            }
            (JudgementState::Mismatch, Some(format!("check wait failed: {error}")))
        }
        Err(_elapsed) => {
            guard.active = false;
            if let Err(error) = signal_group(group, GroupSignal::Kill) {
                tracing::warn!(%error, "cannot kill process group on check timeout");
            }
            match timeout(std::time::Duration::from_secs(5), child.wait()).await {
                Ok(Ok(_)) => {}
                Ok(Err(err)) => tracing::warn!(%err, "error reaping timed out check process"),
                Err(_) => tracing::warn!("reaping timed out check process exceeded grace"),
            }
            (
                JudgementState::Mismatch,
                Some(format!("ran past {check_seconds} seconds")),
            )
        }
    }
}

async fn resolve_mismatch_tail(state: JudgementState, log_path: &Path) -> String {
    if state == JudgementState::Mismatch {
        match read_tail(log_path).await {
            Ok(tail) => tail,
            Err(error) => {
                tracing::warn!(path = %log_path.display(), %error, "cannot read check output tail");
                String::new()
            }
        }
    } else {
        String::new()
    }
}

/// Executes a command check for `step` within `context`.
///
/// Prepared command execution engine for plan step command checks.
///
/// # Errors
/// Returns [`CheckError`] if check execution management fails.
pub async fn execute_command_check(
    context: &CheckContext<'_>,
    step: &Step,
    command_str: &str,
) -> Result<JudgementResult, CheckError> {
    let start = Instant::now();
    if context.check_seconds == 0 {
        return Ok(JudgementResult {
            state: JudgementState::Declared,
            check_line: Some(command_str.to_owned()),
            reason: None,
            duration_ms: 0,
            output_path: None,
            tail: String::new(),
            gate_refused: false,
        });
    }
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

    let (log_path, rel_output_path, std_file, err_file) =
        match open_check_logs(&checks_dir, context.run, step.id()).await {
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
        .process_group(0)
        .kill_on_drop(true);

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

    let (_id, group_pid) = match crate::process::leader_of(&child) {
        Ok(ids) => ids,
        Err(error) => {
            return Ok(JudgementResult {
                state: JudgementState::Mismatch,
                check_line: Some(command_str.to_owned()),
                reason: Some(format!("cannot identify check process leader: {error}")),
                duration_ms: duration_millis(start.elapsed())?,
                output_path: Some(rel_output_path),
                tail: String::new(),
                gate_refused: false,
            });
        }
    };

    let (state, reason) = wait_child_outcome(child, group_pid, context.check_seconds).await;

    let tail = resolve_mismatch_tail(state, &log_path).await;

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
            let is_refused = refuse_command(cmd, context.check_refuse);
            Ok(JudgementResult {
                state: JudgementState::Declared,
                check_line: Some(cmd.clone()),
                reason: if is_refused {
                    Some("refused gate command".to_owned())
                } else {
                    None
                },
                duration_ms: 0,
                output_path: None,
                tail: String::new(),
                gate_refused: is_refused,
            })
        }
        Check::Exists(repo_path) => check_exists(context.worktree, repo_path, check.to_string()).await,
    }
}
