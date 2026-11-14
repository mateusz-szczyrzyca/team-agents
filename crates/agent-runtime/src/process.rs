//! Processes: the supervisor that runs a model CLI and the bounded capture that runs a short helper (`git`, the
//! notification command).
//!
//! Every process is started from an argument vector — never a shell string (SEC-3) — in its own process group, so
//! that a stop reaches the children it started too. A supervised run writes its output to files and is bounded: the
//! number of runs at once, the grace of a stop, the wait for a killed process. A captured run is bounded in time and
//! in the bytes it may return.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::num::{NonZeroU32, NonZeroUsize, TryFromIntError};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use agent_core::config::{AbsolutePath, EnvName, Program};
use rustix::process::{Pid, Signal};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};
use tokio::time::timeout;

/// How long a process that was sent `SIGKILL` may take to be reaped before the stop gives up.
const KILL_WAIT: Duration = Duration::from_secs(10);

/// How long a start refused because the program is busy (`ETXTBSY`) is tried again.
///
/// A program is busy while a file descriptor writing it is open. In a multithreaded process, a child forked by another
/// thread while a program was being written holds such a descriptor until its own `exec`: the program is busy for that
/// moment only. A program that stays open for writing longer is a real error.
const BUSY_RETRY_FOR: Duration = Duration::from_secs(2);
/// The pause between two tries of a start refused because the program is busy.
const BUSY_RETRY_PAUSE: Duration = Duration::from_millis(10);

/// A signal to a whole process group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
pub(crate) enum GroupSignal {
    /// `SIGTERM`: asks the processes to end.
    #[strum(serialize = "SIGTERM")]
    Terminate,
    /// `SIGKILL`: ends them; it cannot be caught.
    #[strum(serialize = "SIGKILL")]
    Kill,
}

/// Why a process has no usable process ID.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PidError {
    /// The child was already reaped, so it has no ID any more.
    #[error("the process has already been reaped")]
    Reaped,
    /// The process has the ID 0, which no process has.
    #[error("the process has the ID 0")]
    Zero,
    /// The operating system gave an ID that does not fit a `pid_t`.
    #[error("the process ID {pid} does not fit a process ID")]
    OutOfRange {
        /// The ID.
        pid: u32,
        /// Why it does not fit.
        source: TryFromIntError,
    },
}

/// A signal could not be sent to a process group.
#[derive(Debug, thiserror::Error)]
#[error("cannot send {signal} to the process group {group}")]
pub struct GroupSignalError {
    /// The signal.
    pub(crate) signal: GroupSignal,
    /// The group: the process ID of its leader.
    pub(crate) group: i32,
    /// What the operating system said.
    #[source]
    pub(crate) source: io::Error,
}

/// A wait status that is neither an exit code nor a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the process ended with the unrecognised wait status {raw}")]
pub struct UnknownExit {
    /// The raw wait status.
    pub raw: i32,
}

/// How a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReport {
    /// It returned this exit code.
    Exited {
        /// The code.
        code: i32,
    },
    /// A signal ended it.
    Signaled {
        /// The signal number.
        signal: i32,
    },
}

impl ExitReport {
    /// Reads a wait status.
    ///
    /// # Errors
    /// [`UnknownExit`] for a status with neither a code nor a signal.
    pub fn from_status(status: ExitStatus) -> Result<Self, UnknownExit> {
        if let Some(code) = status.code() {
            Ok(Self::Exited { code })
        } else if let Some(signal) = status.signal() {
            Ok(Self::Signaled { signal })
        } else {
            Err(UnknownExit { raw: status.into_raw() })
        }
    }

    /// Whether the process returned the code 0.
    #[must_use]
    pub const fn succeeded(self) -> bool {
        matches!(self, Self::Exited { code: 0 })
    }
}

/// What to run, where, and where its output goes.
#[derive(Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    /// The program: a bare name looked up on `PATH`, or an absolute path.
    pub program: Program,
    /// Its arguments, each one argument: never joined, never through a shell.
    pub args: Vec<String>,
    /// Variables added to the environment the supervisor itself runs in. They may hold secrets: `Debug` shows only
    /// the names.
    pub env: BTreeMap<EnvName, String>,
    /// The directory to run in.
    pub working_dir: AbsolutePath,
    /// The file the standard output is appended to; created, with its directories, when missing.
    pub stdout_file: PathBuf,
    /// The file the standard error is appended to; created, with its directories, when missing.
    pub stderr_file: PathBuf,
}

impl fmt::Debug for ProcessSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The values of the environment may be secrets and the arguments may hold a whole prompt: only their names
        // and count are shown.
        f.debug_struct("ProcessSpec")
            .field("program", &self.program)
            .field("args", &self.args.len())
            .field("env", &self.env.keys().collect::<Vec<_>>())
            .field("working_dir", &self.working_dir)
            .field("stdout_file", &self.stdout_file)
            .field("stderr_file", &self.stderr_file)
            .finish()
    }
}

