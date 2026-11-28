//! The log event of a request returned because no worker took it is kept by a history written before it existed.

use agent_runtime::history::{HistoryDb, Reader};

use super::support::roomy;

/// A history of version 27 in `path`: the first version that has the recovery events, without `worker_wait_returned`.
fn history_of_version_27(path: &std::path::Path) -> rusqlite::Result<()> {
    let raw = rusqlite::Connection::open(path)?;
    raw.execute_batch(include_str!("../fixtures/history-before-worker-recovery.sql"))?;
    raw.execute_batch(include_str!("../../src/history/026-commit-cut.sql"))?;
    raw.execute_batch(include_str!("../../src/history/027-worker-request-recovery.sql"))?;
    raw.execute_batch("PRAGMA user_version = 27;")
}

#[test]
fn a_history_of_version_27_is_upgraded_and_keeps_its_logs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    history_of_version_27(&path).unwrap();

    drop(HistoryDb::open(&path, roomy()).unwrap());

    let version: u32 = rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 38);
    assert_eq!(Reader::open(&path).unwrap().daemon_logs().unwrap().len(), 1);
}

#[test]
fn an_upgraded_history_stores_the_event_of_a_request_returned_for_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    history_of_version_27(&path).unwrap();
    drop(HistoryDb::open(&path, roomy()).unwrap());

    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(
        "INSERT INTO daemon_logs (id, first_at, at, level, module, event, team, task, run, message, fields, count, cut) \
         VALUES (2, '2026-11-25T09:01:00Z', '2026-11-25T09:01:00Z', 'info', 'agent::daemon::owner', 'worker_wait_returned', \
         NULL, NULL, NULL, 'nobody took the request', '{}', 1, '')",
    )
    .expect("the event is admitted by the upgraded history");
}
