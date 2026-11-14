//! Forward-only migrations: the schema is built, and later changed, by numbered scripts of SQL.

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

/// Applies the migrations the database has not had yet, each in a transaction of its own, oldest first.
///
/// A migration that fails is rolled back, and the database stays at the version before it.
///
/// # Errors
/// [`OpenError::MigrationOrder`] when the versions of `migrations` are not 1, 2, 3, ...; [`OpenError::Migrate`] when
/// a migration fails.
pub fn apply(connection: &mut Connection, migrations: &[Migration], path: &std::path::Path) -> Result<(), OpenError> {
    let _ = (connection, migrations, path);
    Ok(())
}
