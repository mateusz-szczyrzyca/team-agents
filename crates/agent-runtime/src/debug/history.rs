//! History collection for debug diagnostics, reading live databases without modifying home files.

use std::path::{Path, PathBuf};

use crate::history::ReadError;
use agent_core::history::{Record, SequencedRecord};
use agent_core::ids::{RunId, TeamId};

#[cfg(test)]
mod tests;

/// Why reading live history failed.
#[derive(Debug, thiserror::Error)]
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

fn sidecar_path(db_path: &Path, suffix: &str) -> PathBuf {
    let mut path = db_path.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn history_stamp(db_path: &Path, wal_path: &Path) -> Result<(u64, std::time::SystemTime, u64), HistoryCollectionError> {
    let metadata = std::fs::metadata(db_path).map_err(|source| HistoryCollectionError::Io {
        path: db_path.to_path_buf(),
        source,
    })?;
    let modified = metadata.modified().map_err(|source| HistoryCollectionError::Io {
        path: db_path.to_path_buf(),
        source,
    })?;
    let wal_len = if wal_path.exists() {
        std::fs::metadata(wal_path)
            .map_err(|source| HistoryCollectionError::Io {
                path: wal_path.to_path_buf(),
                source,
            })?
            .len()
    } else {
        0
    };
    Ok((metadata.len(), modified, wal_len))
}

/// Reads the history records for `team` from `history_path` without modifying any files in its directory.
///
/// # Errors
/// [`HistoryCollectionError::Io`] if files could not be read or copied.
/// [`HistoryCollectionError::Changed`] if the database changed during copy.
/// [`HistoryCollectionError::Read`] if reading records from the snapshot fails.
pub(crate) fn read_live_team_history(
    history_path: &Path,
    team: &TeamId,
) -> Result<Vec<SequencedRecord>, HistoryCollectionError> {
    if !history_path.exists() {
        return Err(HistoryCollectionError::Io {
            path: history_path.to_path_buf(),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "history database does not exist"),
        });
    }

    let wal_path = sidecar_path(history_path, "-wal");
    let shm_path = sidecar_path(history_path, "-shm");

    let temp_dir = tempfile::Builder::new()
        .prefix("debug-history-")
        .tempdir()
        .map_err(|source| HistoryCollectionError::Io {
            path: history_path.to_path_buf(),
            source,
        })?;

    let copied_db = temp_dir.path().join("history.db");
    let copied_wal = temp_dir.path().join("history.db-wal");
    let copied_shm = temp_dir.path().join("history.db-shm");

    let before = history_stamp(history_path, &wal_path)?;

    std::fs::copy(history_path, &copied_db).map_err(|source| HistoryCollectionError::Io {
        path: history_path.to_path_buf(),
        source,
    })?;

    if wal_path.exists() {
        let wal_len = std::fs::metadata(&wal_path)
            .map_err(|source| HistoryCollectionError::Io {
                path: wal_path.clone(),
                source,
            })?
            .len();
        if wal_len > 0 {
            std::fs::copy(&wal_path, &copied_wal).map_err(|source| HistoryCollectionError::Io {
                path: wal_path.clone(),
                source,
            })?;
        }
    }

    if shm_path.exists() {
        let shm_len = std::fs::metadata(&shm_path)
            .map_err(|source| HistoryCollectionError::Io {
                path: shm_path.clone(),
                source,
            })?
            .len();
        if shm_len > 0 {
            std::fs::copy(&shm_path, &copied_shm).map_err(|source| HistoryCollectionError::Io {
                path: shm_path.clone(),
                source,
            })?;
        }
    }

    let after = history_stamp(history_path, &wal_path)?;
    if before != after {
        return Err(HistoryCollectionError::Changed);
    }

    let reader = crate::history::Reader::open(&copied_db).map_err(HistoryCollectionError::Read)?;
    reader.team_records(team).map_err(HistoryCollectionError::Read)
}
