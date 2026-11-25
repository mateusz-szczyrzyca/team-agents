//! Existing histories retain their records while learning both refusal takeover events.

use super::support::roomy;
use agent_runtime::history::{HistoryDb, Reader};

#[test]
fn a_version_29_history_keeps_its_logs_and_accepts_both_refusal_takeover_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(include_str!("../fixtures/history-before-task-agent-changed.sql"))
        .unwrap();
    let original: (i64, String, String) = raw
        .query_row("SELECT id, event, fields FROM daemon_logs", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    drop(raw);
    drop(HistoryDb::open(&path, roomy()).unwrap());
    let raw = rusqlite::Connection::open(&path).unwrap();
    let retained: (i64, String, String) = raw
        .query_row("SELECT id, event, fields FROM daemon_logs", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap();
    assert_eq!(retained, original);
    for (id, event) in [
        (2, "task_agent_changed"),
        (3, "task_wait_for_free_agent"),
        (4, "worker_request_ended"),
    ] {
        raw.execute("INSERT INTO daemon_logs (id, first_at, at, level, module, event, message, fields, count, cut) VALUES (?1, '2026-11-25T09:01:00Z', '2026-11-25T09:01:00Z', 'info', 'agent::daemon::owner', ?2, 'recorded', '{}', 1, '')", rusqlite::params![id, event]).unwrap();
    }
    let rows = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(rows.len(), 4);
    for event in [
        agent_core::log::LogEvent::TaskAgentChanged,
        agent_core::log::LogEvent::TaskWaitForFreeAgent,
        agent_core::log::LogEvent::WorkerRequestEnded,
    ] {
        assert_eq!(rows.iter().filter(|row| row.event() == event).count(), 1);
    }
}

#[test]
fn a_version_30_history_preserves_both_worktree_events_when_refusal_events_are_added() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(include_str!("../fixtures/history-before-refusal-takeover-30.sql"))
        .unwrap();
    drop(raw);
    drop(HistoryDb::open(&path, roomy()).unwrap());
    let rows = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(rows.len(), 3);
    for event in [
        agent_core::log::LogEvent::WorktreeSwitched,
        agent_core::log::LogEvent::WorktreeSwitchRefused,
    ] {
        assert_eq!(rows.iter().filter(|row| row.event() == event).count(), 1);
    }
}
