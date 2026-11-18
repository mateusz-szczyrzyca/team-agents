//! Bounded read-only bus message tail, restricted to this team and the end of the bus.

#[cfg(test)]
mod tests;

use crate::store::Home;
use agent_core::state::Team;
use std::path::PathBuf;

/// Maximum bytes read from the end of the bus log.
const LIMIT: u64 = 4 * 1024 * 1024;
/// Number of recent bus messages inspected from the end of the whole bus.
const RECENT_MESSAGES: usize = 50;

/// Why the bus log could not be read.
#[derive(Debug, thiserror::Error)]
pub(super) enum BusError {
    /// The bus log could not be accessed.
    #[error("cannot read bus messages {path:?}")]
    Read {
        /// File path.
        path: PathBuf,
        /// Original operating system failure.
        source: std::io::Error,
    },
    /// Log bytes are not UTF-8.
    #[error("bus messages log is not UTF-8")]
    Utf8(#[source] std::string::FromUtf8Error),
}

/// Reads the last messages to, from or about the team, its tasks and its agents,
/// and the last messages of the whole bus.
///
/// # Errors
/// Returns [`BusError`] when reading the bus file fails.
pub(super) async fn collect(home: &Home, team: &Team) -> Result<Option<Vec<String>>, BusError> {
    let _ = (home, team);
    // STUB: returns empty vec to fail RED tests
    Ok(Some(Vec::new()))
}
