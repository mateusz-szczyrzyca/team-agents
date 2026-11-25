//! The context of a run's session while the run goes on: `tasks/<task>/context.json`, rewritten each time the context of
//! the run's latest model call changes. The daemon reads it to remind a session that is near the bound of its context; the
//! outcome file of the run (`outcome.rs`) tells the same only once the run is over.

use std::path::{Path, PathBuf};

use agent_core::cli::SessionId;
use agent_core::ids::RunId;
use serde::{Deserialize, Serialize};

/// The name of the context file in the task's directory.
pub const CONTEXT_FILE: &str = "context.json";

/// The name of the temporary file the record is written to before it takes its name, so a reader never sees half of it.
const TEMPORARY_FILE: &str = "context.json.tmp";

/// What the context file holds: the context of one run's latest model call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextRecord {
    /// The run that wrote the file; the daemon ignores a file of another run.
    pub run: RunId,
    /// The session the run works in; `None` while the CLI has named none.
    pub session: Option<SessionId>,
    /// The context of the run's latest model call in tokens.
    pub context: u64,
}

/// Why the context file of a task cannot be written or read.
#[derive(Debug, thiserror::Error)]
pub enum ContextError {
    /// The record cannot be turned into JSON.
    #[error("cannot encode the context of a run")]
    Encode(#[source] serde_json::Error),
    /// The context file cannot be written.
    #[error("cannot write the context file {path}")]
    Write {
        /// The file that was to be written.
        path: PathBuf,
        /// What the file system refused.
        #[source]
        source: std::io::Error,
    },
    /// The context file cannot be read.
    #[error("cannot read the context file {path}")]
    Read {
        /// The file that was to be read.
        path: PathBuf,
        /// What the file system refused.
        #[source]
        source: std::io::Error,
    },
    /// The context file is not a record of a context.
    #[error("the context file {path} is not a record of a context")]
    Parse {
        /// The file that was read.
        path: PathBuf,
        /// What the JSON reader found.
        #[source]
        source: serde_json::Error,
    },
}

/// Writes `record` to the context file in `dir`, the directory of the task, through a temporary file renamed over it.
///
/// # Errors
/// [`ContextError`] when the record cannot be encoded or the file cannot be written.
pub async fn write_context(dir: &Path, record: &ContextRecord) -> Result<(), ContextError> {
    let text = serde_json::to_string(record).map_err(ContextError::Encode)?;
    let temporary = dir.join(TEMPORARY_FILE);
    let path = dir.join(CONTEXT_FILE);
    tokio::fs::write(&temporary, text)
        .await
        .map_err(|source| ContextError::Write {
            path: temporary.clone(),
            source,
        })?;
    tokio::fs::rename(&temporary, &path)
        .await
        .map_err(|source| ContextError::Write { path, source })
}

/// Reads the context file in `dir`, the directory of the task; `None` when no run wrote one (a task that has run no CLI
/// with a context, or none yet).
///
/// # Errors
/// [`ContextError`] when the file exists but cannot be read or is not a record of a context.
pub async fn read_context(dir: &Path) -> Result<Option<ContextRecord>, ContextError> {
    let path = dir.join(CONTEXT_FILE);
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(ContextError::Read { path, source }),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|source| ContextError::Parse { path, source })
}