/// Why a process could not be run, watched or stopped.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    /// The supervisor already runs as many processes as it may.
    #[error("{limit} processes are already running: the limit is reached")]
    Limit {
        /// The most processes the supervisor runs at once.
        limit: usize,
        /// Why no slot was free.
        source: TryAcquireError,
    },
    /// The directory of an output file cannot be created.
    #[error("cannot create the directory {path:?}")]
    CreateDir {
        /// The directory.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// An output file cannot be opened.
    #[error("cannot open the output file {path:?}")]
    OpenOutput {
        /// The file.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The program cannot be started.
    #[error("cannot start {program:?}")]
    Spawn {
        /// The program.
        program: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
    /// The started process has no usable ID.
    #[error(transparent)]
    Pid(#[from] PidError),
    /// Waiting for the process failed.
    #[error("cannot wait for the process")]
    Wait {
        /// What the operating system said.
        source: io::Error,
    },
    /// The process ended with a status that is neither an exit code nor a signal.
    #[error(transparent)]
    UnknownExit(#[from] UnknownExit),
    /// A signal could not be sent to the process group.
    #[error(transparent)]
    Signal(#[from] GroupSignalError),
    /// The process did not end, even after `SIGKILL`.
    #[error("the process did not end within {limit:?} of SIGKILL")]
    StopTimeout {
        /// How long it was waited for.
        limit: Duration,
    },
    /// The modification time of an output file cannot be read.
    #[error("cannot read the modification time of {path:?}")]
    Activity {
        /// The output file.
        path: PathBuf,
        /// What the operating system said.
        source: io::Error,
    },
}

/// How a stop went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopHow {
    /// The process had already ended; only what it left behind in its group was killed.
    AlreadyExited,
    /// It ended within the grace after `SIGTERM`.
    Terminated,
    /// It did not end within the grace and was sent `SIGKILL`.
    Killed,
}

/// The result of a stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopReport {
    /// How the process ended.
    pub exit: ExitReport,
    /// How the stop went.
    pub how: StopHow,
}

/// Runs processes, at most a fixed number at once.
#[derive(Debug, Clone)]
pub struct Supervisor {
    /// One permit per process that may run; a running process holds one.
    slots: Arc<Semaphore>,
    /// The number of permits.
    limit: NonZeroUsize,
}

/// Whether the leader of a process has ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lifecycle {
    /// It may still run; its group is killed when the handle is dropped.
    Running,
    /// It ended and was reaped, this way.
    Exited(ExitReport),
}

/// A process the supervisor started: the leader of its own process group.
///
/// Dropping it while it runs kills its group. [`RunningProcess::stop`] is the orderly end.
#[derive(Debug)]
pub struct RunningProcess {
    /// The leader.
    child: Child,
    /// Its ID, which is also its group's.
    id: NonZeroU32,
    /// Its process group.
    group: Pid,
    /// Whether the leader is known to have ended.
    lifecycle: Lifecycle,
    /// The standard output file.
    stdout_file: PathBuf,
    /// The standard error file.
    stderr_file: PathBuf,
    /// When it was started: the activity of a process that has not written yet.
    started: SystemTime,
    /// The slot of the supervisor, given back when the process is dropped.
    _slot: OwnedSemaphorePermit,
}

impl Supervisor {
    /// A supervisor that runs at most `limit` processes at once.
    #[must_use]
    pub fn new(limit: NonZeroUsize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(limit.get())),
            limit,
        }
    }

    /// Starts `spec` in a new process group, its output going to its files.
    ///
    /// # Errors
    /// [`ProcessError::Limit`] when the limit of running processes is reached (the caller decides whether to wait);
    /// [`ProcessError::CreateDir`], [`ProcessError::OpenOutput`], [`ProcessError::Spawn`] and [`ProcessError::Pid`].
    pub async fn spawn(&self, spec: &ProcessSpec) -> Result<RunningProcess, ProcessError> {
        let slot = Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|source| ProcessError::Limit {
                limit: self.limit.get(),
                source,
            })?;
        let stdout = open_output(&spec.stdout_file).await?;
        let stderr = open_output(&spec.stderr_file).await?;
        let mut command = Command::new(spec.program.as_path());
        command
            .args(&spec.args)
            .envs(spec.env.iter().map(|(name, value)| (name.as_str(), value)))
            .current_dir(spec.working_dir.as_path())
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .process_group(0)
            // Last line of defence: the leader does not outlive a handle that was dropped on an error path.
            .kill_on_drop(true);
        let child = spawn_when_free(&mut command)
            .await
            .map_err(|source| ProcessError::Spawn {
                program: spec.program.as_path().to_path_buf(),
                source,
            })?;
        let (id, group) = leader_of(&child)?;
        Ok(RunningProcess {
            child,
            id,
            group,
            lifecycle: Lifecycle::Running,
            stdout_file: spec.stdout_file.clone(),
            stderr_file: spec.stderr_file.clone(),
            started: SystemTime::now(),
            _slot: slot,
        })
    }
}

/// Starts `command`, trying again for [`BUSY_RETRY_FOR`] while the program is busy (`ETXTBSY`).
///
/// # Errors
/// What the operating system said about the last try.
async fn spawn_when_free(command: &mut Command) -> io::Result<Child> {
    let deadline = tokio::time::Instant::now() + BUSY_RETRY_FOR;
    loop {
        match command.spawn() {
            Err(error)
                if error.kind() == io::ErrorKind::ExecutableFileBusy && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(BUSY_RETRY_PAUSE).await;
            }
            started => return started,
        }
    }
}

/// Opens an output file for appending, creating it and its directories.
async fn open_output(path: &Path) -> Result<std::fs::File, ProcessError> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|source| ProcessError::CreateDir {
                path: dir.to_path_buf(),
                source,
            })?;
    }
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|source| ProcessError::OpenOutput {
            path: path.to_path_buf(),
            source,
        })?;
    // The child gets the plain file: tokio's wrapper would only add a buffer between them.
    Ok(file.into_std().await)
}

/// The ID of the leader `child` and its process group (the same number: the leader made the group).
fn leader_of(child: &Child) -> Result<(NonZeroU32, Pid), PidError> {
    let raw = child.id().ok_or(PidError::Reaped)?;
    let signed = i32::try_from(raw).map_err(|source| PidError::OutOfRange { pid: raw, source })?;
    let group = Pid::from_raw(signed).ok_or(PidError::Zero)?;
    let id = NonZeroU32::new(raw).ok_or(PidError::Zero)?;
    Ok((id, group))
}

