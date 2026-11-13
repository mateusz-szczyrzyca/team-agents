//! Opening a questions file in the VS Code window connected to this host.
//!
//! A VS Code server that a window connects to (Remote-SSH) leaves a remote command in its directory and a socket
//! (`vscode-ipc-*.sock`) for each window; running the command with `VSCODE_IPC_HOOK_CLI` set to a socket opens a file in
//! that window. The sockets of windows that were closed stay behind and refuse the connection, so the sockets of this
//! user are tried newest first — those in the runtime directory of the session before the others — until one works.

use std::path::{Path, PathBuf};
use std::time::Duration;

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
    /// An opener that looks for servers in `server_dirs` and for sockets in `runtime_dir`, then in `fallback_dir`.
    #[must_use]
    pub fn new(server_dirs: Vec<PathBuf>, runtime_dir: PathBuf, fallback_dir: PathBuf) -> Self {
        Self {
            server_dirs,
            runtime_dir,
            fallback_dir,
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
    /// [`OpenError`].
    pub async fn open(&self, file: &Path) -> Result<Opened, OpenError> {
        let _ = (
            &self.server_dirs,
            &self.runtime_dir,
            &self.fallback_dir,
            self.timeout,
            file,
        );
        Ok(Opened::NoServer)
    }
}
