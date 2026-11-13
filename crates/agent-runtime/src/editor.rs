//! Opening a questions file in the VS Code window connected to this host.
//!
//! A VS Code server that a window connects to (Remote-SSH) leaves a remote command in its directory and a socket
//! (`vscode-ipc-*.sock`) for each window; running the command with `VSCODE_IPC_HOOK_CLI` set to a socket opens a file in
//! that window. The sockets of windows that were closed stay behind and refuse the connection, so the sockets of this
//! user are tried newest first — those in the runtime directory of the session before the others — until one works.

use std::fs;
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tokio::process::Command;

use crate::process::{CaptureError, CaptureLimits, capture, error_chain};

/// The prefix of the name of a window's socket.
const SOCKET_PREFIX: &str = "vscode-ipc-";

/// The suffix of the name of a window's socket.
const SOCKET_SUFFIX: &str = ".sock";

/// The variable that tells the remote command which window to talk to.
const SOCKET_VARIABLE: &str = "VSCODE_IPC_HOOK_CLI";

/// How many bytes of what the remote command prints are kept.
const MAX_OUTPUT_BYTES: u64 = 64 << 10;

/// How many characters of what a window said an error carries.
const MAX_SAID_CHARS: usize = 2048;

/// Why a file could not be opened in any window.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// The directories of the servers or of the sockets cannot be listed.
    #[error("cannot list {path:?}")]
    List {
        /// The directory.
        path: PathBuf,
        /// What the operating system said.
        source: std::io::Error,
    },
    /// The task that looks for the windows failed.
    #[error("the task that looks for the windows of VS Code failed")]
    Worker(#[source] tokio::task::JoinError),
    /// The remote command cannot be started.
    #[error("cannot run the remote command {program:?} of VS Code")]
    Start {
        /// The command.
        program: PathBuf,
        /// Why it did not start.
        source: CaptureError,
    },
    /// Every window that was tried refused or did not answer.
    #[error("no window of VS Code opened {file:?}; the last one, {socket:?}, said: {said}")]
    Refused {
        /// The file.
        file: PathBuf,
        /// The socket of the last window that was tried.
        socket: PathBuf,
        /// What it said, cut.
        said: String,
    },
}

/// What came of an attempt to open a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// A window opened the file.
    Window {
        /// The socket of the window.
        socket: PathBuf,
    },
    /// There is no VS Code server on this host: nothing to open the file in.
    NoServer,
    /// There is a server but no window is connected.
    NoWindow,
}

/// The two directories that have the sockets of windows; named apart so that they cannot be swapped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketDirs {
    /// The runtime directory of the session, where the servers put the sockets of this session's windows; tried first.
    pub runtime: PathBuf,
    /// Another directory that has sockets of windows, tried after the runtime directory.
    pub fallback: PathBuf,
}

/// The opener of questions files.
#[derive(Debug, Clone)]
pub struct VscodeOpener {
    /// The directories of the VS Code servers, searched for their remote command.
    server_dirs: Vec<PathBuf>,
    /// The runtime directory of the session, where the servers put the sockets of this session's windows.
    runtime_dir: PathBuf,
    /// Another directory that has sockets of windows, tried after the runtime directory.
    fallback_dir: PathBuf,
    /// How long the remote command may take for one window.
    timeout: Duration,
}

impl VscodeOpener {
    /// An opener that looks for servers in `server_dirs` and for sockets in the directories of `sockets`.
    #[must_use]
    pub fn new(server_dirs: Vec<PathBuf>, sockets: SocketDirs) -> Self {
        Self {
            server_dirs,
            runtime_dir: sockets.runtime,
            fallback_dir: sockets.fallback,
            timeout: Duration::from_secs(10),
        }
    }

    /// The same, with another time for the remote command to answer.
    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Opens `file` in a window of VS Code.
    ///
    /// # Errors
    /// [`OpenError`]: a directory that cannot be listed, a remote command that cannot be started, or every window that
    /// was tried refusing.
    pub async fn open(&self, file: &Path) -> Result<Opened, OpenError> {
        let this = self.clone();
        let found = tokio::task::spawn_blocking(move || this.discover())
            .await
            .map_err(OpenError::Worker)??;
        let Some(program) = found.program else {
            return Ok(Opened::NoServer);
        };
        let mut refused = None;
        for socket in found.sockets {
            match self.try_window(&program, &socket, file).await? {
                Ok(()) => return Ok(Opened::Window { socket }),
                Err(said) => refused = Some((socket, said)),
            }
        }
        match refused {
            Some((socket, said)) => Err(OpenError::Refused {
                file: file.to_path_buf(),
                socket,
                said,
            }),
            None => Ok(Opened::NoWindow),
        }
    }

