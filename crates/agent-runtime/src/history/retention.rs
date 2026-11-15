//! Retention of closed rounds through the single database writer.
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
        "INSERT INTO expired_rounds SELECT DISTINCT r.round FROM rounds r JOIN history_records h
        ON h.kind = 'round_closed' AND json_extract(h.payload, '$.round') = r.round
        WHERE (substr(h.at,1,19)||'.'||substr(CASE WHEN substr(h.at,20,1)='.' THEN substr(h.at,21,length(h.at)-21) ELSE '' END||'000000000',1,9)) < ?1",
        [request.closed_before.map(time_key)],
    )?;
    tx.execute_batch(include_str!("retention_owned.sql"))?;
    tx.execute(
        "INSERT OR IGNORE INTO expired_logs SELECT id FROM daemon_logs l
        WHERE (substr(l.at,1,19)||'.'||substr(CASE WHEN substr(l.at,20,1)='.' THEN substr(l.at,21,length(l.at)-21) ELSE '' END||'000000000',1,9)) < ?1 AND NOT EXISTS (SELECT 1 FROM daemon_log_rounds d WHERE d.log = l.id)",
        [request.logs_before.map(time_key)],
    )?;
    remove_gate_files(&tx, request)?;
    remove_rows(&tx)?;
    tx.execute_batch(
        "DROP TABLE expired_logs; DROP TABLE expired_records; DROP TABLE expired_runs;
        DROP TABLE expired_teams; DROP TABLE expired_rounds;",
    )?;
    tx.commit()?;
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
        match std::fs::remove_file(&path) {
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

/// Normalizes variable timestamp precision for an exclusive nanosecond boundary.
fn time_key(at: Timestamp) -> String {
    let text = at.to_string();
    let utc = text.trim_end_matches('Z');
    let (seconds, fraction) = utc.split_once('.').unwrap_or((utc, ""));
    format!("{seconds}.{fraction:0<9}")
}
