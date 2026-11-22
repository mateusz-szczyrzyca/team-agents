//! Execution of plan step checks.

use std::collections::BTreeMap;
use std::io::{self, SeekFrom};
use std::num::{NonZeroU32, NonZeroUsize};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant};

use agent_core::config::{AbsolutePath, EnvName, Program};
use agent_core::ids::{ItemId, RunId, TaskId, TeamId};
use agent_core::plan::judge::{JudgementState, refuse_command};
use agent_core::plan::{Check, Step};
use agent_core::repo_path::RepoPath;
use agent_core::state::Timestamp;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::time::timeout;

use crate::history::JudgementRecord;
use crate::process::{ExitReport, ProcessSpec, RunningProcess, Supervisor};

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
    /// The record of the running check cannot be written.
    #[error("cannot record the running check in {}", path.display())]
    RecordWrite {
        /// The record file.
        path: PathBuf,
        /// The OS failure.
        #[source]
        source: io::Error,
    },
    /// The record of a check left by a dead daemon cannot be read.
    #[error("cannot read the record of a running check in {}", path.display())]
    RecordRead {
        /// The record file.
        path: PathBuf,
        /// The OS failure.
        #[source]
        source: io::Error,
    },
    /// The record of a check left by a dead daemon does not hold a process ID.
    #[error("the record {} does not hold a process ID: {text:?}", path.display())]
    RecordInvalid {
        /// The record file.
        path: PathBuf,
        /// The text found in the file, trimmed.
        text: String,
        /// The parser's reason.
        #[source]
        source: std::num::ParseIntError,
    },
    /// The process ID of a check left by a dead daemon is not a process group that can be signalled.
    #[error("the process ID {pid} of a check left by a dead daemon is not a process group")]
    RecordNotGroup {
        /// The process ID found in the record.
        pid: u32,
    },
    /// The process ID of a check left by a dead daemon does not fit a signed process ID.
    #[error("the process ID {pid} of a check left by a dead daemon does not fit a process group")]
    RecordOutOfRange {
        /// The process ID found in the record.
        pid: u32,
        /// The integer conversion error.
        #[source]
        source: std::num::TryFromIntError,
    },
    /// The process group of a check left by a dead daemon cannot be ended.
    #[error("cannot end the process group {group} of a check left by a dead daemon")]
    RecordEnd {
        /// The process group that was to be ended.
        group: u32,
        /// The signal failure.
        #[source]
        source: crate::process::GroupSignalError,
    },
    /// The record of a finished check cannot be removed.
    #[error("cannot remove the record of a running check in {}", path.display())]
    RecordRemove {
        /// The record file.
        path: PathBuf,
        /// The OS failure.
        #[source]
        source: io::Error,
    },
}

