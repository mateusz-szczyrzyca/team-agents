//! A history written before plans were kept (schema twelve, `fixtures/history-before-plans.sql`, the pinned schema of
//! that version) is migrated to the plans' version: its records stay, it has no plan, and it keeps plans from then on.

use agent_runtime::history::{HistoryDb, PlanRead, PlanRecord, Reader, Store};

use super::support::{at, complete, roomy};

#[test]
fn a_history_written_before_plans_keeps_its_records_and_keeps_plans_after_its_migration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(include_str!("../fixtures/history-before-plans.sql"))
        .unwrap();
    drop(raw);
    let old: Vec<_> = agent_runtime::history::schema::MIGRATIONS
        .iter()
        .copied()
        .filter(|migration| migration.version <= 12)
        .collect();
    let world = complete(7);
    let mut db = HistoryDb::open_with(&path, &old, roomy()).unwrap();
    db.write(&world.records()).unwrap();
    drop(db);
    let round = &world.rounds[0].round;
    let before = Reader::open(&path).unwrap().round_records(round).unwrap();
    let task = world.runs[0].start.task.clone();

    let mut db = HistoryDb::open(&path, roomy()).unwrap();

    let version: u32 = rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 16);
    let reader = Reader::open(&path).unwrap();
    assert_eq!(reader.round_records(round).unwrap(), before);
    assert_eq!(reader.plan_versions(&task).unwrap(), Vec::new());
    let record = PlanRecord {
        task: task.clone(),
        team: world.runs[0].start.team.clone(),
        run: Some(world.runs[0].start.run.clone()),
        at: at(4_000, 0),
        read: PlanRead::Missing,
    };
    db.write_plans(std::slice::from_ref(&record)).unwrap();
    drop(db);
    assert_eq!(Reader::open(&path).unwrap().plan_versions(&task).unwrap(), vec![record]);
}
