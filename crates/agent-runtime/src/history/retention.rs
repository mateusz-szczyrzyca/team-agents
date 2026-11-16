//! Retention of closed rounds through the single database writer.
use super::codec::{time_key, time_key_sql};
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

/// Holds the writer transaction until every owned row has been removed.
pub(super) fn retain(connection: &mut rusqlite::Connection, request: &Retention) -> Result<(), RetainError> {
    if request.closed_before.is_none() && request.logs_before.is_none() {
        return Ok(());
    }
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute_batch(include_str!("retention.sql"))?;
    tx.execute(
        concat!(
            "INSERT INTO expired_rounds SELECT DISTINCT r.round FROM rounds r JOIN history_records h
            ON h.kind = 'round_closed' AND json_extract(h.payload, '$.round') = r.round
            WHERE (",
            time_key_sql!("h.at"),
            ") < ?1"
        ),
        [request.closed_before.map(time_key)],
    )?;
    tx.execute_batch(include_str!("retention_owned.sql"))?;
    tx.execute(
        concat!(
            "INSERT OR IGNORE INTO expired_logs SELECT id FROM daemon_logs l
            WHERE (",
            time_key_sql!("l.at"),
            ") < ?1 AND NOT EXISTS (SELECT 1 FROM daemon_log_rounds d WHERE d.log = l.id)"
        ),
        [request.logs_before.map(time_key)],
    )?;
    remove_gate_files(&tx, request)?;
    remove_rows(&tx)?;
    tx.execute_batch(
        "DROP TABLE expired_logs; DROP TABLE expired_records; DROP TABLE expired_runs;
        DROP TABLE expired_teams; DROP TABLE expired_rounds;",
    )?;
    tx.commit()?;
    // While the daemon keeps the writer open across daily passes, checkpoint and truncate the WAL so
    // freed pages are returned to the filesystem; a busy reader only defers truncation to the next pass.
    if let Err(error) = connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);") {
        tracing::warn!(%error, "cannot truncate history write-ahead log after retention");
    }
    Ok(())
}

/// Removes only files owned by records selected in the same transaction.
fn remove_gate_files(tx: &rusqlite::Transaction<'_>, request: &Retention) -> Result<(), RetainError> {
    let mut statement = tx.prepare(
        "SELECT json_extract(payload, '$.record.result.log') FROM history_records
        WHERE kind = 'gate' AND sequence IN (SELECT sequence FROM expired_records)",
    )?;
    let paths = statement.query_map([], |row| row.get::<_, String>(0))?;
    for path in paths {
        let path = PathBuf::from(path?);
        // Stored gate paths are untrusted. Refusing an out-of-home path rolls back retention.
        if !path.starts_with(&request.home)
            || path
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(RetainError::File {
                path,
                source: std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "gate output is outside the history home",
                ),
            });
        }
        match remove_beneath(&request.home, &path) {
            Ok(()) => (),
            // A previous interrupted cleanup may have removed the output before committing its rows.
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => (),
            Err(source) => return Err(RetainError::File { path, source }),
        }
    }
    Ok(())
}

/// Deletes children before parents and compacts the positions of surviving shared records.
fn remove_rows(tx: &rusqlite::Transaction<'_>) -> Result<(), rusqlite::Error> {
    for (table, column, expired, key) in [
        ("history_record_rounds", "sequence", "expired_records", "sequence"),
        ("history_records", "sequence", "expired_records", "sequence"),
        ("daemon_log_rounds", "log", "expired_logs", "id"),
        ("daemon_logs", "id", "expired_logs", "id"),
        ("steps", "run", "expired_runs", "run"),
        ("run_sessions", "run", "expired_runs", "run"),
        ("launches", "run", "expired_runs", "run"),
        ("run_ends", "run", "expired_runs", "run"),
        ("changed_files", "run", "expired_runs", "run"),
        ("commits", "run", "expired_runs", "run"),
        ("run_changes", "run", "expired_runs", "run"),
        ("run_rounds", "run", "expired_runs", "run"),
        ("runs", "run", "expired_runs", "run"),
        ("rounds", "round", "expired_rounds", "round"),
        ("plan", "team", "expired_teams", "team"),
        ("teams", "team", "expired_teams", "team"),
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE {column} IN (SELECT {key} FROM {expired})"),
            [],
        )?;
    }
    let expired_rounds: i64 = tx.query_row("SELECT count(*) FROM expired_rounds", [], |row| row.get(0))?;
    if expired_rounds == 0 {
        return Ok(());
    }
    for (table, key) in [
        ("run_rounds", "run"),
        ("history_record_rounds", "sequence"),
        ("daemon_log_rounds", "log"),
    ] {
        tx.execute_batch(&format!(
            "CREATE TEMP TABLE retained_links AS SELECT {key},
            row_number() OVER (PARTITION BY {key} ORDER BY position) AS position, round FROM {table}
            WHERE round NOT IN (SELECT round FROM expired_rounds);
            DELETE FROM {table}; INSERT INTO {table} SELECT * FROM retained_links; DROP TABLE retained_links;"
        ))?;
    }
    Ok(())
}

/// Resolves parents relative to owned directory descriptors without following symlinks.
/// The final unlink removes a symlink itself, never its target.
fn remove_beneath(home: &std::path::Path, path: &std::path::Path) -> Result<(), std::io::Error> {
    use rustix::fs::{AtFlags, Mode, OFlags, open, openat, unlinkat};
    let relative = path
        .strip_prefix(home)
        .map_err(|source| std::io::Error::new(std::io::ErrorKind::PermissionDenied, source))?;
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
    let mut directory = open(home, flags, Mode::empty())?;
    let mut parts = relative.components().peekable();
    while let Some(part) = parts.next() {
        let std::path::Component::Normal(name) = part else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "invalid gate output path",
            ));
        };
        if parts.peek().is_none() {
            return unlinkat(&directory, name, AtFlags::empty()).map_err(Into::into);
        }
        directory = openat(&directory, name, flags | OFlags::NOFOLLOW, Mode::empty())?;
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "gate output has no filename",
    ))
}