/// Sends `signal` to every process of `group`.
///
/// A group that no longer exists is not an error: every process of it has ended, which is what the caller wants.
pub(crate) fn signal_group(group: Pid, signal: GroupSignal) -> Result<(), GroupSignalError> {
    let raw = match signal {
        GroupSignal::Terminate => Signal::TERM,
        GroupSignal::Kill => Signal::KILL,
    };
    match rustix::process::kill_process_group(group, raw) {
        Ok(()) => Ok(()),
        Err(errno) if errno == rustix::io::Errno::SRCH => Ok(()),
        Err(errno) => Err(GroupSignalError {
            signal,
            group: group.as_raw_nonzero().get(),
            source: errno.into(),
        }),
    }
}

/// The error and its causes on one line, for a log record or a message to the human.
#[must_use]
pub fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

impl RunningProcess {
    /// The process ID of the leader, which is also the ID of its group.
    #[must_use]
    pub const fn id(&self) -> NonZeroU32 {
        self.id
    }

    /// Waits for the leader to end. Cancel safe; it can be called again after it returned.
    ///
    /// Processes the leader started may outlive it: [`RunningProcess::stop`] ends them.
    ///
    /// # Errors
    /// [`ProcessError::Wait`] and [`ProcessError::UnknownExit`].
    pub async fn wait(&mut self) -> Result<ExitReport, ProcessError> {
        if let Lifecycle::Exited(exit) = self.lifecycle {
            return Ok(exit);
        }
        let status = self
            .child
            .wait()
            .await
            .map_err(|source| ProcessError::Wait { source })?;
        let exit = ExitReport::from_status(status)?;
        self.lifecycle = Lifecycle::Exited(exit);
        Ok(exit)
    }

    /// The last time the process showed life: the later modification time of its two output files, or its start.
    ///
    /// # Errors
    /// [`ProcessError::Activity`] when an output file cannot be inspected.
    pub async fn last_activity(&self) -> Result<SystemTime, ProcessError> {
        let mut latest = self.started;
        for path in [&self.stdout_file, &self.stderr_file] {
            let modified = tokio::fs::metadata(path)
                .await
                .and_then(|metadata| metadata.modified())
                .map_err(|source| ProcessError::Activity {
                    path: path.clone(),
                    source,
                })?;
            latest = latest.max(modified);
        }
        Ok(latest)
    }

    /// Ends the whole process group: `SIGTERM`, then `SIGKILL` after `grace` — and always `SIGKILL` for whatever is
    /// left of the group once the leader is gone.
    ///
    /// The leader is reaped before the last `SIGKILL`: a process group ID can only be reused once no process is left
    /// in the group, so the signal reaches the children of this leader or nobody.
    ///
    /// # Errors
    /// [`ProcessError::Signal`], [`ProcessError::Wait`], [`ProcessError::UnknownExit`] and
    /// [`ProcessError::StopTimeout`]. The group is killed when the handle is dropped after an error.
    pub async fn stop(mut self, grace: Duration) -> Result<StopReport, ProcessError> {
        let (exit, how) = match self.lifecycle {
            Lifecycle::Exited(exit) => (exit, StopHow::AlreadyExited),
            Lifecycle::Running => {
                signal_group(self.group, GroupSignal::Terminate)?;
                let ended = timeout(grace, self.child.wait()).await;
                match ended {
                    Ok(status) => {
                        let status = status.map_err(|source| ProcessError::Wait { source })?;
                        (ExitReport::from_status(status)?, StopHow::Terminated)
                    }
                    // The grace is over and the leader is still there: the case `SIGKILL` is for.
                    Err(_grace_over) => {
                        signal_group(self.group, GroupSignal::Kill)?;
                        (self.reap_killed().await?, StopHow::Killed)
                    }
                }
            }
        };
        self.lifecycle = Lifecycle::Exited(exit);
        signal_group(self.group, GroupSignal::Kill)?;
        Ok(StopReport { exit, how })
    }

    /// Waits, for a bounded time, for the leader that was sent `SIGKILL`.
    async fn reap_killed(&mut self) -> Result<ExitReport, ProcessError> {
        let reaped = timeout(KILL_WAIT, self.child.wait()).await;
        match reaped {
            Ok(status) => {
                let status = status.map_err(|source| ProcessError::Wait { source })?;
                Ok(ExitReport::from_status(status)?)
            }
            // `SIGKILL` cannot be caught: a process that is still not reaped is stuck in the kernel.
            Err(_wait_over) => Err(ProcessError::StopTimeout { limit: KILL_WAIT }),
        }
    }
}

impl Drop for RunningProcess {
    fn drop(&mut self) {
        // `Drop` cannot return an error, so a failure is logged: the harness is already giving the process up.
        if self.lifecycle == Lifecycle::Running
            && let Err(error) = signal_group(self.group, GroupSignal::Kill)
        {
            tracing::warn!(error = %error_chain(&error), "cannot kill the group of a dropped process");
        }
    }
}

/// The bounds of a captured run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CaptureLimits {
    /// How long the process may run before its group is killed.
    pub(crate) timeout: Duration,
    /// How many bytes of each stream are kept; the rest is read and dropped.
    pub(crate) max_bytes: u64,
}

/// One captured stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stream {
    /// The first bytes, at most [`CaptureLimits::max_bytes`].
    pub(crate) bytes: Vec<u8>,
    /// How many bytes came after them and were dropped; 0 when `bytes` is all of it.
    pub(crate) dropped: u64,
}

/// The result of a captured run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Captured {
    /// How the process ended.
    pub(crate) exit: ExitReport,
    /// What it wrote to the standard output.
    pub(crate) stdout: Stream,
    /// What it wrote to the standard error.
    pub(crate) stderr: Stream,
}

