//! A history from before named no-tool events preserves its logs, round links, search and cursor on upgrade.

use std::collections::BTreeMap;

use agent_core::history::Cut;
use agent_core::log::{LogEvent, LogLevel, LogRecord};
use agent_runtime::history::{HistoryDb, Reader, Store};

use super::support::{at, roomy};

#[test]
fn version_20_history_keeps_its_logs_and_accepts_both_no_tool_events_after_upgrade() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("../fixtures/history-before-no-tool-events.sql"))
        .unwrap();
    drop(connection);

    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 31);
    drop(connection);
    let records = [LogEvent::RunNoToolRetry, LogEvent::RunNoToolFailed].map(|event| LogRecord {
        at: at(2, 0),
        module: "agent::daemon::owner".to_owned(),
        event,
        team: None,
        task: None,
        message: event.as_str().to_owned(),
        fields: BTreeMap::new(),
        cut: Cut::default(),
    });
    db.write_logs(&records)
        .expect("the migrated history accepts both no-tool events");
    drop(db);

    let connection = rusqlite::Connection::open(&path).unwrap();
    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(
        logs.iter()
            .map(|entry| (entry.event(), entry.level(), entry.message()))
            .collect::<Vec<_>>(),
        vec![
            (LogEvent::TeamChanged, LogLevel::Info, "existing team"),
            (LogEvent::RunNoToolRetry, LogLevel::Info, "run_no_tool_retry"),
            (LogEvent::RunNoToolFailed, LogLevel::Warn, "run_no_tool_failed"),
        ]
    );
    let links: Vec<(i64, i64, String)> = connection
        .prepare("SELECT * FROM daemon_log_rounds ORDER BY log, position")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(links, vec![(7, 1, "round-20261123-000000-bbbb".to_owned())]);
    let cursor: i64 = connection
        .query_row(
            "SELECT value FROM history_cursors WHERE name = 'daemon_logs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(cursor, 9);
    let search: Vec<String> = connection
        .prepare("SELECT body FROM history_search WHERE log IS NOT NULL ORDER BY log")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(search, vec!["existing team", "run_no_tool_retry", "run_no_tool_failed"]);
}
