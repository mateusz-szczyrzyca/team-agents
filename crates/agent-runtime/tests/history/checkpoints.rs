//! A history written before checkpoints existed keeps its rows when it is upgraded, and then takes the record of a
//! checkpoint and the log event of a checkpoint that failed.

use agent_core::log::LogEvent;
use agent_runtime::history::{HistoryDb, Reader};
use rusqlite::Connection;

use super::support::roomy;

/// The rows of the tables the migration recreates, in a form that compares equal before and after it.
fn rows(raw: &Connection) -> rusqlite::Result<(Vec<String>, Vec<String>)> {
    let records = raw
        .prepare("SELECT sequence || '|' || kind || '|' || coalesce(key, '') || '|' || at || '|' || team || '|' || coalesce(payload, '') FROM history_records ORDER BY sequence")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    let logs = raw
        .prepare(
            "SELECT id || '|' || level || '|' || event || '|' || message || '|' || fields FROM daemon_logs ORDER BY id",
        )?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    Ok((records, logs))
}

#[test]
fn a_version_32_history_keeps_its_rows_and_accepts_a_checkpoint_and_its_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(include_str!("../fixtures/history-before-checkpoints.sql"))
        .unwrap();
    let before = rows(&raw).unwrap();
    assert_eq!(before.0.len(), 2);
    drop(raw);

    drop(HistoryDb::open(&path, roomy()).unwrap());

    let raw = Connection::open(&path).unwrap();
    let version: u32 = raw.pragma_query_value(None, "user_version", |row| row.get(0)).unwrap();
    assert_eq!(version, 34);
    assert_eq!(rows(&raw).unwrap(), before);
    raw.execute(
        "INSERT INTO history_records (kind, key, at, team, payload, cut) VALUES ('checkpoint', NULL, '2026-11-26T09:01:00Z', 'team-20261126-100000-t0t0', '{\"event\":\"checkpoint\"}', '')",
        [],
    )
    .unwrap();
    raw.execute("INSERT INTO daemon_logs (id, first_at, at, level, module, event, message, fields, count, cut) VALUES (11, '2026-11-26T09:01:00Z', '2026-11-26T09:01:00Z', 'warn', 'agent::daemon::owner', 'checkpoint_failed', 'recorded', '{}', 1, '')", []).unwrap();
    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(
        logs.iter()
            .filter(|row| row.event() == LogEvent::CheckpointFailed)
            .count(),
        1
    );
    assert_eq!(logs.len(), 5);
}

#[test]
fn a_checkpoint_of_an_upgraded_history_is_read_with_its_pull_request_and_draft() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(include_str!("../fixtures/history-before-checkpoints.sql"))
        .unwrap();
    drop(raw);
    drop(HistoryDb::open(&path, roomy()).unwrap());
    let round = "round-20261126-100000-r0r0";
    let payload = format!(
        r#"{{"event":"checkpoint","round":"{round}","task":"task-20261126-100000-t0t0","job":"item","tier":"normal","cause":"pull_opened","pull_request":7,"head":"0123456789abcdef0123456789abcdef01234567","draft":true,"phase":"in_progress","snapshot":"refs/agents/snapshots/{round}/task-20261126-100000-t0t0/c1"}}"#
    );
    let raw = Connection::open(&path).unwrap();
    raw.execute(
        "INSERT INTO history_records (kind, key, at, team, payload, cut) VALUES ('checkpoint', NULL, '2026-11-26T09:01:00Z', 'team-20261126-100000-t0t0', ?1, '')",
        [&payload],
    )
    .unwrap();
    let sequence = raw.last_insert_rowid();
    raw.execute(
        "INSERT INTO history_record_rounds (sequence, position, round) VALUES (?1, 1, ?2)",
        rusqlite::params![sequence, round],
    )
    .unwrap();
    drop(raw);

    let kept = Reader::open(&path)
        .unwrap()
        .checkpoints_of_round(&round.parse().unwrap())
        .unwrap();

    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].pull_request.map(|number| number.get().get()), Some(7));
    assert!(kept[0].draft);
    assert_eq!(
        kept[0].snapshot.as_deref(),
        Some("refs/agents/snapshots/round-20261126-100000-r0r0/task-20261126-100000-t0t0/c1")
    );
}
