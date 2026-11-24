//! The outcome of a run, written in the run's own directory once its process is gone: the context of its last model call
//! and its final message, whole. The owner reads a session's size from it first, so that the bound of a session does not
//! depend on the history, which may be disabled, may drop a row, or may be unreadable.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The name of the outcome file in the run's directory.
pub const OUTCOME_FILE: &str = "outcome.json";

/// The name of the temporary file the outcome is written to before it takes its name, so a reader never sees half of it.
const TEMPORARY_FILE: &str = "outcome.json.tmp";

/// What a run ended with, as the outcome file keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunOutcome {
    /// The context of the run's last model call in tokens; `None` when its stream told no size.
    pub last_context: Option<u64>,
    /// The agent's final message, whole; `None` when the run ended without one.
    pub final_message: Option<String>,
}

/// Why the outcome of a run cannot be written or read.
#[derive(Debug, thiserror::Error)]
pub enum OutcomeError {
    /// The outcome cannot be turned into JSON.
    #[error("cannot encode the outcome of a run")]
    Encode(#[source] serde_json::Error),
    /// The outcome file cannot be written.
    #[error("cannot write the outcome file {path}")]
    Write {
        /// The file that was to be written.
        path: PathBuf,
        /// What the file system refused.
        #[source]
        source: std::io::Error,
    },
    /// The outcome file cannot be read.
    #[error("cannot read the outcome file {path}")]
    Read {
        /// The file that was to be read.
        path: PathBuf,
        /// What the file system refused.
        #[source]
        source: std::io::Error,
    },
    /// The outcome file is not an outcome.
    #[error("the outcome file {path} is not an outcome")]
    Parse {
        /// The file that was read.
        path: PathBuf,
        /// What the JSON reader found.
        #[source]
        source: serde_json::Error,
    },
}

/// Writes `outcome` to the outcome file in `dir`, through a temporary file renamed over it.
///
/// # Errors
/// [`OutcomeError`] when the outcome cannot be encoded or the file cannot be written.
pub async fn write_outcome(dir: &Path, outcome: &RunOutcome) -> Result<(), OutcomeError> {
    let text = serde_json::to_string(outcome).map_err(OutcomeError::Encode)?;
    let temporary = dir.join(TEMPORARY_FILE);
    let path = dir.join(OUTCOME_FILE);
    tokio::fs::write(&temporary, text)
        .await
        .map_err(|source| OutcomeError::Write {
            path: temporary.clone(),
            source,
        })?;
    tokio::fs::rename(&temporary, &path)
        .await
        .map_err(|source| OutcomeError::Write { path, source })
}

/// Reads the outcome file in `dir`; `None` when the run wrote none (an older run, or one that was cancelled).
///
/// # Errors
/// [`OutcomeError`] when the file exists but cannot be read or is not an outcome.
pub async fn read_outcome(dir: &Path) -> Result<Option<RunOutcome>, OutcomeError> {
    let path = dir.join(OUTCOME_FILE);
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(text) => text,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(OutcomeError::Read { path, source }),
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|source| OutcomeError::Parse { path, source })
}