/// Why a captured run failed.
#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    /// The program cannot be started.
    #[error("cannot start the program")]
    Spawn {
        /// What the operating system said.
        source: io::Error,
    },
    /// The process has no usable ID.
    #[error(transparent)]
    Pid(#[from] PidError),
    /// A pipe of the process is missing.
    #[error("the process has no {pipe} pipe")]
    MissingPipe {
        /// Which pipe.
        pipe: &'static str,
    },
    /// Reading its output or waiting for it failed.
    #[error("cannot read the output of the process or wait for it")]
    Io {
        /// What the operating system said.
        source: io::Error,
    },
    /// The process ended with a status that is neither an exit code nor a signal.
    #[error(transparent)]
    UnknownExit(#[from] UnknownExit),
    /// The process ran over its time and its group was killed.
    #[error("the process did not end within {limit:?}")]
    Timeout {
        /// The time it had.
        limit: Duration,
    },
    /// A signal could not be sent to the process group.
    #[error(transparent)]
    Signal(#[from] GroupSignalError),
}

/// Reads `reader` to its end, keeping the first `max_bytes`: the rest is read and dropped so that the writer is
/// never blocked on a full pipe.
async fn read_bounded<R: AsyncRead + Unpin>(mut reader: R, max_bytes: u64) -> io::Result<Stream> {
    let mut bytes = Vec::new();
    (&mut reader).take(max_bytes).read_to_end(&mut bytes).await?;
    let dropped = tokio::io::copy(&mut reader, &mut tokio::io::sink()).await?;
    Ok(Stream { bytes, dropped })
}

/// Runs `command` to its end with its output captured and bounded, in its own process group.
///
/// Its standard input is closed. When it runs over `limits.timeout`, its whole group is killed.
///
/// # Errors
/// [`CaptureError`]: the program cannot be started, its output cannot be read, it ran over its time or its group
/// cannot be signalled.
pub(crate) async fn capture(command: Command, limits: CaptureLimits) -> Result<Captured, CaptureError> {
    capture_feeding(command, limits, None, None).await
}

/// Captures a snapshot command within an absolute deadline, including executable-busy retries.
/// Timeout kills the process group and allows at most one further second to reap its leader.
///
/// # Errors
/// The errors of [`capture`], including timeout before the process could start.
pub(crate) async fn capture_until(
    command: Command,
    limits: CaptureLimits,
    deadline: tokio::time::Instant,
) -> Result<Captured, CaptureError> {
    capture_feeding(command, limits, None, Some(deadline)).await
}

/// Writes `input` to the standard input of a child and closes it, when there is an input and a pipe.
async fn write_input(stdin: Option<ChildStdin>, input: Option<&[u8]>) -> io::Result<()> {
    let (Some(mut pipe), Some(bytes)) = (stdin, input) else {
        return Ok(());
    };
    match pipe.write_all(bytes).await {
        Ok(()) => {}
        // The process ended, or closed its input, before it read everything: what it made of what it did read is in
        // its output and its exit status, which the caller gets.
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
        Err(error) => return Err(error),
    }
    pipe.shutdown().await
}

/// [`capture`], with `input` written to the standard input when there is one; without, the input is closed.
async fn capture_feeding(
    mut command: Command,
    limits: CaptureLimits,
    input: Option<&[u8]>,
    deadline: Option<tokio::time::Instant>,
) -> Result<Captured, CaptureError> {
    if deadline.is_some_and(|deadline| deadline <= tokio::time::Instant::now()) {
        return Err(CaptureError::Timeout { limit: limits.timeout });
    }
    command
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true);
    let started = match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, spawn_when_free(&mut command))
            .await
            .map_err(|_elapsed| CaptureError::Timeout { limit: limits.timeout })?,
        None => spawn_when_free(&mut command).await,
    };
    let mut child = started.map_err(|source| CaptureError::Spawn { source })?;
    let (_, group) = leader_of(&child)?;
    let stdin = child.stdin.take();
    let stdout = child
        .stdout
        .take()
        .ok_or(CaptureError::MissingPipe { pipe: "stdout" })?;
    let stderr = child
        .stderr
        .take()
        .ok_or(CaptureError::MissingPipe { pipe: "stderr" })?;
    let run = async {
        let (stdout, stderr, status, written) = tokio::join!(
            read_bounded(stdout, limits.max_bytes),
            read_bounded(stderr, limits.max_bytes),
            child.wait(),
            write_input(stdin, input)
        );
        let io_error = |source| CaptureError::Io { source };
        written.map_err(io_error)?;
        Ok::<_, CaptureError>((
            stdout.map_err(io_error)?,
            stderr.map_err(io_error)?,
            status.map_err(io_error)?,
        ))
    };
    let outcome = match deadline {
        Some(deadline) => {
            tokio::time::timeout_at(deadline.min(tokio::time::Instant::now() + limits.timeout), run).await
        }
        None => timeout(limits.timeout, run).await,
    };
    match outcome {
        Ok(result) => {
            let (stdout, stderr, status) = result?;
            Ok(Captured {
                exit: ExitReport::from_status(status)?,
                stdout,
                stderr,
            })
        }
        Err(_time_over) => {
            signal_group(group, GroupSignal::Kill)?;
            // The error being returned is the timeout; the reaping of the killed leader is cleanup, logged if it fails.
            let reap_for = if deadline.is_some() {
                Duration::from_secs(1)
            } else {
                KILL_WAIT
            };
            match timeout(reap_for, child.wait()).await {
                Ok(Ok(_status)) => {}
                Ok(Err(error)) => tracing::warn!(error = %error, "cannot reap a captured process that was killed"),
                Err(_wait_over) => tracing::warn!("a captured process that was killed did not end"),
            }
            Err(CaptureError::Timeout { limit: limits.timeout })
        }
    }
}

