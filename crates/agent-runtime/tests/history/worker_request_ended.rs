//! The log event of the end of a worker's request is kept by a history written before it existed.

use agent_runtime::history::{HistoryDb, Reader};

use super::support::roomy;

/// A history of version 28 in `path`: the first version that has `worker_wait_returned`, without `worker_request_ended`.
fn history_of_version_28(path: &std::path::Path) -> Result<(), rusqlite::Error> {
    let raw = rusqlite::Connection::open(path)?;
    raw.execute_batch(include_str!("../fixtures/history-before-worker-recovery.sql"))?;
    raw.execute_batch(include_str!("../../src/history/026-commit-cut.sql"))?;
    raw.execute_batch(include_str!("../../src/history/027-worker-request-recovery.sql"))?;
    raw.execute_batch(include_str!("../../src/history/028-worker-wait.sql"))?;
    raw.execute_batch("PRAGMA user_version = 28;")
}

#[test]
fn a_history_of_version_28_is_upgraded_and_keeps_its_logs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    history_of_version_28(&path).unwrap();

    drop(HistoryDb::open(&path, roomy()).unwrap());

    let version: u32 = rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 38);
    assert_eq!(Reader::open(&path).unwrap().daemon_logs().unwrap().len(), 1);
}

#[test]
fn an_upgraded_history_stores_the_event_of_the_end_of_a_workers_request() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    history_of_version_28(&path).unwrap();
    drop(HistoryDb::open(&path, roomy()).unwrap());

    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(
        "INSERT INTO daemon_logs (id, first_at, at, level, module, event, team, task, run, message, fields, count, cut) \
         VALUES (2, '2026-11-25T09:01:00Z', '2026-11-25T09:01:00Z', 'info', 'agent::daemon::owner', 'worker_request_ended', \
         NULL, NULL, NULL, 'a request of a worker ended', '{}', 1, '')",
    )
    .expect("the event is admitted by the upgraded history");
}
