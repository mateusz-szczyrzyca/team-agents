//! Shift events preserve schema-37 histories, their links, search and cursors.

use std::collections::BTreeMap;

use agent_core::history::Cut;
use agent_core::log::{LogEvent, LogLevel, LogRecord};
use agent_runtime::history::{HistoryDb, Reader, Store};

use agent_core::history::MaxTextBytes;

#[test]
fn version_37_history_preserves_data_and_accepts_shift_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(include_str!("fixtures/history-before-findings.sql"))
        .unwrap();
    connection
        .execute_batch(include_str!("../src/history/037-findings.sql"))
        .unwrap();
    connection.pragma_update(None, "user_version", 37).unwrap();
    drop(connection);

    let mut db = HistoryDb::open(&path, MaxTextBytes::default()).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 38);
    drop(connection);
    let records = [LogEvent::ShiftNoFreeAgent].map(|event| LogRecord {
        at: "2026-11-23T00:00:02Z".parse().unwrap(),
        module: "agent::daemon::owner".to_owned(),
        event,
        team: None,
        task: None,
        message: event.as_str().to_owned(),
        fields: BTreeMap::new(),
        cut: Cut::default(),
    });
    db.write_logs(&records)
        .expect("the migrated history accepts the shift event");
    drop(db);

    let connection = rusqlite::Connection::open(&path).unwrap();
    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(
        logs.iter()
            .map(|entry| (entry.event(), entry.level(), entry.message()))
            .collect::<Vec<_>>(),
        vec![
            (LogEvent::TeamChanged, LogLevel::Info, "existing team"),
            (LogEvent::ShiftNoFreeAgent, LogLevel::Info, "shift_no_free_agent"),
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
    assert_eq!(cursor, 8);
    let search: Vec<String> = connection
        .prepare("SELECT body FROM history_search WHERE log IS NOT NULL ORDER BY log")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(search, vec!["existing team", "shift_no_free_agent"]);
}
