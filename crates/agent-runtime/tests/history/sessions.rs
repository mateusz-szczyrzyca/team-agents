//! Session observations are append-only, ordered, migration-safe and independently validated on read.

use std::num::NonZeroU32;
use std::str::FromStr;

use agent_core::history::{Record, RunSession};
use agent_runtime::history::{HistoryDb, ReadError, Reader, Store};
use rand::rngs::Xoshiro256PlusPlus;
use rand::{RngExt, SeedableRng};
use rusqlite::Connection;

use super::support::{at, complete, database, roomy};

#[test]
fn seeded_session_observations_round_trip_in_order_and_duplicate_delivery_is_idempotent() {
    for seed in 0..40 {
        let (_dir, path, mut db) = database();
        let world = complete(seed);
        db.write(&world.records()).unwrap();
        let run = world.runs[0].start.run.clone();
        let mut rng = Xoshiro256PlusPlus::seed_from_u64(seed);
        let expected: Vec<_> = (1..=rng.random_range(2..20))
            .map(|n| RunSession {
                run: run.clone(),
                number: NonZeroU32::new(n).unwrap(),
                at: at(i64::from(n), 0),
                session: format!("session-{}-{n}", rng.random::<u64>()).parse().unwrap(),
            })
            .collect();
        let records: Vec<_> = expected
            .iter()
            .rev()
            .map(|s| Record::Session(Box::new(s.clone())))
            .collect();
        db.write(&records).unwrap();
        db.write(&records).unwrap();
        assert_eq!(
            Reader::open(&path).unwrap().sessions(&run).unwrap(),
            expected,
            "seed={seed}"
        );
    }
}

#[test]
fn version_one_fixture_remains_readable_and_migrates_without_losing_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("fixtures/schema-v1.sql"))
        .unwrap();
    let world = complete(7);
    let run = world.runs[0].start.run.clone();
    let mut db = HistoryDb::open_with(&path, &agent_runtime::history::schema::MIGRATIONS[..1], roomy()).unwrap();
    db.write(&world.records()).unwrap();
    drop(db);
    let old = Reader::open(&path).unwrap();
    assert_eq!(old.sessions(&run).unwrap(), Vec::<RunSession>::new());
    let original = old.run(&run).unwrap();
    drop(old);
    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    let observation = RunSession {
        run: run.clone(),
        number: NonZeroU32::new(1).unwrap(),
        at: at(10, 0),
        session: "session-after-upgrade".parse().unwrap(),
    };
    db.write(&[Record::Session(Box::new(observation.clone()))]).unwrap();
    let reader = Reader::open(&path).unwrap();
    assert_eq!(reader.run(&run).unwrap(), original);
    assert_eq!(reader.sessions(&run).unwrap(), [observation]);
    let raw = Connection::open(&path).unwrap();
    assert_eq!(
        raw.pragma_query_value::<u32, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        2
    );
}

#[test]
fn session_order_numbers_are_checked_in_sql_and_again_when_read() {
    let (_dir, path, mut db) = database();
    let run = complete(1).runs.remove(0).start;
    db.write(&[
        Record::Run(Box::new(run.clone())),
        Record::Session(Box::new(RunSession {
            run: run.run.clone(),
            number: NonZeroU32::new(1).unwrap(),
            at: at(1, 0),
            session: "session-1".parse().unwrap(),
        })),
    ])
    .unwrap();
    assert_eq!(Reader::open(&path).unwrap().sessions(&run.run).unwrap().len(), 1);
    let raw = Connection::open(&path).unwrap();
    assert_eq!(
        raw.execute("UPDATE run_sessions SET n = 0", [])
            .unwrap_err()
            .sqlite_error_code(),
        Some(rusqlite::ErrorCode::ConstraintViolation)
    );
    raw.execute_batch("PRAGMA ignore_check_constraints = ON; UPDATE run_sessions SET n = 0")
        .unwrap();
    assert!(matches!(
        Reader::open(&path).unwrap().sessions(&run.run),
        Err(ReadError::Value {
            table: "run_sessions",
            column: "n",
            ..
        })
    ));
}

#[test]
fn malformed_session_ids_are_refused_when_read_and_orphans_are_refused_when_written() {
    let (_dir, path, mut db) = database();
    let run = complete(1).runs.remove(0).start;
    let observation = RunSession {
        run: run.run.clone(),
        number: NonZeroU32::new(1).unwrap(),
        at: at(1, 0),
        session: "session-1".parse().unwrap(),
    };
    assert!(matches!(
        db.write(&[Record::Session(Box::new(observation.clone()))]),
        Err(agent_runtime::history::WriteError::Sqlite { .. })
    ));
    db.write(&[
        Record::Run(Box::new(run.clone())),
        Record::Session(Box::new(observation)),
    ])
    .unwrap();
    Connection::open(&path)
        .unwrap()
        .execute("UPDATE run_sessions SET session = ''", [])
        .unwrap();
    assert!(matches!(
        Reader::open(&path).unwrap().sessions(&run.run),
        Err(ReadError::Value {
            table: "run_sessions",
            column: "session",
            ..
        })
    ));
    assert_eq!(
        Reader::open(&path)
            .unwrap()
            .sessions(&agent_core::ids::RunId::from_str("run-20261114-100000-none").unwrap())
            .unwrap(),
        Vec::<RunSession>::new()
    );
}
