//! Recovery log events preserve a history written before interrupted worker requests were recorded.

use std::collections::BTreeMap;

use agent_core::history::Cut;
use agent_core::log::{LogEvent, LogLevel, LogRecord};
use agent_runtime::history::{HistoryDb, Reader, Store};

use super::support::{at, roomy};

/// An entry of the daemon's log for `event` at minute `minute`.
fn entry(event: LogEvent, minute: i64) -> LogRecord {
    LogRecord {
        at: at(minute, 0),
        module: "agent::daemon::owner::owned_request".to_owned(),
        event,
        team: None,
        task: None,
        message: "a worker request is recovered".to_owned(),
        fields: BTreeMap::new(),
        cut: Cut::default(),
    }
}

#[test]
fn a_history_of_version_25_keeps_its_logs_and_accepts_worker_recovery_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("../fixtures/history-before-worker-recovery.sql"))
        .unwrap();

    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    let version: u32 = rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert!(version > 25, "the upgrade stops past 25, at {version}");
    db.write_logs(&[
        entry(LogEvent::WorkerRequestReturned, 2),
        entry(LogEvent::WorkerRequestRequeued, 3),
    ])
    .expect("recovery events are stored");
    drop(db);

    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(
        logs.iter()
            .map(|entry| (entry.event(), entry.level()))
            .collect::<Vec<_>>(),
        vec![
            (LogEvent::DaemonStarted, LogLevel::Info),
            (LogEvent::WorkerRequestReturned, LogLevel::Info),
            (LogEvent::WorkerRequestRequeued, LogLevel::Info),
        ]
    );
}

#[test]
fn a_history_of_version_26_keeps_its_commit_cut_and_accepts_worker_recovery_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(include_str!("../fixtures/history-before-worker-recovery.sql"))
        .unwrap();
    raw.execute_batch(include_str!("../../src/history/026-commit-cut.sql"))
        .unwrap();
    raw.execute_batch(
        "INSERT INTO daemon_logs (id, first_at, at, level, module, event, team, task, run, message, fields, count, cut) \
         VALUES (2, '2026-11-25T09:01:00Z', '2026-11-25T09:01:00Z', 'info', 'agent::daemon::owner', 'commit_cut', \
         NULL, NULL, NULL, 'a run that ignored the reminder to commit is cut', '{}', 1, ''); \
         PRAGMA user_version = 26;",
    )
    .unwrap();
    drop(raw);

    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    let version: u32 = rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 32);
    db.write_logs(&[
        entry(LogEvent::WorkerRequestReturned, 2),
        entry(LogEvent::WorkerRequestRequeued, 3),
    ])
    .expect("recovery events are stored after commit_cut");
    drop(db);

    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(
        logs.iter()
            .map(|entry| (entry.event(), entry.level()))
            .collect::<Vec<_>>(),
        vec![
            (LogEvent::DaemonStarted, LogLevel::Info),
            (LogEvent::CommitCut, LogLevel::Info),
            (LogEvent::WorkerRequestReturned, LogLevel::Info),
            (LogEvent::WorkerRequestRequeued, LogLevel::Info),
        ]
    );
}
