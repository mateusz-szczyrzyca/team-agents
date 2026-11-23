//! A history predating run usage keeps its runs and gets the table of usage after the upgrade. A run whose usage was
//! never written has no row, which reads as "not told": the upgrade writes no usage of its own.

use agent_core::cli::RunUsage;
use agent_runtime::history::{HistoryDb, RunUsageRecord, Store};

use super::support::{complete, database, roomy};

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

#[test]
fn a_written_usage_is_kept_for_its_run() {
    let (_dir, path, mut db) = database();
    let world = complete(1);
    db.write(&world.records()).unwrap();
    let run = world.runs[0].start.run.clone();
    let usage = RunUsage {
        first_context: Some(1_000),
        last_context: Some(2_500),
        tokens_written: Some(400),
        cost_usd: Some(0.25),
    };

    db.write_usage(&[RunUsageRecord {
        run: run.clone(),
        usage,
    }])
    .unwrap();

    let raw = rusqlite::Connection::open(&path).unwrap();
    let row: (Option<i64>, Option<i64>, Option<i64>, Option<f64>) = raw
        .query_row(
            "SELECT first_context, last_context, tokens_written, cost_usd FROM run_usage WHERE run = ?1",
            [run.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(row, (Some(1_000), Some(2_500), Some(400), Some(0.25)));
}

#[test]
fn a_part_the_cli_did_not_tell_is_kept_as_not_told() {
    let (_dir, path, mut db) = database();
    let world = complete(1);
    db.write(&world.records()).unwrap();
    let run = world.runs[0].start.run.clone();
    let usage = RunUsage {
        first_context: None,
        last_context: None,
        tokens_written: Some(500),
        cost_usd: None,
    };

    db.write_usage(&[RunUsageRecord {
        run: run.clone(),
        usage,
    }])
    .unwrap();

    let raw = rusqlite::Connection::open(&path).unwrap();
    let row: (Option<i64>, Option<i64>, Option<i64>, Option<f64>) = raw
        .query_row(
            "SELECT first_context, last_context, tokens_written, cost_usd FROM run_usage WHERE run = ?1",
            [run.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(row, (None, None, Some(500), None));
}
