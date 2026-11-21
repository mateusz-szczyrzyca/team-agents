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
    assert_eq!(version, 17);
    assert_eq!(Reader::open(&path).unwrap().round_records(round).unwrap(), before);
    drop(db);
}

#[test]
fn a_truncated_split_decision_reason_reads_back_with_its_cut_metadata() {
    use agent_core::history::{EventRecord, HistoryEvent, Record, SplitDecision};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let world = complete(8);
    let round = world.rounds[0].round.clone();
    let event = EventRecord {
        at: super::support::at(500, 0),
        team: world.team.team.clone(),
        rounds: vec![round.clone()],
        event: HistoryEvent::SplitDecision {
            task: world.runs[0].start.task.clone(),
            item: "1.1".parse().unwrap(),
            decision: SplitDecision::Corrected,
            corrections: 1,
            question: None,
            reason: "日本語日本語".to_owned(),
        },
    };
    let mut db = HistoryDb::open(&path, super::support::limit(10)).unwrap();
    db.write(&[
        Record::Team(Box::new(world.team.clone())),
        Record::Round(Box::new(world.rounds[0].clone())),
        Record::Event(Box::new(event.clone())),
    ])
    .unwrap();
    let mut expected = event;
    if let HistoryEvent::SplitDecision { reason, .. } = &mut expected.event {
        *reason = "日本語".to_owned();
    }
    let expected = Record::Event(Box::new(expected));

    let read = Reader::open(&path).unwrap().round_records(&round);

    assert!(
        matches!(&read, Ok(records) if records.last().map(|row| (&row.record, row.cut.fields().collect::<Vec<_>>()))
            == Some((&expected, vec!["reason"]))),
        "{read:?}"
    );
}
