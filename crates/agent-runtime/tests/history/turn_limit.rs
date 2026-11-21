//! Turn-limit records and an upgrade of a populated version-six history.

use agent_core::history::RunWhy;
use agent_core::state::RunEnd;
use agent_runtime::history::schema::MIGRATIONS;
use agent_runtime::history::{HistoryDb, Reader, Store};
use rusqlite::Connection;

use super::support::{complete, database, roomy};

#[test]
fn turn_limit_end_and_continuation_reason_round_trip() {
    for seed in 0..20 {
        let (_dir, path, mut db) = database();
        let mut world = complete(seed);
        world.runs[0].start.why = RunWhy::TurnLimit;
        world.runs[0].end.as_mut().unwrap().end = RunEnd::TurnLimit;
        db.write(&world.records())
            .unwrap_or_else(|error| panic!("seed {seed}: {error:?}"));
        let rows = Reader::open(&path).unwrap().runs_of_team(&world.team.team).unwrap();
        let row = rows
            .iter()
            .find(|row| row.start.value.run == world.runs[0].start.run)
            .unwrap();
        assert_eq!(row.start.value, world.runs[0].start, "seed {seed}");
        assert_eq!(
            row.end.as_ref().unwrap().value,
            *world.runs[0].end.as_ref().unwrap(),
            "seed {seed}"
        );
    }
}

#[test]
fn version_six_upgrade_keeps_records_links_search_and_foreign_keys() {
    for seed in 0..10 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let old: Vec<_> = MIGRATIONS.iter().copied().filter(|m| m.version <= 6).collect();
        let mut db = HistoryDb::open_with(&path, &old, roomy()).unwrap();
        let world = complete(seed);
        db.write(&world.records()).unwrap();
        drop(db);
        let reader = Reader::open(&path).unwrap();
        let before = reader.runs_of_team(&world.team.team).unwrap();
        let steps = reader.steps(&world.runs[0].start.run).unwrap();
        let changes = reader.changes(&world.runs[0].start.run).unwrap();
        drop(reader);
        let raw = Connection::open(&path).unwrap();
        let search_before: i64 = raw
            .query_row("SELECT count(*) FROM history_search", [], |r| r.get(0))
            .unwrap();
        drop(raw);
        drop(HistoryDb::open(&path, roomy()).unwrap());
        let reader = Reader::open(&path).unwrap();
        assert_eq!(reader.runs_of_team(&world.team.team).unwrap(), before, "seed {seed}");
        assert_eq!(reader.steps(&world.runs[0].start.run).unwrap(), steps, "seed {seed}");
        assert_eq!(
            reader.changes(&world.runs[0].start.run).unwrap(),
            changes,
            "seed {seed}"
        );
        let raw = Connection::open(&path).unwrap();
        let version: u32 = raw.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version, 16, "seed {seed}");
        let violations: i64 = raw
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(violations, 0, "seed {seed}");
        let search_after: i64 = raw
            .query_row("SELECT count(*) FROM history_search", [], |r| r.get(0))
            .unwrap();
        assert_eq!(search_before, search_after, "seed {seed}");
    }
}

#[test]
fn turn_limit_fields_of_other_variants_are_refused_by_sql_and_reader() {
    for (table, column, value) in [
        ("runs", "why_item", "1.1"),
        ("runs", "why_asked_by", "run-20261116-000000-0000"),
        ("runs", "why_pass", "1"),
        ("runs", "why_pr", "1"),
        ("run_ends", "outcome", "DONE"),
        ("run_ends", "exit_code", "0"),
        ("run_ends", "resets_at", "2026-11-16T00:00:00Z"),
    ] {
        let (_dir, path, mut db) = database();
        let mut world = complete(88);
        world.runs[0].start.why = RunWhy::TurnLimit;
        world.runs[0].end.as_mut().unwrap().end = RunEnd::TurnLimit;
        db.write(&world.records()).unwrap();
        let raw = Connection::open(&path).unwrap();
        let sql = format!("UPDATE {table} SET {column} = ?1 WHERE run = ?2");
        let id = world.runs[0].start.run.as_str();
        let error = raw.execute(&sql, [value, id]).unwrap_err();
        assert_eq!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::ConstraintViolation)
        );
        raw.execute_batch("PRAGMA ignore_check_constraints = ON").unwrap();
        raw.execute(&sql, [value, id]).unwrap();
        let error = Reader::open(&path).unwrap().run(&world.runs[0].start.run).unwrap_err();
        assert!(
            matches!(error, agent_runtime::history::ReadError::Value { table: t, column: c, .. } if t == table && c == column),
            "{table}.{column}: {error:?}"
        );
    }
}
