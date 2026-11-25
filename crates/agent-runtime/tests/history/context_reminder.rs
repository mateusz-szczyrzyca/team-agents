//! A history written before the reminder of a full context keeps its logs when it is upgraded, and takes the event of
//! the reminder.

use std::collections::BTreeMap;

use agent_core::history::Cut;
use agent_core::log::{LogEvent, LogLevel, LogRecord};
use agent_runtime::history::{HistoryDb, Reader, Store};

use super::support::{at, roomy};

/// An entry of the daemon's log for `event` at minute `minute`.
fn entry(event: LogEvent, minute: i64) -> LogRecord {
    LogRecord {
        at: at(minute, 0),
        module: "agent::daemon::owner::context_reminder".to_owned(),
        event,
        team: None,
        task: None,
        message: "a session is reminded of its full context".to_owned(),
        fields: BTreeMap::from([("context".to_owned(), serde_json::json!(130_000))]),
        cut: Cut::default(),
    }
}

#[test]
fn a_history_of_version_24_keeps_its_logs_when_upgraded_and_takes_the_context_event() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("../fixtures/history-before-context-reminder.sql"))
        .unwrap();

    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    db.write_logs(&[entry(LogEvent::ContextReminded, 2)])
        .expect("the event of the reminder is stored");
    drop(db);

    let version: u32 = rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert!(version > 24, "the upgrade stops past 24, at {version}");
    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(
        logs.iter()
            .map(|entry| (entry.event(), entry.level()))
            .collect::<Vec<_>>(),
        vec![
            (LogEvent::DaemonStarted, LogLevel::Info),
            (LogEvent::ContextReminded, LogLevel::Info),
        ]
    );
}
