//! Histories written before a worker's damaged request file was logged under a name of its own keep their logs when
//! upgraded, and take the new event.

use std::collections::BTreeMap;

use agent_core::history::Cut;
use agent_core::log::{LogEvent, LogLevel, LogRecord};
use agent_runtime::history::{HistoryDb, Reader, Store};

use super::support::{at, roomy};

/// An entry of the daemon's log for `event` at minute `minute`.
fn entry(event: LogEvent, minute: i64) -> LogRecord {
    LogRecord {
        at: at(minute, 0),
        module: "agent::daemon::owner::worker_result".to_owned(),
        event,
        team: None,
        task: None,
        message: "the file of a worker's request cannot be used".to_owned(),
        fields: BTreeMap::from([("file".to_owned(), serde_json::json!("worker-request-1.json"))]),
        cut: Cut::default(),
    }
}

#[test]
fn a_history_of_version_18_keeps_its_logs_when_upgraded_and_takes_the_damage_of_a_workers_request_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("../fixtures/history-before-worker-request-damaged.sql"))
        .unwrap();

    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    db.write_logs(&[entry(LogEvent::WorkerRequestDamaged, 2)])
        .expect("the damage of a request file is stored");
    drop(db);

    let version: u32 = rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 22);
    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(
        logs.iter()
            .map(|entry| (entry.event(), entry.level()))
            .collect::<Vec<_>>(),
        vec![(LogEvent::WorkerRequestDamaged, LogLevel::Warn)]
    );
}
