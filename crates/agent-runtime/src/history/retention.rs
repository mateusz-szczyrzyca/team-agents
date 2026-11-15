//! Retention of closed rounds through the single database writer.
use agent_core::state::Timestamp;
use std::path::PathBuf;

/// Optional exclusive cutoffs; absent means keep everything in that category.
#[derive(Debug, Clone)]
pub struct Retention {
    /// Delete rounds closed strictly before this instant.
    pub closed_before: Option<Timestamp>,
    /// Delete log rows without rounds strictly before this instant.
    pub logs_before: Option<Timestamp>,
    /// Root under which gate output files may be removed.
    pub home: PathBuf,
}

/// Failure of a retention transaction or gate-output removal.
#[derive(Debug, thiserror::Error)]
pub enum RetainError {
    /// SQLite refused the operation.
    #[error("cannot retain history")]
    Database(#[from] rusqlite::Error),
    /// A gate output could not be removed.
    #[error("cannot remove retained gate output {path}")]
    File {
        /// File whose removal failed.
        path: PathBuf,
        /// Filesystem cause.
        #[source]
        source: std::io::Error,
    },
}
