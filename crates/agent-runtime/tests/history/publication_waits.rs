//! Migration eight: adds `publication_waits` warning event to `daemon_logs`,
//! preserving existing records, daemon logs, links, search, and foreign keys.

use std::collections::BTreeMap;

use agent_core::history::Cut;
use agent_core::log::{LogEvent, LogRecord, LogTeamScope};
use agent_runtime::history::schema::MIGRATIONS;
use agent_runtime::history::{HistoryDb, Reader, Store};
use rusqlite::Connection;

use super::support::{at, complete, roomy};

#[test]
fn version_seven_fixture_remains_readable_and_migrates_to_version_eight() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");

    // Populate a database with the frozen version 7 schema fixture
    Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("fixtures/schema-v7.sql"))
        .unwrap();

    let world = complete(42);
    let mut db_v7 = HistoryDb::open_with(&path, &MIGRATIONS[..7], roomy()).unwrap();
    db_v7.write(&world.records()).unwrap();

    // Write a scoped baseline daemon log linked to team and open round
    db_v7
        .write_logs(&[LogRecord {
            at: at(1, 0),
            module: "agent::daemon".to_owned(),
            event: LogEvent::BusDown,
            team: Some(LogTeamScope::new(
                world.team.team.clone(),
                [world.rounds[0].round.clone()],
            )),
            task: None,
            message: "bus is down".to_owned(),
            fields: BTreeMap::new(),
            cut: Cut::default(),
        }])
        .unwrap();
    drop(db_v7);

    // Verify raw user_version is 7
    let raw = Connection::open(&path).unwrap();
    let v7: u32 = raw.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
    assert_eq!(v7, 7);

    // In version 7, SQLite CHECK constraint refuses publication_waits
    let check_fails = raw.execute_batch(
        "INSERT INTO daemon_logs (id, first_at, at, level, module, event, message, fields, count, cut) \
         VALUES (999, '2026-11-16T00:00:00Z', '2026-11-16T00:00:00Z', 'warn', 'agent::daemon', \
         'publication_waits', 'waits', '{}', 1, '');",
    );
    assert!(check_fails.is_err(), "v7 schema must reject publication_waits");
    assert!(matches!(
        check_fails,
        Err(rusqlite::Error::SqliteFailure(source, _))
            if source.code == rusqlite::ErrorCode::ConstraintViolation
                && source.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_CHECK
    ));
    drop(raw);

    // Before migration, Reader can read existing records and logs with round links
    let reader_v7 = Reader::open(&path).unwrap();
    let runs_before = reader_v7.runs_of_team(&world.team.team).unwrap();
    let logs_before = reader_v7.daemon_logs().unwrap();
    assert_eq!(logs_before.len(), 1);
    assert_eq!(logs_before[0].event(), LogEvent::BusDown);
    assert_eq!(
        reader_v7.daemon_logs_for_round(&world.rounds[0].round).unwrap().len(),
        1
    );
    assert_eq!(reader_v7.daemon_logs_for_team(&world.team.team).unwrap().len(), 1);
    drop(reader_v7);

    assert_migrated_v8_state(&path, &world, &runs_before);
}

