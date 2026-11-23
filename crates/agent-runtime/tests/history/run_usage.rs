//! A history predating run usage keeps its runs and gets the table of usage after the upgrade. A run whose usage was
//! never written has no row, which reads as "not told": the upgrade writes no usage of its own.

use agent_runtime::history::HistoryDb;

use super::support::roomy;

/// Upgrades a copy of the version 20 fixture and returns its connection, which keeps the file open.
fn upgraded() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("../fixtures/history-before-run-usage.sql"))
        .unwrap();
    drop(HistoryDb::open(&path, roomy()).unwrap());
    let connection = rusqlite::Connection::open(&path).unwrap();
    (dir, connection)
}

#[test]
fn a_version_20_history_is_upgraded_to_version_21() {
    let (_dir, connection) = upgraded();

    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();

    assert_eq!(version, 21);
}

#[test]
fn a_run_of_the_upgraded_history_has_no_usage_row() {
    let (_dir, connection) = upgraded();

    let rows: i64 = connection
        .query_row("SELECT COUNT(*) FROM run_usage", [], |row| row.get(0))
        .unwrap();

    assert_eq!(rows, 0);
}

#[test]
fn the_usage_table_refuses_a_negative_count() {
    let (_dir, connection) = upgraded();

    let err = connection
        .execute(
            "INSERT INTO run_usage (run, first_context, last_context, tokens_written, cost_usd) \
             VALUES ('run-1', -1, NULL, NULL, NULL)",
            [],
        )
        .expect_err("a context cannot be negative");

    assert!(matches!(
        err,
        rusqlite::Error::SqliteFailure(failure, _) if failure.code == rusqlite::ErrorCode::ConstraintViolation
    ));
}
