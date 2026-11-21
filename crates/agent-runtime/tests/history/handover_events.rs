//! Histories written before the hand-over of a task was logged keep their logs when upgraded, and take the new event.

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
        fields: BTreeMap::from([("from".to_owned(), serde_json::json!("claude-1"))]),
        cut: Cut::default(),
    }
}

#[test]
fn a_history_of_version_15_keeps_its_logs_when_upgraded_and_takes_the_hand_over_of_a_task() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("../fixtures/history-before-handover-events.sql"))
        .unwrap();
    let old: Vec<_> = agent_runtime::history::schema::MIGRATIONS
        .iter()
        .copied()
        .filter(|migration| migration.version <= 15)
        .collect();
    let mut db = HistoryDb::open_with(&path, &old, roomy()).unwrap();
    db.write_logs(&[entry(LogEvent::TaskStatusChanged, 1)]).unwrap();
    drop(db);

    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    db.write_logs(&[entry(LogEvent::TaskHandedOver, 2)])
        .expect("the hand-over is stored");
    drop(db);

    let version: u32 = rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 16);
    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(
        logs.iter()
            .map(|entry| (entry.event(), entry.level()))
            .collect::<Vec<_>>(),
        vec![
            (LogEvent::TaskStatusChanged, LogLevel::Info),
            (LogEvent::TaskHandedOver, LogLevel::Info)
        ]
    );
}