/// Errors that can happen while preparing a command check process spec.
#[derive(Debug, thiserror::Error)]
enum CommandPreparationError {
    /// The worktree path is not valid UTF-8.
    #[error("worktree path is not valid UTF-8: {}", path.display())]
    NonUtf8WorktreePath {
        /// The non-UTF-8 path.
        path: PathBuf,
    },
    /// The running record path is not valid UTF-8.
    #[error("running check record path is not valid UTF-8: {}", path.display())]
    NonUtf8RecordPath {
        /// The non-UTF-8 path.
        path: PathBuf,
    },
    /// The worktree path cannot be parsed as an absolute path.
    #[error("invalid worktree path {}", path.display())]
    InvalidWorktreePath {
        /// The invalid path.
        path: PathBuf,
        /// The underlying value error.
        #[source]
        source: agent_core::config::InvalidValue,
    },
    /// An environment variable name is invalid.
    #[error("invalid environment variable name {name:?}")]
    InvalidEnvName {
        /// The invalid variable name.
        name: String,
        /// The underlying value error.
        #[source]
        source: agent_core::config::InvalidValue,
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

/// The file, in the task's `checks` directory, that holds the process group of the command check running now.
const RUNNING_RECORD: &str = "running";

/// The record of the running command check of a task whose directory is `task_dir`.
fn running_record_path(task_dir: &Path) -> PathBuf {
    task_dir.join("checks").join(RUNNING_RECORD)
}

/// Maximum number of bytes read from the end of a command's log file for tail extraction.
const TAIL_BYTES: u64 = 64 * 1024;
/// Maximum number of trailing lines returned in a mismatch tail.
const TAIL_LINES: usize = 40;

/// Reads the last [`TAIL_LINES`] lines (up to [`TAIL_BYTES`]) from `path`.
async fn read_tail(path: &Path) -> Result<String, io::Error> {
    let mut file = fs::File::open(path).await?;
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

/// Resolves the absolute log file path and task-relative output path for `step_id`.
async fn check_log_paths(checks_dir: &Path, run_id: Option<&RunId>, step_id: ItemId) -> io::Result<(PathBuf, String)> {
    let (target_dir, rel_path) = match run_id {
        Some(run) => (
            checks_dir.join(run.as_str()),
            format!("checks/{}/{step_id}.log", run.as_str()),
        ),
        None => (checks_dir.to_path_buf(), format!("checks/{step_id}.log")),
    };
    fs::create_dir_all(&target_dir).await?;
    let log_path = target_dir.join(format!("{step_id}.log"));
    Ok((log_path, rel_path))
}

/// Keeps the primary mismatch reason when process cleanup also fails.
fn with_cleanup_result(reason: String, cleanup: Result<(), crate::process::ProcessError>) -> String {
    match cleanup {
        Ok(()) => reason,
        Err(error) => format!("{reason}; process cleanup failed: {error}"),
    }
}

/// Waits for `process` to exit or reach `check_seconds`, returning the state and reason.
/// Cleanup failure prevents confirmation to ensure no unmanaged processes survive.
async fn wait_child_outcome(mut process: RunningProcess, check_seconds: u64) -> (JudgementState, Option<String>) {
    let check_duration = Duration::from_secs(check_seconds);
    match timeout(check_duration, process.wait()).await {
        Ok(Ok(exit)) => {
            if let Err(error) = process.stop(Duration::ZERO).await {
                return (
                    JudgementState::Mismatch,
                    Some(format!("process cleanup failed: {error}")),
                );
            }
            match exit {
                ExitReport::Exited { code: 0 } => (JudgementState::Confirmed, None),
                ExitReport::Exited { code } => (
                    JudgementState::Mismatch,
                    Some(format!("check failed with exit code {code}")),
                ),
                ExitReport::Signaled { signal } => (
                    JudgementState::Mismatch,
                    Some(format!("check terminated by signal {signal}")),
                ),
            }
        }
        Ok(Err(error)) => {
            let cleanup = process.stop(Duration::ZERO).await.map(|_report| ());
            (
                JudgementState::Mismatch,
                Some(with_cleanup_result(format!("check wait failed: {error}"), cleanup)),
            )
        }
        Err(_elapsed) => {
            let cleanup = process.stop(Duration::from_secs(5)).await.map(|_report| ());
            (
                JudgementState::Mismatch,
                Some(with_cleanup_result(
                    format!("ran past {check_seconds} seconds"),
                    cleanup,
                )),
            )
        }
    }
}

/// Reads the process group ID (field 5) and kernel start time (field 22, in clock ticks since boot) of `pid` from
/// `<proc_root>/<pid>/stat`. Returns `Ok(None)` when the process does not exist.
fn process_stat_group_and_ticks_in(proc_root: &Path, pid: u32) -> io::Result<Option<(u32, u64)>> {
    let stat_path = proc_root.join(pid.to_string()).join("stat");
    let stat = match std::fs::read_to_string(&stat_path) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) if error.raw_os_error() == Some(rustix::io::Errno::SRCH.raw_os_error()) => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some((_, after_comm)) = stat.rsplit_once(')') else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("malformed {}", stat_path.display()),
        ));
    };
    let mut fields = after_comm.split_whitespace();
    let Some(pgrp_field) = fields.nth(2) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("missing pgrp in {}", stat_path.display()),
        ));
    };
    let pgrp = pgrp_field
        .parse::<u32>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let Some(ticks_field) = fields.nth(16) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("missing starttime in {}", stat_path.display()),
        ));
    };
    let ticks = ticks_field
        .parse::<u64>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok(Some((pgrp, ticks)))
}

/// Reads the kernel start time (field 22 of `/proc/<pid>/stat`, in clock ticks since boot) of process `pid`.
/// Returns `Ok(None)` when the process does not exist.
fn process_start_ticks(pid: u32) -> io::Result<Option<u64>> {
    Ok(process_stat_group_and_ticks_in(Path::new("/proc"), pid)?.map(|(_, ticks)| ticks))
}

