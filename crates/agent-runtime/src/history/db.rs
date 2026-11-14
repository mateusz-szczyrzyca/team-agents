//! The database of the history, opened for writing: its pragmas, its migrations and the transaction a batch of records
//! is written in.

use std::path::Path;
use std::time::Duration;

use agent_core::history::{MaxTextBytes, Record};
use rusqlite::{Connection, OpenFlags, TransactionBehavior};

use super::error::{OpenError, WriteError};
use super::migrate::{self, Migration};
use super::schema::{LEDGER_VERSION, MIGRATIONS};
use super::write::{self, Ledger};

/// How long the writer waits for a lock another process holds (a reader that checkpoints, a person with `sqlite3`) before
/// the batch fails and is counted as lost.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// The longest text one field keeps.
    limit: MaxTextBytes,
    /// Whether the database has the ledger.
    ledger: Ledger,
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
    /// A database of a version newer than the last of `migrations` is left as it is: nothing is written to it.
    ///
    /// # Errors
    /// [`OpenError`].
    pub fn open_with(path: &Path, migrations: &[Migration], limit: MaxTextBytes) -> Result<Self, OpenError> {
        migrate::check_order(migrations)?;
        let database = |source| OpenError::Database {
            path: path.to_path_buf(),
            source,
        };
        let mut connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(database)?;
        connection.busy_timeout(BUSY_TIMEOUT).map_err(database)?;
        let found = migrate::version_of(&connection).map_err(database)?;
        let supported = migrate::latest(migrations);
        if found > supported {
            return Err(OpenError::Newer {
                path: path.to_path_buf(),
                found,
                supported,
            });
        }
        let mode: String = connection
            .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
            .map_err(database)?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(OpenError::NotWal {
                path: path.to_path_buf(),
                mode,
            });
        }
        // The log is flushed at its checkpoints, not at every commit: a crash may lose the last commits, never corrupts.
        connection
            .pragma_update(None, "synchronous", "NORMAL")
            .map_err(database)?;
        connection.pragma_update(None, "foreign_keys", true).map_err(database)?;
        migrate::apply(&mut connection, migrations, path)?;
        let version = migrate::version_of(&connection).map_err(database)?;
        let ledger = if version >= LEDGER_VERSION {
            Ledger::Kept
        } else {
            Ledger::Absent
        };
        Ok(Self {
            connection,
            limit,
            ledger,
        })
    }

    /// The longest text one field keeps.
    #[must_use]
    pub const fn limit(&self) -> MaxTextBytes {
        self.limit
    }
}

impl Store for HistoryDb {
    fn write(&mut self, records: &[Record]) -> Result<(), WriteError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|source| WriteError::Transaction { source })?;
        for record in records {
            write::insert(&transaction, record, self.limit, self.ledger)?;
        }
        transaction
            .commit()
            .map_err(|source| WriteError::Transaction { source })
    }
}
