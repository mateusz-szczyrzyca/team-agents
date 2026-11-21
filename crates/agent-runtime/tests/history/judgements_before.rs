//! A history written before judgements (schema 15) preserves its records and upgrades to schema 16.

use agent_core::ids::ItemId;
use agent_core::plan::judge::JudgementState;
use agent_runtime::history::{HistoryDb, JudgementRecord, Reader, Store};

use super::support::{at, complete, roomy};

#[test]
fn history_before_judgements_upgrades_and_retains_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(include_str!("../fixtures/history-before-step-judgements.sql"))
        .unwrap();
    drop(raw);

    let old: Vec<_> = agent_runtime::history::schema::MIGRATIONS
        .iter()
        .copied()
        .filter(|migration| migration.version <= 15)
        .collect();
    let world = complete(7);
    let mut db = HistoryDb::open_with(&path, &old, roomy()).unwrap();
    db.write(&world.records()).unwrap();
    drop(db);

    let round = &world.rounds[0].round;
    let before = Reader::open(&path).unwrap().round_records(round).unwrap();
    let task = world.runs[0].start.task.clone();
    let team = world.runs[0].start.team.clone();

    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    let version: u32 = rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 16);

    let reader = Reader::open(&path).unwrap();
    assert_eq!(reader.round_records(round).unwrap(), before);
    assert_eq!(reader.step_judgements(&task).unwrap(), Vec::new());

    let record = JudgementRecord {
        task: task.clone(),
        team: team.clone(),
        run: Some(world.runs[0].start.run.clone()),
        at: at(5_000, 0),
        step: "1.1".parse::<ItemId>().unwrap(),
        state: JudgementState::Confirmed,
        check: Some("exists ok.txt".to_owned()),
        reason: None,
        duration_ms: 10,
        output_path: None,
    };
    db.write_judgements(std::slice::from_ref(&record)).unwrap();
    drop(db);

    assert_eq!(reader.step_judgements(&task).unwrap(), vec![record]);
}