    /// Runs the remote command `program` for the window of `socket`. The outer error is a command that cannot be started,
    /// the inner one a window that did not take the file, with what it said.
    async fn try_window(&self, program: &Path, socket: &Path, file: &Path) -> Result<Result<(), String>, OpenError> {
        let mut command = Command::new(program);
        command.arg(file).env(SOCKET_VARIABLE, socket);
        let limits = CaptureLimits {
            timeout: self.timeout,
            max_bytes: MAX_OUTPUT_BYTES,
        };
        match capture(command, limits).await {
            Ok(captured) if captured.exit.succeeded() => Ok(Ok(())),
            // What the window said is for a human to read: invalid UTF-8 in it is replaced.
            Ok(captured) => Ok(Err(String::from_utf8_lossy(&captured.stderr.bytes)
                .trim()
                .chars()
                .take(MAX_SAID_CHARS)
                .collect())),
            Err(source @ CaptureError::Spawn { .. }) => Err(OpenError::Start {
                program: program.to_path_buf(),
                source,
            }),
            // A window that does not answer in time, or whose output cannot be read, is a window that did not take it.
            Err(other) => Ok(Err(said_by_a_failed_capture(&other))),
        }
    }

    /// Finds the remote command of the newest server and the sockets of the windows, in the order to try them.
    fn discover(&self) -> Result<Found, OpenError> {
        let mut clis = Vec::new();
        for server in &self.server_dirs {
            for build in children(&server.join("cli/servers"))? {
                clis.extend(children(&build.join("server/bin/remote-cli"))?);
            }
            for build in children(&server.join("bin"))? {
                clis.extend(children(&build.join("bin/remote-cli"))?);
            }
        }
        let program = newest(clis)?;
        let uid = rustix::process::getuid().as_raw();
        let mut sockets = windows(&self.runtime_dir, uid)?;
        if self.fallback_dir != self.runtime_dir {
            sockets.extend(windows(&self.fallback_dir, uid)?);
        }
        Ok(Found { program, sockets })
    }
}

/// What a look at the host found.
struct Found {
    /// The remote command of the newest server, if there is a server.
    program: Option<PathBuf>,
    /// The sockets of the windows of this user, in the order to try them.
    sockets: Vec<PathBuf>,
}

/// What a window that did not answer is said to have said: the failure and its causes, so that a human sees why.
fn said_by_a_failed_capture(error: &CaptureError) -> String {
    error_chain(error)
}

/// The entries of `dir`; none when the directory is not there.
fn children(dir: &Path) -> Result<Vec<PathBuf>, OpenError> {
    let list = |source| OpenError::List {
        path: dir.to_path_buf(),
        source,
    };
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        // A host without that server layout is the specified case of nothing to open the file in.
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(list(source)),
    };
    entries
        .map(|entry| entry.map(|found| found.path()).map_err(list))
        .collect()
}

/// The metadata of `path`, following a link; `None` for a file that is not there.
fn look_through(path: &Path) -> Result<Option<fs::Metadata>, OpenError> {
    seen(path, fs::metadata(path))
}

/// The metadata of `path` itself, not of what a link points to; `None` for a file that is not there.
fn look_at(path: &Path) -> Result<Option<fs::Metadata>, OpenError> {
    seen(path, fs::symlink_metadata(path))
}

/// `looked`, the result of looking at `path`, with a file that is not there as `None`.
fn seen(path: &Path, looked: io::Result<fs::Metadata>) -> Result<Option<fs::Metadata>, OpenError> {
    match looked {
        Ok(metadata) => Ok(Some(metadata)),
        // A file that was there a moment ago and is gone — a window that closed — is not there.
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(OpenError::List {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// When the file the metadata is of was last changed.
fn changed(path: &Path, metadata: &fs::Metadata) -> Result<SystemTime, OpenError> {
    metadata.modified().map_err(|source| OpenError::List {
        path: path.to_path_buf(),
        source,
    })
}

/// The executable file of `paths` that was changed last (a link is followed).
fn newest(paths: Vec<PathBuf>) -> Result<Option<PathBuf>, OpenError> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for path in paths {
        let Some(metadata) = look_through(&path)? else {
            continue;
        };
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            continue;
        }
        let time = changed(&path, &metadata)?;
        if best.as_ref().is_none_or(|(latest, _)| time > *latest) {
            best = Some((time, path));
        }
    }
    Ok(best.map(|(_, path)| path))
}

/// The sockets of the windows in `dir` that `uid` owns, newest first.
fn windows(dir: &Path, uid: u32) -> Result<Vec<PathBuf>, OpenError> {
    let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
    for path in children(dir)? {
        let named = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(SOCKET_PREFIX) && name.ends_with(SOCKET_SUFFIX));
        if !named {
            continue;
        }
        let Some(metadata) = look_at(&path)? else {
            continue;
        };
        // A socket of another user is not ours to talk to.
        if metadata.file_type().is_socket() && metadata.uid() == uid {
            found.push((changed(&path, &metadata)?, path));
        }
    }
    found.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
    Ok(found.into_iter().map(|(_, path)| path).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_that_failed_to_answer_is_said_to_have_failed_with_its_cause() {
        let error = CaptureError::Io {
            source: io::Error::new(io::ErrorKind::BrokenPipe, "the pipe broke"),
        };

        assert_eq!(
            said_by_a_failed_capture(&error),
            "cannot read the output of the process or wait for it: the pipe broke"
        );
    }
}
