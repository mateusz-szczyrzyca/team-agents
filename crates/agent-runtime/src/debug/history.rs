//! History collection for debug diagnostics, reading live databases without modifying home files.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::history::ReadError;
use agent_core::history::{Record, SequencedRecord};
use agent_core::ids::{RunId, TeamId};

#[cfg(test)]
mod tests;

/// Maximum permitted size for history files (64 MiB).
pub(crate) const MAX_HISTORY_BYTES: u64 = 64 * 1024 * 1024;

/// Why reading live history failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HistoryCollectionError {
    /// A database or sidecar file exceeds the maximum permitted size.
    #[error("history file {} exceeds limit ({size} bytes > {limit} bytes)", path.display())]
    TooLarge {
        /// File that exceeded the limit.
        path: PathBuf,
        /// Observed size in bytes.
        size: u64,
        /// Maximum allowed size in bytes.
        limit: u64,
    },
    /// The database or WAL file could not be read or copied.
    #[error("cannot read database file {}", path.display())]
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

/// Fingerprint of a history file on disk before or after copying.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    /// File size in bytes.
    len: u64,
    /// Last modification timestamp.
    modified: std::time::SystemTime,
}

/// Fingerprints of database and sidecar files across the copy operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HistoryStamp {
    /// Main database file stamp.
    db: FileStamp,
    /// WAL sidecar file stamp, if present.
    wal: Option<FileStamp>,
    /// SHM sidecar file stamp, if present.
    shm: Option<FileStamp>,
}

fn file_stamp(path: &Path) -> Result<Option<FileStamp>, HistoryCollectionError> {
    match std::fs::metadata(path) {
        Ok(metadata) => {
            let modified = metadata.modified().map_err(|err| HistoryCollectionError::Io {
                path: path.to_path_buf(),
                source: err,
            })?;
            Ok(Some(FileStamp {
                len: metadata.len(),
                modified,
            }))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(HistoryCollectionError::Io {
            path: path.to_path_buf(),
            source: err,
        }),
    }
}

fn take_history_stamp(
    db_path: &Path,
    wal_path: &Path,
    shm_path: &Path,
    limit: u64,
) -> Result<HistoryStamp, HistoryCollectionError> {
    let db = file_stamp(db_path)?.ok_or_else(|| HistoryCollectionError::Io {
        path: db_path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::NotFound, "history database does not exist"),
    })?;
    if db.len > limit {
        return Err(HistoryCollectionError::TooLarge {
            path: db_path.to_path_buf(),
            size: db.len,
            limit,
        });
    }

    let wal = file_stamp(wal_path)?;
    if let Some(w) = wal
        && w.len > limit
    {
        return Err(HistoryCollectionError::TooLarge {
            path: wal_path.to_path_buf(),
            size: w.len,
            limit,
        });
    }

    let shm = file_stamp(shm_path)?;
    if let Some(s) = shm
        && s.len > limit
    {
        return Err(HistoryCollectionError::TooLarge {
            path: shm_path.to_path_buf(),
            size: s.len,
            limit,
        });
    }

    Ok(HistoryStamp { db, wal, shm })
}

fn copy_bounded(source: &Path, destination: &Path, limit: u64) -> Result<u64, HistoryCollectionError> {
    let metadata = std::fs::metadata(source).map_err(|err| HistoryCollectionError::Io {
        path: source.to_path_buf(),
        source: err,
    })?;
    if metadata.len() > limit {
        return Err(HistoryCollectionError::TooLarge {
            path: source.to_path_buf(),
            size: metadata.len(),
            limit,
        });
    }

    let mut reader = std::fs::File::open(source).map_err(|err| HistoryCollectionError::Io {
        path: source.to_path_buf(),
        source: err,
    })?;
    let mut writer = std::fs::File::create(destination).map_err(|err| HistoryCollectionError::Io {
        path: destination.to_path_buf(),
        source: err,
    })?;

    let mut take = std::io::Read::take(&mut reader, limit.saturating_add(1));
    let copied = std::io::copy(&mut take, &mut writer).map_err(|err| HistoryCollectionError::Io {
        path: source.to_path_buf(),
        source: err,
    })?;
    if copied > limit {
        return Err(HistoryCollectionError::TooLarge {
            path: source.to_path_buf(),
            size: copied,
            limit,
        });
    }

    writer.flush().map_err(|err| HistoryCollectionError::Io {
        path: destination.to_path_buf(),
        source: err,
    })?;
    Ok(copied)
}

/// Reads the history records for `team` from `history_path` without modifying any files in its directory.
///
/// # Errors
/// [`HistoryCollectionError::TooLarge`] if database or sidecar files exceed [`MAX_HISTORY_BYTES`].
/// [`HistoryCollectionError::Io`] if files could not be read or copied.
/// [`HistoryCollectionError::Changed`] if the database changed during copy.
/// [`HistoryCollectionError::Read`] if reading records from the snapshot fails.
pub(crate) fn read_live_team_history(
    history_path: &Path,
    team: &TeamId,
) -> Result<Vec<SequencedRecord>, HistoryCollectionError> {
    if !history_path.exists() {
        return Ok(Vec::new());
    }

    let wal_path = sidecar_path(history_path, "-wal");
    let shm_path = sidecar_path(history_path, "-shm");

    let before = take_history_stamp(history_path, &wal_path, &shm_path, MAX_HISTORY_BYTES)?;

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

    copy_bounded(history_path, &copied_db, MAX_HISTORY_BYTES)?;

    if let Some(w) = before.wal
        && w.len > 0
    {
        copy_bounded(&wal_path, &copied_wal, MAX_HISTORY_BYTES)?;
    }

    if let Some(s) = before.shm
        && s.len > 0
    {
        copy_bounded(&shm_path, &copied_shm, MAX_HISTORY_BYTES)?;
    }

    let after = take_history_stamp(history_path, &wal_path, &shm_path, MAX_HISTORY_BYTES)?;
    if before != after {
        return Err(HistoryCollectionError::Changed);
    }

    let reader = crate::history::Reader::open(&copied_db).map_err(HistoryCollectionError::Read)?;
    reader.team_records(team).map_err(HistoryCollectionError::Read)
}