fn assert_migrated_v8_state(
    path: &std::path::Path,
    world: &super::support::World,
    runs_before: &[agent_runtime::history::RunRow],
) {
    // Opening with HistoryDb::open migrates to version 8
    let mut db_v8 = HistoryDb::open(path, roomy()).unwrap();

    // Write a coalesced identical log and a publication_waits log entry in migrated database
    db_v8
        .write_logs(&[
            LogRecord {
                at: at(1, 500_000_000),
                module: "agent::daemon".to_owned(),
                event: LogEvent::BusDown,
                team: Some(LogTeamScope::new(
                    world.team.team.clone(),
                    [world.rounds[0].round.clone()],
                )),
                task: None,
                message: "bus is down".to_owned(),
                fields: BTreeMap::new(),
                cut: Cut::default(),
            },
            LogRecord {
                at: at(2, 0),
                module: "agent::daemon".to_owned(),
                event: LogEvent::PublicationWaits,
                team: Some(LogTeamScope::new(
                    world.team.team.clone(),
                    [world.rounds[0].round.clone()],
                )),
                task: None,
                message: "publication waits".to_owned(),
                fields: BTreeMap::new(),
                cut: Cut::default(),
            },
        ])
        .unwrap();
    drop(db_v8);

    // Verify raw user_version is 8
    let raw = Connection::open(path).unwrap();
    let v8: u32 = raw.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
    assert_eq!(v8, 8);

    let violations: i64 = raw
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(violations, 0, "no foreign key violations after migration 8");

    // Verify FTS search index is working on the logs
    let search_count: i64 = raw
        .query_row(
            "SELECT count(*) FROM history_search WHERE history_search MATCH 'bus'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(search_count >= 1, "history search matches bus down log");
    drop(raw);

    // Reader verifies all records and both old and new logs are present
    let reader_v8 = Reader::open(path).unwrap();
    assert_eq!(reader_v8.runs_of_team(&world.team.team).unwrap(), runs_before);
    let logs_after = reader_v8.daemon_logs().unwrap();
    assert_eq!(logs_after.len(), 2);
    assert_eq!(logs_after[0].event(), LogEvent::BusDown);
    assert_eq!(
        logs_after[0].count().get(),
        2,
        "coalesced count incremented on migrated table"
    );
    assert_eq!(logs_after[1].event(), LogEvent::PublicationWaits);

    // Check daemon_log_rounds links survived and work for new log
    let round_logs = reader_v8.daemon_logs_for_round(&world.rounds[0].round).unwrap();
    assert_eq!(
        round_logs.len(),
        2,
        "daemon_log_rounds links preserved across migration"
    );
    assert_eq!(reader_v8.daemon_logs_for_team(&world.team.team).unwrap().len(), 2);
}

#[test]
fn version_seven_upgrade_keeps_records_links_search_and_foreign_keys() {
    for seed in 0..10 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let old: Vec<_> = MIGRATIONS.iter().copied().filter(|m| m.version <= 7).collect();
        let mut db = HistoryDb::open_with(&path, &old, roomy()).unwrap();
        let world = complete(seed);
        db.write(&world.records()).unwrap();
        db.write_logs(&[LogRecord {
            at: at(10, 0),
            module: "agent::daemon".to_owned(),
            event: LogEvent::BusDown,
            team: Some(LogTeamScope::new(
                world.team.team.clone(),
                [world.rounds[0].round.clone()],
            )),
            task: None,
            message: "bus down".to_owned(),
            fields: BTreeMap::new(),
            cut: Cut::default(),
        }])
        .unwrap();
        drop(db);

        let reader = Reader::open(&path).unwrap();
        let runs_before = reader.runs_of_team(&world.team.team).unwrap();
        let logs_before = reader.daemon_logs().unwrap();
        assert_eq!(
            reader.daemon_logs_for_round(&world.rounds[0].round).unwrap().len(),
            1,
            "seed {seed}"
        );
        drop(reader);

        let raw = Connection::open(&path).unwrap();
        let search_before: i64 = raw
            .query_row("SELECT count(*) FROM history_search", [], |r| r.get(0))
            .unwrap();
        drop(raw);

        // Migrate to version 8
        let mut upgraded = HistoryDb::open(&path, roomy()).unwrap();
        upgraded
            .write_logs(&[LogRecord {
                at: at(20, 0),
                module: "agent::daemon".to_owned(),
                event: LogEvent::PublicationWaits,
                team: Some(LogTeamScope::new(
                    world.team.team.clone(),
                    [world.rounds[0].round.clone()],
                )),
                task: None,
                message: "branch waits".to_owned(),
                fields: BTreeMap::new(),
                cut: Cut::default(),
            }])
            .unwrap();
        drop(upgraded);

        let reader = Reader::open(&path).unwrap();
        assert_eq!(
            reader.runs_of_team(&world.team.team).unwrap(),
            runs_before,
            "seed {seed}"
        );
        let logs_after = reader.daemon_logs().unwrap();
        assert_eq!(logs_after.len(), logs_before.len() + 1, "seed {seed}");
        assert_eq!(
            logs_after.last().unwrap().event(),
            LogEvent::PublicationWaits,
            "seed {seed}"
        );
        assert_eq!(
            reader.daemon_logs_for_round(&world.rounds[0].round).unwrap().len(),
            2,
            "seed {seed}"
        );

        let raw = Connection::open(&path).unwrap();
        let version: u32 = raw.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version, 8, "seed {seed}");
        let violations: i64 = raw
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(violations, 0, "seed {seed}");
        let search_after: i64 = raw
            .query_row("SELECT count(*) FROM history_search", [], |r| r.get(0))
            .unwrap();
        assert!(search_after > search_before, "seed {seed}");
    }
}
