//! A history written before the reminder to commit keeps its logs when it is upgraded, and takes the two events of the
//! reminder.

use std::collections::BTreeMap;

use agent_core::history::Cut;
use agent_core::log::{LogEvent, LogLevel, LogRecord};
use agent_runtime::history::{HistoryDb, Reader, Store};

use super::support::{at, roomy};

/// An entry of the daemon's log for `event` at minute `minute`.
fn entry(event: LogEvent, minute: i64) -> LogRecord {
    LogRecord {
        at: at(minute, 0),
        module: "agent::daemon::owner::commit_reminder".to_owned(),
        event,
        team: None,
        task: None,
        message: "a run is reminded to commit".to_owned(),
        fields: BTreeMap::from([("lines".to_owned(), serde_json::json!(412))]),
        cut: Cut::default(),
    }
}

#[test]
fn a_history_of_version_23_keeps_its_logs_when_upgraded_and_takes_the_reminder_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("../fixtures/history-before-commit-reminder.sql"))
        .unwrap();

    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    db.write_logs(&[entry(LogEvent::CommitReminded, 2), entry(LogEvent::CommitFollowed, 3)])
        .expect("the events of the reminder are stored");
    drop(db);

    let version: u32 = rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert!(version > 23, "the upgrade stops past 23, at {version}");
    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(
        logs.iter()
            .map(|entry| (entry.event(), entry.level()))
            .collect::<Vec<_>>(),
        vec![
            (LogEvent::DaemonStarted, LogLevel::Info),
            (LogEvent::CommitReminded, LogLevel::Info),
            (LogEvent::CommitFollowed, LogLevel::Info),
        ]
    );
}