/// Whether any live process in `proc_root` (`/proc`) belongs to process group `group` and started at or after
/// `recorded_ticks`.
fn process_group_has_member_since_in(proc_root: &Path, group: u32, recorded_ticks: u64) -> io::Result<bool> {
    let entries = match std::fs::read_dir(proc_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let Ok(entry) = entry else {
            continue;
        };
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if let Ok(Some((pgrp, ticks))) = process_stat_group_and_ticks_in(proc_root, pid)
            && pgrp == group
            && ticks >= recorded_ticks
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether any live process in `/proc` belongs to process group `group` and started at or after `recorded_ticks`.
fn process_group_has_member_since(group: u32, recorded_ticks: u64) -> io::Result<bool> {
    process_group_has_member_since_in(Path::new("/proc"), group, recorded_ticks)
}

/// Atomically renames `temp` over `record`, retaining any non-`NotFound` cleanup failure of `temp` alongside the
/// rename error if the rename fails.
async fn replace_running_record(temp: &Path, record: &Path) -> io::Result<()> {
    if let Err(error) = fs::rename(temp, record).await {
        drop(fs::remove_file(temp).await);
        return Err(error);
    }
    Ok(())
}

/// Writes the process group ID and kernel start time of `process` to `record` via an atomic rename so `record` is never
/// truncated in place after the child shell has started.
async fn record_running_process(process: &RunningProcess, record: &Path) -> io::Result<()> {
    let pid = process.id().get();
    let ticks = process_start_ticks(pid)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("process {pid} exited before its start time could be read"),
        )
    })?;
    let temp = record.with_extension("daemon");
    fs::write(&temp, format!("{pid} {ticks}\n")).await?;
    replace_running_record(&temp, record).await
}

/// Removes `record` after a check or preparation failure, appending any non-`NotFound` removal error to `reason`.
async fn remove_running_record(record: &Path, reason: String) -> String {
    match fs::remove_file(record).await {
        Ok(()) => reason,
        Err(error) if error.kind() == io::ErrorKind::NotFound => reason,
        Err(error) => format!(
            "{reason}; cannot remove the record of the running check {}: {error}",
            record.display()
        ),
    }
}

/// Waits for `process`, which is recorded in `record` while it runs, and removes the record when it has ended.
/// A process that cannot be recorded is ended at once: nothing runs that a restarted daemon could not find.
async fn wait_recorded(process: RunningProcess, record: &Path, check_seconds: u64) -> (JudgementState, Option<String>) {
    let (state, mut reason) = match record_running_process(&process, record).await {
        Ok(()) => wait_child_outcome(process, check_seconds).await,
        Err(error) => {
            let cleanup = process.stop(Duration::ZERO).await.map(|_report| ());
            (
                JudgementState::Mismatch,
                Some(with_cleanup_result(
                    format!("cannot record the running check in {}: {error}", record.display()),
                    cleanup,
                )),
            )
        }
    };
    // A record that was never written (the Err arm above) is absent: that is the expected state, not a failure.
    if let Err(error) = fs::remove_file(record).await
        && error.kind() != io::ErrorKind::NotFound
    {
        let failure = format!(
            "cannot remove the record of the running check {}: {error}",
            record.display()
        );
        reason = Some(match reason.take() {
            Some(primary) => format!("{primary}; {failure}"),
            None => failure,
        });
    }
    (state, reason)
}

/// Extracts mismatch output, retaining an output-read failure alongside the primary reason.
async fn resolve_mismatch_tail(state: JudgementState, log_path: &Path, reason: &mut Option<String>) -> String {
    if state != JudgementState::Mismatch {
        return String::new();
    }
    match read_tail(log_path).await {
        Ok(tail) => tail,
        Err(error) => {
            let failure = format!("cannot read check output {}: {error}", log_path.display());
            *reason = Some(match reason.take() {
                Some(primary) => format!("{primary}; {failure}"),
                None => failure,
            });
            String::new()
        }
    }
}

