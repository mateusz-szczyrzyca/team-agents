//! Histories from before direct publication questions retain their records and accept the new status pairs.

use std::collections::BTreeMap;

use agent_core::history::{Cut, MaxTextBytes};
use agent_core::log::{LogEvent, LogRecord};
use agent_runtime::history::{HistoryDb, Reader, Store};
use serde_json::json;

#[test]
fn pre_question_transition_history_keeps_old_logs_and_reads_direct_question_transitions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(include_str!("fixtures/history-before-plans.sql"))
        .unwrap();
    raw.execute(
        "INSERT INTO history_cursors (name, value) VALUES ('daemon_logs', 0)",
        [],
    )
    .unwrap();
    drop(raw);
    let old: Vec<_> = agent_runtime::history::schema::MIGRATIONS
        .iter()
        .copied()
        .filter(|migration| migration.version <= 12)
        .collect();
    let record = |second, from: &str, to: &str| LogRecord {
        at: format!("2026-11-21T12:00:{second:02}Z").parse().unwrap(),
        module: "agent::daemon::owner".to_owned(),
        event: LogEvent::TaskStatusChanged,
        team: None,
        task: None,
        message: "task status changed".to_owned(),
        fields: BTreeMap::from([("from".to_owned(), json!(from)), ("to".to_owned(), json!(to))]),
        cut: Cut::default(),
    };
    let mut db = HistoryDb::open_with(&path, &old, MaxTextBytes::default()).unwrap();
    db.write_logs(&[record(0, "running", "waiting")]).unwrap();
    drop(db);
    let previous = Reader::open(&path).unwrap().daemon_logs().unwrap();
    let mut db = HistoryDb::open(&path, MaxTextBytes::default()).unwrap();
    assert_eq!(Reader::open(&path).unwrap().daemon_logs().unwrap(), previous);
    let added = [record(1, "waiting", "questions"), record(2, "failed", "questions")];
    db.write_logs(&added).unwrap();
    drop(db);
    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(logs.len(), 3);
    assert_eq!(logs.first(), previous.first());
    for (entry, expected) in logs.iter().skip(1).zip(added) {
        assert_eq!(entry.event(), expected.event);
        assert_eq!(entry.fields(), &expected.fields);
    }
}
