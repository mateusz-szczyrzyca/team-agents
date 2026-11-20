//! Histories written before proposal decisions retain their complete records when upgraded.

use agent_runtime::history::{HistoryDb, Reader, Store};

use super::support::{complete, roomy};

#[test]
fn proposal_decision_migration_preserves_the_previous_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(include_str!("../fixtures/history-before-split-decisions.sql"))
        .unwrap();
    drop(raw);
    let old: Vec<_> = agent_runtime::history::schema::MIGRATIONS
        .iter()
        .copied()
        .filter(|migration| migration.version <= 14)
        .collect();
    let world = complete(7);
    let mut db = HistoryDb::open_with(&path, &old, roomy()).unwrap();
    db.write(&world.records()).unwrap();
    drop(db);
    let round = &world.rounds[0].round;
    let before = Reader::open(&path).unwrap().round_records(round).unwrap();

    let db = HistoryDb::open(&path, roomy()).unwrap();

    let version: u32 = rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 15);
    assert_eq!(Reader::open(&path).unwrap().round_records(round).unwrap(), before);
    drop(db);
}
