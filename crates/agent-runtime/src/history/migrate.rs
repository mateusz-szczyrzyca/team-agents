//! Forward-only migrations: the schema is built, and later changed, by numbered scripts of SQL.

use std::path::Path;

use rusqlite::Connection;

use super::error::OpenError;

/// One step of the schema's history: the SQL that takes a database from the version before it to its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Migration {
    /// The version the database has after this migration: 1 for the first, then 2, 3, ...
    pub version: u32,
    /// The statements, run in one transaction together with the update of `PRAGMA user_version`.
    pub sql: &'static str,
}

/// The newest version `migrations` lead to; `0` when there are none.
#[must_use]
pub fn latest(migrations: &[Migration]) -> u32 {
    migrations.last().map_or(0, |migration| migration.version)
}

/// The version of the database `connection` is open on: `PRAGMA user_version`.
///
/// # Errors
/// What SQLite says when the pragma cannot be read.
pub fn version_of(connection: &Connection) -> Result<u32, rusqlite::Error> {
    connection.pragma_query_value(None, "user_version", |row| row.get(0))
}

/// Checks that the versions of `migrations` are 1, 2, 3, ... in order.
///
/// # Errors
/// [`OpenError::MigrationOrder`] for the first migration that stands in the wrong place.
pub fn check_order(migrations: &[Migration]) -> Result<(), OpenError> {
    let mut expected = 0_u32;
    for migration in migrations {
        expected = expected.saturating_add(1);
        if migration.version != expected {
            return Err(OpenError::MigrationOrder {
                found: migration.version,
                expected,
            });
        }
    }
    Ok(())
}

/// Applies the migrations the database has not had yet, each in a transaction of its own, oldest first.
///
/// A migration that fails is rolled back, and the database stays at the version before it.
///
/// # Errors
/// [`OpenError::MigrationOrder`] when the versions of `migrations` are not 1, 2, 3, ...; [`OpenError::Database`] when
/// the version cannot be read; [`OpenError::Migrate`] when a migration fails.
pub fn apply(connection: &mut Connection, migrations: &[Migration], path: &Path) -> Result<(), OpenError> {
    check_order(migrations)?;
    let current = version_of(connection).map_err(|source| OpenError::Database {
        path: path.to_path_buf(),
        source,
    })?;
    for migration in migrations.iter().filter(|migration| migration.version > current) {
        let failed = |source| OpenError::Migrate {
            path: path.to_path_buf(),
            version: migration.version,
            source,
        };
        let transaction = connection.transaction().map_err(failed)?;
        transaction.execute_batch(migration.sql).map_err(failed)?;
        transaction
            .pragma_update(None, "user_version", migration.version)
            .map_err(failed)?;
        transaction.commit().map_err(failed)?;
    }
    Ok(())
}