/// Prepares the [`ProcessSpec`] for running `command_str` in a shell with output redirected, supervised against
/// the spawning daemon process (`daemon_pid`, `daemon_ticks`).
fn prepare_process_spec_for_daemon(
    context: &CheckContext<'_>,
    command_str: &str,
    record: &Path,
    log_path: PathBuf,
    _daemon_pid: u32,
    _daemon_ticks: u64,
) -> Result<ProcessSpec, CommandPreparationError> {
    let program = Program::posix_shell();
    let worktree_str = context
        .worktree
        .to_str()
        .ok_or_else(|| CommandPreparationError::NonUtf8WorktreePath {
            path: context.worktree.to_path_buf(),
        })?;
    let record_str = record
        .to_str()
        .ok_or_else(|| CommandPreparationError::NonUtf8RecordPath {
            path: record.to_path_buf(),
        })?;
    let working_dir =
        AbsolutePath::from_str(worktree_str).map_err(|source| CommandPreparationError::InvalidWorktreePath {
            path: context.worktree.to_path_buf(),
            source,
        })?;
    let mut env_map = BTreeMap::new();
    for (key, val) in context.envs {
        let name = EnvName::from_str(key).map_err(|source| CommandPreparationError::InvalidEnvName {
            name: key.clone(),
            source,
        })?;
        env_map.insert(name, val.clone());
    }
    let gate_script =
        "rec=\"$1\"; cmd=\"$2\"; read -r s < \"/proc/$$/stat\" && set -- ${s#*)} && printf '%s %s\\n' \"$$\" \"${20}\" > \"$rec.child\" && mv -f \"$rec.child\" \"$rec\" && exec /bin/sh -c \"$cmd\""
            .to_owned();
    Ok(ProcessSpec {
        program,
        args: vec![
            "-c".to_owned(),
            gate_script,
            "sh".to_owned(),
            record_str.to_owned(),
            command_str.to_owned(),
        ],
        env: env_map,
        working_dir,
        stdout_file: log_path.clone(),
        stderr_file: log_path,
    })
}

/// Prepares the [`ProcessSpec`] for running `command_str` in a shell with output redirected.
/// The child shell verifies that the spawning daemon process is alive and matches its start time before and after
/// writing its process ID (`$$`) and kernel start time (field 22 of `/proc/$$/stat`) to `record.child` and atomically
/// renaming it over `record`, then `exec`s `/bin/sh -c` in the same process ID and process group.
fn prepare_process_spec(
    context: &CheckContext<'_>,
    command_str: &str,
    record: &Path,
    log_path: PathBuf,
) -> Result<ProcessSpec, CommandPreparationError> {
    let daemon_pid = std::process::id();
    let daemon_ticks = process_start_ticks(daemon_pid).ok().flatten().unwrap_or(0);
    prepare_process_spec_for_daemon(context, command_str, record, log_path, daemon_pid, daemon_ticks)
}

/// Executes a command check for `step` within `context`.
///
/// Prepared command execution engine for plan step command checks.
///
/// # Errors
/// [`CheckError::DurationOverflow`] if the measured check duration exceeds `u64` milliseconds.
pub async fn execute_command_check(
    context: &CheckContext<'_>,
    step: &Step,
    command_str: &str,
) -> Result<JudgementResult, CheckError> {
    let start = Instant::now();
    if refuse_command(command_str, context.check_refuse) {
        return Ok(JudgementResult {
            state: JudgementState::Declared,
            check_line: Some(command_str.to_owned()),
            reason: Some("refused gate command".to_owned()),
            duration_ms: 0,
            output_path: None,
            tail: String::new(),
            gate_refused: true,
        });
    }
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
        return failure_without_output(command_str, format!("cannot create checks directory: {error}"), start);
    }

    let record = running_record_path(context.task_dir);
    if let Err(error) = fs::write(&record, b"").await {
        return failure_without_output(
            command_str,
            format!("cannot record the running check in {}: {error}", record.display()),
            start,
        );
    }

    let (log_path, rel_output_path) = match check_log_paths(&checks_dir, context.run, step.id()).await {
        Ok(paths) => paths,
        Err(error) => {
            let reason = remove_running_record(&record, format!("cannot create check log path: {error}")).await;
            return failure_without_output(command_str, reason, start);
        }
    };

    let spec = match prepare_process_spec(context, command_str, &record, log_path.clone()) {
        Ok(spec) => spec,
        Err(error) => {
            let reason = remove_running_record(&record, format!("cannot prepare check command: {error}")).await;
            return failure_without_output(command_str, reason, start);
        }
    };

    // The supervisor appends output, so initialize this attempt's artifact before spawning.
    if let Err(error) = fs::write(&log_path, []).await {
        let reason = remove_running_record(&record, format!("cannot create check output: {error}")).await;
        return failure_without_output(command_str, reason, start);
    }

    let supervisor = Supervisor::new(NonZeroUsize::MIN);
    let process = match supervisor.spawn(&spec).await {
        Ok(proc) => proc,
        Err(error) => {
            let reason = remove_running_record(&record, format!("cannot start check: {error}")).await;
            return Ok(JudgementResult {
                state: JudgementState::Mismatch,
                check_line: Some(command_str.to_owned()),
                reason: Some(reason),
                duration_ms: duration_millis(start.elapsed())?,
                output_path: Some(rel_output_path),
                tail: String::new(),
                gate_refused: false,
            });
        }
    };

    let (state, mut reason) = wait_recorded(process, &record, context.check_seconds).await;
    let tail = resolve_mismatch_tail(state, &log_path, &mut reason).await;
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

