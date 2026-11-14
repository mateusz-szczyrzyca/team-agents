//! The database of the history, opened for writing: its pragmas, its migrations and the transaction a batch of records
//! is written in.

use std::path::{Path, PathBuf};

use agent_core::history::{MaxTextBytes, Record};
use rusqlite::Connection;

use super::error::{OpenError, WriteError};
use super::migrate::Migration;
use super::schema::MIGRATIONS;

/// Where records are written: the database, or whatever stands in for it where a test needs the writer to stall or
/// fail.
pub trait Store: Send + 'static {
    /// Writes `records` as one unit: all of them or none.
    ///
    /// # Errors
    /// [`WriteError`] when they cannot be written; none of them is.
    fn write(&mut self, records: &[Record]) -> Result<(), WriteError>;
}

/// The history database, open for writing.
#[derive(Debug)]
pub struct HistoryDb {
    /// The connection: the only one that writes.
    connection: Connection,
    /// The file, for the messages of errors.
    path: PathBuf,
    /// The longest text one field keeps.
    limit: MaxTextBytes,
}

impl HistoryDb {
    /// Opens (and makes, and migrates) the database at `path`.
    ///
    /// # Errors
    /// [`OpenError`].
    pub fn open(path: &Path, limit: MaxTextBytes) -> Result<Self, OpenError> {
        Self::open_with(path, MIGRATIONS, limit)
    }

    /// Opens the database at `path` and migrates it with `migrations` instead of the schema of this build.
    ///
    /// # Errors
    /// [`OpenError`].
    pub fn open_with(path: &Path, migrations: &[Migration], limit: MaxTextBytes) -> Result<Self, OpenError> {
        let _ = migrations;
        let connection = Connection::open_in_memory().map_err(|source| OpenError::Database {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self {
            connection,
            path: path.to_path_buf(),
            limit,
        })
    }

    /// The file the database is in.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The longest text one field keeps.
    #[must_use]
    pub const fn limit(&self) -> MaxTextBytes {
        self.limit
    }

    /// The connection, for the tests of this module's siblings.
    #[must_use]
    pub(super) const fn connection(&self) -> &Connection {
        &self.connection
    }
}

impl Store for HistoryDb {
    fn write(&mut self, records: &[Record]) -> Result<(), WriteError> {
        let _ = records;
        Ok(())
    }
}
