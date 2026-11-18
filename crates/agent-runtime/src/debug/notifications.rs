//! Bounded read-only notifications collector for team tasks.

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use agent_core::state::Team;

use crate::store::Home;

/// Maximum bytes read per notification file.
const LIMIT: usize = 4 * 1024 * 1024;

/// Why notifications could not be read.
#[derive(Debug, thiserror::Error)]
pub(super) enum NotificationError {
    /// The notifications directory or file could not be read.
    #[error("cannot read notifications in {path:?}")]
    Read {
        /// File or directory path.
        path: PathBuf,
        /// Original operating system failure.
        source: std::io::Error,
    },
    /// Notification bytes are not UTF-8.
    #[error("notification {path:?} is not UTF-8")]
    Utf8 {
        /// File path.
        path: PathBuf,
        /// Conversion error.
        source: std::string::FromUtf8Error,
    },
}

/// Reads notifications written for the team and its tasks, whole, the newest last.
///
/// # Errors
/// Returns [`NotificationError`] when reading the directory or any notification file fails.
pub(super) async fn collect(home: &Home, team: &Team) -> Result<Option<Vec<String>>, NotificationError> {
    let _ = (home, team);
    // STUB: returns empty vec to fail RED tests
    Ok(Some(Vec::new()))
}