/// Runs `command` like [`capture`], with `input` written to its standard input, which is closed after it.
///
/// # Errors
/// The errors of [`capture`].
pub(crate) async fn capture_with_input(
    command: Command,
    limits: CaptureLimits,
    input: &[u8],
) -> Result<Captured, CaptureError> {
    capture_feeding(command, limits, Some(input), None).await
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::str::FromStr;
    use std::time::UNIX_EPOCH;

    use super::*;

    /// How long a test waits for something a process does on its own before it gives up.
    const PATIENCE: Duration = Duration::from_secs(20);

    /// A grace that a process which ignores `SIGTERM` is sure to exceed, and one that is long enough for a process
    /// that does not.
    const SHORT_GRACE: Duration = Duration::from_millis(200);
    const LONG_GRACE: Duration = Duration::from_secs(20);

    fn abs(path: &Path) -> AbsolutePath {
        AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
    }

    fn env_name(name: &str) -> EnvName {
        name.parse().unwrap()
    }

    fn supervisor() -> Supervisor {
        Supervisor::new(NonZeroUsize::new(4).unwrap())
    }

    /// A spec that runs `program` with `args` in `dir`, its output in `dir/out` and `dir/err`.
    fn spec(dir: &Path, program: &str, args: &[&str]) -> ProcessSpec {
        ProcessSpec {
            program: program.parse().unwrap(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            env: BTreeMap::new(),
            working_dir: abs(dir),
            stdout_file: dir.join("out"),
            stderr_file: dir.join("err"),
        }
    }

    /// A spec that runs the shell script `script` in `dir`.
    fn script(dir: &Path, script: &str) -> ProcessSpec {
        spec(dir, "sh", &["-c", script])
    }

    /// The state letter and the group of a process from the process table; `None` when it is not there.
    fn process_table_entry(pid: i32) -> Option<(char, i32)> {
        let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // The name is in parentheses and may hold spaces: the fields start after the last parenthesis.
        let fields: Vec<&str> = text.rsplit_once(") ")?.1.split(' ').collect();
        Some((fields.first()?.chars().next()?, fields.get(2)?.parse().ok()?))
    }

    /// Whether the process is gone: not in the process table, or a zombie that only waits to be reaped.
    fn is_gone(pid: i32) -> bool {
        process_table_entry(pid).is_none_or(|(state, _)| matches!(state, 'Z' | 'X'))
    }

    /// Calls `check` until it returns true, for at most [`PATIENCE`].
    async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + PATIENCE;
        while !check() {
            assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Waits until the file at `path` holds a line `ready`.
    async fn wait_ready(path: &Path) {
        eventually("the process to be ready", || {
            std::fs::read_to_string(path).is_ok_and(|text| text.lines().any(|line| line == "ready"))
        })
        .await;
    }

    /// The process IDs a script wrote to `dir/pids`, one per line.
    fn pids(dir: &Path) -> Vec<i32> {
        std::fs::read_to_string(dir.join("pids"))
            .unwrap()
            .lines()
            .map(|line| line.parse().unwrap())
            .collect()
    }

    async fn assert_all_gone(pids: &[i32]) {
        assert_ne!(pids, [] as [i32; 0]);
        eventually("the group's processes to end", || pids.iter().all(|pid| is_gone(*pid))).await;
    }

    /// A script that starts two background sleepers — one ignoring `SIGTERM` — records their IDs in `pids` and
    /// then runs `tail`.
    fn sleepers(tail: &str) -> String {
        format!("sleep 300 & echo $! >> pids; (trap '' TERM; exec sleep 300) & echo $! >> pids; echo ready; {tail}")
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    // Rule: the arguments reach the program as they are: no shell expands them.
    #[tokio::test]
    async fn the_arguments_are_not_interpreted_by_a_shell() {
        let dir = tempfile::tempdir().unwrap();
        let args = ["%s|", "a b", "$(echo hi)", ";false", "*"];

        let mut process = supervisor().spawn(&spec(dir.path(), "printf", &args)).await.unwrap();
        let exit = process.wait().await.unwrap();

        assert_eq!(exit, ExitReport::Exited { code: 0 });
        assert_eq!(read(&dir.path().join("out")), "a b|$(echo hi)|;false|*|");
    }

    // Rule: the process runs in the working directory it was given.
    #[tokio::test]
    async fn the_process_runs_in_the_working_directory() {
        let dir = tempfile::tempdir().unwrap();

        let mut process = supervisor().spawn(&spec(dir.path(), "pwd", &[])).await.unwrap();
        process.wait().await.unwrap();

        let printed = read(&dir.path().join("out"));
        assert_eq!(Path::new(printed.trim_end()), dir.path().canonicalize().unwrap());
    }

    // Rule: the variables of the spec are added to the environment, and the others stay.
    #[tokio::test]
    async fn the_environment_is_added_to_the_inherited_one() {
        let dir = tempfile::tempdir().unwrap();
        let mut run = script(dir.path(), "printf '%s|%s' \"$AGENT_TEST_TOKEN\" \"${PATH:+path}\"");
        run.env.insert(env_name("AGENT_TEST_TOKEN"), "s3cr3t value".to_owned());

        let mut process = supervisor().spawn(&run).await.unwrap();
        process.wait().await.unwrap();

        assert_eq!(read(&dir.path().join("out")), "s3cr3t value|path");
    }

    // Rule: a spec holds secrets in its environment, so its Debug output never shows a value.
    #[test]
    fn debug_output_of_a_spec_shows_no_environment_value() {
        let dir = tempfile::tempdir().unwrap();
        let mut run = script(dir.path(), "true");
        run.env.insert(env_name("TOKEN"), "s3cr3t-value".to_owned());

        let shown = format!("{run:?}");

        assert!(shown.contains("TOKEN"), "{shown}");
        assert!(!shown.contains("s3cr3t-value"), "{shown}");
    }

    // Rule: standard output and standard error go to their own files; the directories are created and a later run
    // appends.
    #[tokio::test]
    async fn output_goes_to_files_that_are_created_and_appended_to() {
        let dir = tempfile::tempdir().unwrap();
        let mut run = script(dir.path(), "echo to-out; echo to-err >&2");
        run.stdout_file = dir.path().join("logs/run/out");
        run.stderr_file = dir.path().join("logs/run/err");

        for _ in 0..2 {
            let mut process = supervisor().spawn(&run).await.unwrap();
            process.wait().await.unwrap();
        }

        assert_eq!(read(&dir.path().join("logs/run/out")), "to-out\nto-out\n");
        assert_eq!(read(&dir.path().join("logs/run/err")), "to-err\nto-err\n");
    }

    // Rule: the standard input is closed, so a program that reads it ends instead of waiting for a terminal.
    #[tokio::test]
    async fn the_standard_input_is_closed() {
        let dir = tempfile::tempdir().unwrap();

        let mut process = supervisor().spawn(&spec(dir.path(), "cat", &[])).await.unwrap();
        let exit = process.wait().await.unwrap();

        assert_eq!(exit, ExitReport::Exited { code: 0 });
    }

    // Rule: the exit code is reported as it is, and a nonzero code is not a success.
    #[tokio::test]
    async fn wait_reports_the_exit_code() {
        let dir = tempfile::tempdir().unwrap();

        let mut process = supervisor().spawn(&script(dir.path(), "exit 3")).await.unwrap();
        let exit = process.wait().await.unwrap();

        assert_eq!(exit, ExitReport::Exited { code: 3 });
        assert!(!exit.succeeded());
        assert!(ExitReport::Exited { code: 0 }.succeeded());
        assert!(!ExitReport::Signaled { signal: 9 }.succeeded());
    }

    // Rule: a process ended by a signal is reported with the signal, not as a code.
    #[tokio::test]
    async fn wait_reports_the_signal() {
        let dir = tempfile::tempdir().unwrap();

        let mut process = supervisor().spawn(&script(dir.path(), "kill -9 $$")).await.unwrap();
        let exit = process.wait().await.unwrap();

        assert_eq!(exit, ExitReport::Signaled { signal: 9 });
    }

    // Rule: waiting again after the end gives the same answer.
    #[tokio::test]
    async fn wait_can_be_repeated_after_the_process_ended() {
        let dir = tempfile::tempdir().unwrap();
        let mut process = supervisor().spawn(&script(dir.path(), "exit 5")).await.unwrap();
        let first = process.wait().await.unwrap();

        let second = process.wait().await.unwrap();

        assert_eq!(
            (first, second),
            (ExitReport::Exited { code: 5 }, ExitReport::Exited { code: 5 })
        );
    }

    // Rule: a program that does not exist is a spawn error that names it.
    #[tokio::test]
    async fn a_missing_program_is_a_spawn_error() {
        let dir = tempfile::tempdir().unwrap();
        let run = spec(dir.path(), "/nonexistent/agent-test-program", &[]);

        let result = supervisor().spawn(&run).await;

        assert!(
            matches!(&result, Err(ProcessError::Spawn { program, .. }) if program == Path::new("/nonexistent/agent-test-program")),
            "{result:?}"
        );
    }

    // Rule: an output file that cannot be opened is an error that names the file.
    #[tokio::test]
    async fn an_output_file_that_cannot_be_opened_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut run = script(dir.path(), "true");
        run.stdout_file = dir.path().to_path_buf();

        let result = supervisor().spawn(&run).await;

        assert!(
            matches!(&result, Err(ProcessError::OpenOutput { path, .. }) if path == dir.path()),
            "{result:?}"
        );
    }

    // Rule: a directory that cannot be created is an error that names it.
    #[tokio::test]
    async fn an_output_directory_that_cannot_be_created_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file"), "x").unwrap();
        let mut run = script(dir.path(), "true");
        run.stdout_file = dir.path().join("file/sub/out");

        let result = supervisor().spawn(&run).await;

        assert!(
            matches!(&result, Err(ProcessError::CreateDir { path, .. }) if path == &dir.path().join("file/sub")),
            "{result:?}"
        );
    }

    // Rule: the process leads its own group, so that a stop of the group does not reach the harness.
    #[tokio::test]
    async fn the_process_leads_its_own_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let process = supervisor()
            .spawn(&script(dir.path(), "echo ready; sleep 300"))
            .await
            .unwrap();
        wait_ready(&dir.path().join("out")).await;
        let id = i32::try_from(process.id().get()).unwrap();

        let entry = process_table_entry(id);

        assert_eq!(entry.map(|(_, group)| group), Some(id));
        process.stop(LONG_GRACE).await.unwrap();
    }

    // Rule: the supervisor runs at most its limit, and a slot is free again when the process is gone.
    #[tokio::test]
    async fn the_limit_of_running_processes_is_enforced_and_given_back() {
        let dir = tempfile::tempdir().unwrap();
        let supervisor = Supervisor::new(NonZeroUsize::new(1).unwrap());
        let first = supervisor.spawn(&script(dir.path(), "sleep 300")).await.unwrap();

        let refused = supervisor.spawn(&script(dir.path(), "true")).await;
        assert!(
            matches!(&refused, Err(ProcessError::Limit { limit: 1, .. })),
            "{refused:?}"
        );

        first.stop(LONG_GRACE).await.unwrap();
        let mut again = supervisor.spawn(&script(dir.path(), "true")).await.unwrap();
        again.wait().await.unwrap();
    }

    // Rule: last activity is the later of the two output files' modification times.
    #[tokio::test]
    async fn last_activity_is_the_latest_modification_of_the_output_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut process = supervisor()
            .spawn(&script(dir.path(), "echo a; echo b >&2"))
            .await
            .unwrap();
        process.wait().await.unwrap();
        let far = UNIX_EPOCH + Duration::from_secs(4_000_000_000);
        let farther = far + Duration::from_secs(100);
        let touch = |name: &str, time: SystemTime| {
            std::fs::File::options()
                .write(true)
                .open(dir.path().join(name))
                .unwrap()
                .set_modified(time)
                .unwrap();
        };

        touch("out", far);
        touch("err", UNIX_EPOCH);
        assert_eq!(process.last_activity().await.unwrap(), far);

        touch("err", farther);
        assert_eq!(process.last_activity().await.unwrap(), farther);
    }

    // Rule: a process that has not written anything yet counts from its start.
    #[tokio::test]
    async fn last_activity_of_a_silent_process_is_not_before_its_start() {
        let dir = tempfile::tempdir().unwrap();
        let before = SystemTime::now();
        let process = supervisor().spawn(&script(dir.path(), "sleep 300")).await.unwrap();
        let old = UNIX_EPOCH + Duration::from_secs(1);
        for name in ["out", "err"] {
            std::fs::File::options()
                .write(true)
                .open(dir.path().join(name))
                .unwrap()
                .set_modified(old)
                .unwrap();
        }

        let activity = process.last_activity().await.unwrap();

        assert!(activity >= before);
        process.stop(LONG_GRACE).await.unwrap();
    }

    // Rule: a stop ends the whole group — the leader and everything it started, also what ignores SIGTERM.
    #[tokio::test]
    async fn stop_leaves_no_process_of_the_group_behind() {
        let dir = tempfile::tempdir().unwrap();
        let process = supervisor()
            .spawn(&script(dir.path(), &sleepers("wait")))
            .await
            .unwrap();
        wait_ready(&dir.path().join("out")).await;
        let children = pids(dir.path());
        assert_eq!(children.len(), 2);
        assert!(
            children.iter().all(|pid| !is_gone(*pid)),
            "the children run before the stop"
        );

        let report = process.stop(LONG_GRACE).await.unwrap();

        assert_eq!(report.how, StopHow::Terminated);
        assert_eq!(report.exit, ExitReport::Signaled { signal: 15 });
        assert_all_gone(&children).await;
    }

    // Rule: a leader that ignores SIGTERM is killed when the grace is over.
    #[tokio::test]
    async fn stop_kills_a_leader_that_ignores_sigterm_after_the_grace() {
        let dir = tempfile::tempdir().unwrap();
        let process = supervisor()
            .spawn(&script(
                dir.path(),
                &sleepers("trap '' TERM; while :; do sleep 1; done"),
            ))
            .await
            .unwrap();
        wait_ready(&dir.path().join("out")).await;
        let children = pids(dir.path());
        let leader = i32::try_from(process.id().get()).unwrap();

        let report = process.stop(SHORT_GRACE).await.unwrap();

        assert_eq!(report.how, StopHow::Killed);
        assert_eq!(report.exit, ExitReport::Signaled { signal: 9 });
        assert_all_gone(&children).await;
        assert_all_gone(&[leader]).await;
    }

    // Rule: a stop after the leader ended still clears what it left in its group.
    #[tokio::test]
    async fn stop_after_the_leader_ended_kills_what_it_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let mut process = supervisor()
            .spawn(&script(dir.path(), "sleep 300 & echo $! >> pids; echo ready"))
            .await
            .unwrap();
        wait_ready(&dir.path().join("out")).await;
        let exit = process.wait().await.unwrap();
        let children = pids(dir.path());
        assert!(
            children.iter().all(|pid| !is_gone(*pid)),
            "the child outlives its leader"
        );

        let report = process.stop(LONG_GRACE).await.unwrap();

        assert_eq!(
            report,
            StopReport {
                exit,
                how: StopHow::AlreadyExited
            }
        );
        assert_all_gone(&children).await;
    }

    // Rule: dropping a process that runs kills its group: nothing is left running by accident.
    #[tokio::test]
    async fn dropping_a_running_process_kills_its_group() {
        let dir = tempfile::tempdir().unwrap();
        let process = supervisor()
            .spawn(&script(dir.path(), &sleepers("wait")))
            .await
            .unwrap();
        wait_ready(&dir.path().join("out")).await;
        let children = pids(dir.path());
        let leader = i32::try_from(process.id().get()).unwrap();

        drop(process);

        assert_all_gone(&children).await;
        assert_all_gone(&[leader]).await;
    }

    fn limits(timeout: Duration, max_bytes: u64) -> CaptureLimits {
        CaptureLimits { timeout, max_bytes }
    }

    fn shell(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    // Rule: a captured run returns both streams and how it ended, also when it failed.
    #[tokio::test]
    async fn capture_returns_both_streams_and_the_exit() {
        let captured = capture(shell("echo out; echo err >&2; exit 4"), limits(LONG_GRACE, 1024))
            .await
            .unwrap();

        assert_eq!(
            captured,
            Captured {
                exit: ExitReport::Exited { code: 4 },
                stdout: Stream {
                    bytes: b"out\n".to_vec(),
                    dropped: 0
                },
                stderr: Stream {
                    bytes: b"err\n".to_vec(),
                    dropped: 0
                },
            }
        );
    }

    // Rule: output over the limit is read to the end (the process is not blocked) but only the first bytes are kept.
    #[tokio::test]
    async fn capture_keeps_the_first_bytes_and_counts_the_rest() {
        let captured = capture(shell("head -c 100000 /dev/zero | tr '\\0' x"), limits(LONG_GRACE, 10))
            .await
            .unwrap();

        assert_eq!(captured.exit, ExitReport::Exited { code: 0 });
        assert_eq!(captured.stdout.bytes, b"xxxxxxxxxx");
        assert_eq!(captured.stdout.dropped, 99_990);
    }

    // Rule: the standard input of a captured run is closed.
    #[tokio::test]
    async fn capture_closes_the_standard_input() {
        let captured = capture(Command::new("cat"), limits(LONG_GRACE, 1024)).await.unwrap();

        assert_eq!(captured.exit, ExitReport::Exited { code: 0 });
    }

    // Rule: a run over its time is an error, and its group — the children too — is killed.
    #[tokio::test]
    async fn capture_kills_the_group_of_a_run_that_takes_too_long() {
        let dir = tempfile::tempdir().unwrap();
        let mut command = shell("sleep 300 & echo $! >> pids; sleep 300");
        command.current_dir(dir.path());

        let result = capture(command, limits(SHORT_GRACE, 1024)).await;

        assert!(
            matches!(&result, Err(CaptureError::Timeout { limit }) if *limit == SHORT_GRACE),
            "{result:?}"
        );
        assert_all_gone(&pids(dir.path())).await;
    }

    // Rule: a program that cannot be started is a spawn error.
    #[tokio::test]
    async fn capture_reports_a_program_that_cannot_be_started() {
        let result = capture(
            Command::new("/nonexistent/agent-test-program"),
            limits(LONG_GRACE, 1024),
        )
        .await;

        assert!(matches!(&result, Err(CaptureError::Spawn { .. })), "{result:?}");
    }

    /// A script `run` in `dir` and its file still open for writing, as in a child that another thread forked while
    /// the script was being written: `exec` refuses it (`ETXTBSY`) until the file is closed.
    fn script_still_open(dir: &Path) -> (PathBuf, std::fs::File) {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let path = dir.join("run");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o755)
            .open(&path)
            .unwrap();
        file.write_all(b"#!/bin/sh\necho ran\n").unwrap();
        (path, file)
    }

    /// Closes `file` after a moment, from another task.
    fn close_soon(file: std::fs::File) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            drop(file);
        })
    }

    // Rule: a program that is busy only for a moment, still open for writing elsewhere, is started once it is free.
    #[tokio::test]
    async fn capture_starts_a_program_that_was_busy_for_a_moment() {
        let dir = tempfile::tempdir().unwrap();
        let (path, file) = script_still_open(dir.path());
        let closer = close_soon(file);

        let result = capture(Command::new(&path), limits(LONG_GRACE, 1024)).await;

        closer.await.unwrap();
        assert_eq!(result.unwrap().stdout.bytes, b"ran\n");
    }

    // Rule: a supervised program that is busy only for a moment is started once it is free.
    #[tokio::test]
    async fn spawn_starts_a_program_that_was_busy_for_a_moment() {
        let dir = tempfile::tempdir().unwrap();
        let (path, file) = script_still_open(dir.path());
        let closer = close_soon(file);

        let result = supervisor().spawn(&spec(dir.path(), path.to_str().unwrap(), &[])).await;

        closer.await.unwrap();
        let mut process = result.unwrap();
        process.wait().await.unwrap();
        assert_eq!(std::fs::read_to_string(dir.path().join("out")).unwrap(), "ran\n");
    }

    // Rule: a status with neither a code nor a signal is refused, a code and a signal are told apart.
    #[test]
    fn a_wait_status_is_a_code_or_a_signal() {
        use std::os::unix::process::ExitStatusExt;

        assert_eq!(
            ExitReport::from_status(ExitStatus::from_raw(3 << 8)),
            Ok(ExitReport::Exited { code: 3 })
        );
        assert_eq!(
            ExitReport::from_status(ExitStatus::from_raw(9)),
            Ok(ExitReport::Signaled { signal: 9 })
        );
        assert_eq!(
            ExitReport::from_status(ExitStatus::from_raw(0x137f)),
            Err(UnknownExit { raw: 0x137f })
        );
    }

    // Rule: an output file that is gone is an activity error that names it.
    #[tokio::test]
    async fn last_activity_reports_an_output_file_that_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let mut process = supervisor().spawn(&script(dir.path(), "true")).await.unwrap();
        process.wait().await.unwrap();
        std::fs::remove_file(dir.path().join("out")).unwrap();

        let result = process.last_activity().await;

        assert!(
            matches!(&result, Err(ProcessError::Activity { path, .. }) if path == &dir.path().join("out")),
            "{result:?}"
        );
    }

    // Rule: the input of a run with input is on its standard input, which is then closed.
    #[tokio::test]
    async fn capture_with_input_feeds_the_standard_input_and_closes_it() {
        let input = "héllo\nłódź\n";

        let captured = capture_with_input(Command::new("cat"), limits(LONG_GRACE, 1024), input.as_bytes())
            .await
            .unwrap();

        assert_eq!(captured.exit, ExitReport::Exited { code: 0 });
        assert_eq!(captured.stdout.bytes, input.as_bytes());
    }

    // Rule: input larger than a pipe is written while the output is read, so neither side blocks the other.
    #[tokio::test]
    async fn capture_with_input_larger_than_a_pipe_does_not_deadlock() {
        let input = vec![b'x'; 1 << 20];

        let captured = capture_with_input(Command::new("cat"), limits(LONG_GRACE, 2 << 20), &input)
            .await
            .unwrap();

        assert_eq!(captured.stdout.bytes.len(), 1 << 20);
        assert_eq!(captured.stdout.dropped, 0);
    }

    // Rule: a process that ends without reading its input is not an error of the capture: its exit says what happened.
    #[tokio::test]
    async fn capture_with_input_leaves_a_process_that_does_not_read_it_to_its_exit() {
        let input = vec![b'x'; 1 << 20];

        let captured = capture_with_input(shell("exit 3"), limits(LONG_GRACE, 1024), &input)
            .await
            .unwrap();

        assert_eq!(captured.exit, ExitReport::Exited { code: 3 });
    }

    #[derive(Debug, thiserror::Error)]
    #[error("outer")]
    struct Outer(#[source] Inner);

    #[derive(Debug, thiserror::Error)]
    #[error("inner")]
    struct Inner;

    #[test]
    fn an_error_is_followed_by_its_causes() {
        assert_eq!(error_chain(&Outer(Inner)), "outer: inner");
    }

    #[test]
    fn an_error_without_a_cause_is_its_own_text() {
        assert_eq!(error_chain(&Inner), "inner");
    }
}
