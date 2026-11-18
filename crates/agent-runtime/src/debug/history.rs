//! History collection for debug diagnostics, reading live databases without modifying home files.

use std::path::{Path, PathBuf};

use crate::history::ReadError;
use agent_core::history::{Record, SequencedRecord};
use agent_core::ids::{RunId, TeamId};

#[cfg(test)]
mod tests;

/// Why reading live history failed.
#[derive(Debug, thiserror::Error)]
#[expect(dead_code, reason = "constructed in GREEN commit")]
pub(crate) enum HistoryCollectionError {
    /// The database or WAL file could not be read or copied.
    #[error("cannot read database file {path:?}")]
    Io {
        /// Path that could not be accessed.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The database file changed during snapshot creation.
    #[error("history changed during collection")]
    Changed,
    /// Reading history records failed.
    #[error("cannot read history records")]
    Read(#[source] ReadError),
}

/// Extracts the [`RunId`] associated with a [`Record`], if any.
#[must_use]
#[cfg_attr(not(test), expect(dead_code, reason = "wired in collect.rs in GREEN commit"))]
pub(crate) fn record_run_id(record: &Record) -> Option<&RunId> {
    match record {
        Record::Run(r) => Some(&r.run),
        Record::Launch(l) => Some(&l.run),
        Record::Step(s) => Some(&s.run),
        Record::RunEnd(e) => Some(&e.run),
        Record::Changes(c) => Some(&c.run),
        Record::Session(s) => Some(&s.run),
        Record::Team(_) | Record::Round(_) | Record::Event(_) => None,
    }
}

/// Reads the history records for `team` from `history_path` without modifying any files in its directory.
///
/// # Errors
/// [`HistoryCollectionError::Io`] if files could not be read or copied.
/// [`HistoryCollectionError::Changed`] if the database changed during copy.
/// [`HistoryCollectionError::Read`] if reading records from the snapshot fails.
#[cfg_attr(not(test), expect(dead_code, reason = "wired in collect.rs in GREEN commit"))]
pub(crate) fn read_live_team_history(
    _history_path: &Path,
    _team: &TeamId,
) -> Result<Vec<SequencedRecord>, HistoryCollectionError> {
    // Stub for RED commit: returns empty records
    Ok(Vec::new())
}
