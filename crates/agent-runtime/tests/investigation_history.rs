//! Migration 33 preserves existing history and admits investigator runs and reserved events.

use agent_core::history::MaxTextBytes;
use agent_runtime::history::{HistoryDb, schema::MIGRATIONS};
use rusqlite::Connection;

#[test]
fn investigation_migration_preserves_related_history_and_accepts_new_role_and_events() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let old: Vec<_> = MIGRATIONS.iter().copied().filter(|m| m.version <= 32).collect();
    drop(HistoryDb::open_with(&path, &old, MaxTextBytes::default()).unwrap());
    let raw = Connection::open(&path).unwrap();
    populate_history(&raw).unwrap();
    let before: Vec<(String, String)> = raw
        .prepare("SELECT kind, key FROM history_records ORDER BY sequence")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(before.len(), 3);
    drop(raw);
    drop(HistoryDb::open(&path, MaxTextBytes::default()).unwrap());
    let raw = Connection::open(&path).unwrap();
    let after: Vec<(String, String)> = raw
        .prepare("SELECT kind, key FROM history_records ORDER BY sequence")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(after, before);
    let related: (String, String, String, String, String, i64) = raw
        .query_row(
            "SELECT r.prompt, rr.round, s.session, e.final_message, lr.round, c.value FROM runs r \
        JOIN run_rounds rr USING(run) JOIN run_sessions s USING(run) JOIN run_ends e USING(run) \
        JOIN daemon_log_rounds lr ON lr.log = 7 JOIN history_cursors c ON c.name = 'daemon_logs'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .unwrap();
    assert_eq!(
        related,
        (
            "Legacy 日本語".into(),
            "legacy-round".into(),
            "legacy-session".into(),
            "Legacy answer".into(),
            "legacy-round".into(),
            7
        )
    );
    let hits: i64 = raw
        .query_row(
            "SELECT count(*) FROM history_search WHERE history_search MATCH 'searchable'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hits, 1);
    let linked: i64 = raw
        .query_row(
            "SELECT count(*) FROM history_record_rounds WHERE round = 'legacy-round'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(linked, 3);
    let prompt_hits: i64 = raw
        .query_row(
            "SELECT count(*) FROM history_search WHERE history_search MATCH '日本語'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(prompt_hits, 1);
    let broken: i64 = raw
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(broken, 0);
    assert_eq!(raw.execute("UPDATE runs SET role = 'investigator'", []).unwrap(), 1);
    for (id, event, level) in [
        (8, "investigation_started", "info"),
        (9, "investigation_answered", "info"),
        (10, "investigation_kind_unknown", "warn"),
    ] {
        assert_eq!(raw.execute("INSERT INTO daemon_logs (id, first_at, at, level, module, event, message, fields, count, cut) VALUES (?, '2026-11-26T00:00:00Z', '2026-11-26T00:00:00Z', ?, 'agent::daemon', ?, 'new event', '{}', 1, '')", rusqlite::params![id, level, event]).unwrap(), 1);
    }
    let cursor: i64 = raw
        .query_row(
            "SELECT value FROM history_cursors WHERE name = 'daemon_logs'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cursor, 10);
}

/// Populates the old schema with linked rows and searchable records.
fn populate_history(raw: &Connection) -> rusqlite::Result<()> {
    raw.execute_batch(r"
        PRAGMA foreign_keys = ON;
        INSERT INTO runs (run, team, task, at, agent, tier, cli, role, why, prompt, worktree, branch, cut)
        VALUES ('legacy-run', 'legacy-team', 'legacy-task', '2026-11-26T00:00:00Z', 'inv-codex',
        'investigator', 'codex', 'reviewer', 'first_run', 'Legacy 日本語', '/tmp/worktree', 'main', '');
        INSERT INTO run_rounds VALUES ('legacy-run', 1, 'legacy-round');
        INSERT INTO run_sessions VALUES ('legacy-run', 1, '2026-11-26T00:00:00Z', 'legacy-session');
        INSERT INTO run_ends VALUES ('legacy-run', '2026-11-26T00:01:00Z', 'reported', 'DONE', NULL, NULL, 'Legacy answer', '');
        INSERT INTO daemon_logs (id, first_at, at, level, module, event, message, fields, count, cut)
        VALUES (7, '2026-11-26T00:00:00Z', '2026-11-26T00:00:00Z', 'info', 'agent::daemon',
        'daemon_started', 'legacy searchable message', '{}', 1, '');
        INSERT INTO daemon_log_rounds VALUES (7, 1, 'legacy-round');
        INSERT INTO history_records (kind, key, at, team, cut) VALUES
        ('run', 'legacy-run', '2026-11-26T00:00:00Z', 'legacy-team', ''),
        ('session', 'legacy-run/1', '2026-11-26T00:00:00Z', 'legacy-team', ''),
        ('run_end', 'legacy-run', '2026-11-26T00:01:00Z', 'legacy-team', '');
        INSERT INTO history_record_rounds
        SELECT sequence, 1, 'legacy-round' FROM history_records;
    ")
}
