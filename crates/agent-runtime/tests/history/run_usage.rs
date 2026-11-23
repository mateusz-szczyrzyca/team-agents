//! A history predating run usage keeps its runs and gets the table of usage after the upgrade. A run whose usage was
//! never written has no row, which reads as "not told": the upgrade writes no usage of its own.

use agent_core::cli::RunUsage;
use agent_runtime::history::{HistoryDb, Reader, RunUsageRecord, Store};

use super::support::{complete, database, roomy};

/// Upgrades a copy of the version 21 fixture and returns its connection, which keeps the file open.
fn upgraded() -> (tempfile::TempDir, rusqlite::Connection) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("../fixtures/history-before-run-usage-v21.sql"))
        .unwrap();
    drop(HistoryDb::open(&path, roomy()).unwrap());
    let connection = rusqlite::Connection::open(&path).unwrap();
    (dir, connection)
}

#[test]
fn a_version_21_history_is_upgraded_to_version_22() {
    let (_dir, connection) = upgraded();

    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();

    assert_eq!(version, 22);
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

#[tokio::test]
async fn the_usage_handed_to_the_writer_is_written_once_the_writer_closes() {
    use agent_runtime::history::{DEFAULT_QUEUE, History, Setup, TracingReporter};
    use std::sync::Arc;

    let (_dir, path, mut db) = database();
    let world = complete(1);
    db.write(&world.records()).unwrap();
    let run = world.runs[0].start.run.clone();
    let started = History::start(
        db,
        Setup {
            reporter: Arc::new(TracingReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    let usage = RunUsage {
        first_context: Some(7),
        last_context: Some(9),
        tokens_written: Some(3),
        cost_usd: None,
    };

    started
        .history
        .persist_usage(vec![RunUsageRecord {
            run: run.clone(),
            usage,
        }])
        .unwrap();
    started.writer.close().unwrap();

    let raw = rusqlite::Connection::open(&path).unwrap();
    let row: (Option<i64>, Option<i64>, Option<i64>, Option<f64>) = raw
        .query_row(
            "SELECT first_context, last_context, tokens_written, cost_usd FROM run_usage WHERE run = ?1",
            [run.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(row, (Some(7), Some(9), Some(3), None));
}

#[test]
fn the_reader_gives_back_the_usage_written_for_a_run() {
    let (_dir, path, mut db) = database();
    let world = complete(1);
    db.write(&world.records()).unwrap();
    let run = world.runs[0].start.run.clone();
    let usage = RunUsage {
        first_context: Some(10),
        last_context: Some(20),
        tokens_written: None,
        cost_usd: Some(0.5),
    };
    db.write_usage(&[RunUsageRecord {
        run: run.clone(),
        usage,
    }])
    .unwrap();

    let read = Reader::open(&path).unwrap().run_usage(&run).unwrap();

    assert_eq!(read, Some(usage));
}

#[test]
fn the_reader_tells_no_usage_for_a_run_without_a_row() {
    let (_dir, path, mut db) = database();
    let world = complete(1);
    db.write(&world.records()).unwrap();
    let run = world.runs[0].start.run.clone();

    let read = Reader::open(&path).unwrap().run_usage(&run).unwrap();

    assert_eq!(read, None);
}

#[test]
fn the_reader_gives_back_the_usage_of_many_runs_in_one_call_and_leaves_out_a_run_without_a_row() {
    let (_dir, path, mut db) = database();
    let world = complete(2);
    db.write(&world.records()).unwrap();
    let with_row = world.runs[0].start.run.clone();
    let without_row = world.runs[1].start.run.clone();
    let usage = RunUsage {
        first_context: Some(10),
        last_context: Some(20),
        tokens_written: Some(5),
        cost_usd: None,
    };
    db.write_usage(&[RunUsageRecord {
        run: with_row.clone(),
        usage,
    }])
    .unwrap();

    let read = Reader::open(&path)
        .unwrap()
        .run_usages(&[with_row.clone(), without_row])
        .unwrap();

    assert_eq!(read, std::collections::BTreeMap::from([(with_row, usage)]));
}

#[test]
fn the_reader_gives_no_usage_of_many_runs_when_it_is_asked_for_none() {
    let (_dir, path, _db) = database();

    let read = Reader::open(&path).unwrap().run_usages(&[]).unwrap();

    assert!(read.is_empty());
}

#[test]
fn a_run_of_a_populated_version_21_history_survives_the_upgrade_and_reads_as_not_told() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("../fixtures/history-before-run-usage-v21-with-run.sql"))
        .unwrap();
    drop(HistoryDb::open(&path, roomy()).unwrap());
    let run: agent_core::ids::RunId = "run-20261120-100100-lg01".parse().unwrap();

    let raw = rusqlite::Connection::open(&path).unwrap();
    let runs: i64 = raw
        .query_row("SELECT COUNT(*) FROM runs", [], |row| row.get(0))
        .unwrap();
    let read = Reader::open(&path).unwrap().run_usage(&run).unwrap();
    let many = Reader::open(&path).unwrap().run_usages(&[run]).unwrap();

    assert_eq!(runs, 1);
    assert_eq!(read, None);
    assert!(many.is_empty());
}

#[test]
fn the_reader_gives_back_the_usage_of_more_runs_than_one_query_may_hold() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("../fixtures/history-before-run-usage-v21-with-run.sql"))
        .unwrap();
    drop(HistoryDb::open(&path, roomy()).unwrap());
    let raw = rusqlite::Connection::open(&path).unwrap();
    let mut runs = Vec::new();
    for number in 0..1_200_u64 {
        let id = format!("run-20261120-100100-{number:04}");
        raw.execute(
            "INSERT INTO runs (run, team, task, at, agent, tier, cli, model, role, why, prompt, worktree, branch, session, cut) \
             SELECT ?1, team, task, at, agent, tier, cli, model, role, why, prompt, worktree, branch, session, cut \
             FROM runs WHERE run = 'run-20261120-100100-lg01'",
            [&id],
        )
        .unwrap();
        raw.execute(
            "INSERT INTO run_usage (run, first_context, last_context, tokens_written, cost_usd) VALUES (?1, ?2, NULL, NULL, NULL)",
            rusqlite::params![&id, i64::try_from(number).unwrap()],
        )
        .unwrap();
        runs.push(id.parse::<agent_core::ids::RunId>().unwrap());
    }

    let read = Reader::open(&path).unwrap().run_usages(&runs).unwrap();

    assert_eq!(read.len(), 1_200);
    let last = runs.last().unwrap();
    assert_eq!(read[last].first_context, Some(1_199));
}