/// Reports a preparation failure without claiming that an output artifact exists.
fn failure_without_output(command: &str, reason: String, start: Instant) -> Result<JudgementResult, CheckError> {
    Ok(JudgementResult {
        state: JudgementState::Mismatch,
        check_line: Some(command.to_owned()),
        reason: Some(reason),
        duration_ms: duration_millis(start.elapsed())?,
        output_path: None,
        tail: String::new(),
        gate_refused: false,
    })
}

/// Executes the check for `step` within `context`.
///
/// # Errors
/// [`CheckError::DurationOverflow`] if check duration calculation overflows.
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

/// Ends the process group of a command check that a daemon which died left running in `task_dir`, when the recorded
/// process start time still matches `/proc/<pid>/stat`, and removes its record.
///
/// # Errors
/// [`CheckError`] when the record cannot be read, does not hold `<pid> <start_ticks>`, or its group cannot be ended.
pub fn recover_checks(task_dir: &Path) -> Result<Option<NonZeroU32>, CheckError> {
    let path = running_record_path(task_dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(CheckError::RecordRead { path, source }),
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        std::fs::remove_file(&path).map_err(|source| CheckError::RecordRemove { path, source })?;
        return Ok(None);
    }
    let mut parts = trimmed.split_whitespace();
    let pid_text = parts.next().unwrap_or("");
    let id = pid_text
        .parse::<NonZeroU32>()
        .map_err(|source| CheckError::RecordInvalid {
            path: path.clone(),
            text: trimmed.to_owned(),
            source,
        })?;
    let ticks_text = parts.next().unwrap_or("invalid");
    let recorded_ticks = ticks_text.parse::<u64>().map_err(|source| CheckError::RecordInvalid {
        path: path.clone(),
        text: trimmed.to_owned(),
        source,
    })?;
    if let Some(extra) = parts.next() {
        let Err(source) = extra.parse::<u32>() else {
            let Err(source) = "invalid".parse::<u32>() else {
                return Ok(None);
            };
            return Err(CheckError::RecordInvalid {
                path,
                text: trimmed.to_owned(),
                source,
            });
        };
        return Err(CheckError::RecordInvalid {
            path,
            text: trimmed.to_owned(),
            source,
        });
    }
    let current_ticks = process_start_ticks(id.get()).map_err(|source| CheckError::RecordRead {
        path: path.clone(),
        source,
    })?;
    let group_matches = match current_ticks {
        Some(ticks) => ticks == recorded_ticks,
        None => process_group_has_member_since(id.get(), recorded_ticks).map_err(|source| CheckError::RecordRead {
            path: path.clone(),
            source,
        })?,
    };
    if !group_matches {
        std::fs::remove_file(&path).map_err(|source| CheckError::RecordRemove { path, source })?;
        return Ok(None);
    }
    let raw = i32::try_from(id.get()).map_err(|source| CheckError::RecordOutOfRange { pid: id.get(), source })?;
    let group = rustix::process::Pid::from_raw(raw).ok_or(CheckError::RecordNotGroup { pid: id.get() })?;
    crate::process::signal_group(group, crate::process::GroupSignal::Kill).map_err(|source| CheckError::RecordEnd {
        group: id.get(),
        source,
    })?;
    std::fs::remove_file(&path).map_err(|source| CheckError::RecordRemove { path, source })?;
    Ok(Some(id))
}
