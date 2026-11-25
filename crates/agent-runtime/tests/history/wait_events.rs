//! Histories written before a queued task was logged as waiting for its agent keep their logs when upgraded, and take the
//! new event.

use std::collections::BTreeMap;

use agent_core::history::Cut;
use agent_core::log::{LogEvent, LogLevel, LogRecord};
use agent_runtime::history::{HistoryDb, Reader, Store};

use super::support::{at, roomy};

/// An entry of the daemon's log for `event` at minute `minute`.
fn entry(event: LogEvent, minute: i64) -> LogRecord {
    LogRecord {
        at: at(minute, 0),
        module: "agent::daemon::owner".to_owned(),
        event,
        team: None,
        task: None,
        message: "a task of the team".to_owned(),
        fields: BTreeMap::from([("agent".to_owned(), serde_json::json!("claude-1"))]),
        cut: Cut::default(),
    }
}

#[test]
fn a_history_of_version_17_keeps_its_logs_when_upgraded_and_takes_the_wait_of_a_task_for_its_agent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("../fixtures/history-before-wait-events.sql"))
        .unwrap();

    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    db.write_logs(&[entry(LogEvent::TaskWaitsForAgent, 2)])
        .expect("the wait of a task for its agent is stored");
    drop(db);

    let version: u32 = rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 24);
    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(
        logs.iter()
            .map(|entry| (entry.event(), entry.level()))
            .collect::<Vec<_>>(),
        vec![(LogEvent::TaskWaitsForAgent, LogLevel::Info)]
    );
}
