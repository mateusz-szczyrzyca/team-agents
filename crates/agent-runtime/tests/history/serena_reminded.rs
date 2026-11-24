//! A history predating the reminder keeps its records when it is upgraded, and the upgraded file accepts the record of
//! a reminder and reads it back.

use agent_core::history::{HistoryEvent, Record};
use agent_core::ids::{RunId, TaskId, TeamId};
use agent_core::kinds::CliKind;
use agent_runtime::history::{HistoryDb, Reader, Store};

use super::support::roomy;

#[test]
fn a_version_22_history_keeps_its_records_and_takes_the_record_of_a_reminder() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("../fixtures/history-before-serena-reminded.sql"))
        .unwrap();
    let team = TeamId::try_from("team-20261124-100000-t0t0".to_owned()).unwrap();
    let before: usize = Reader::open(&path).unwrap().team_records(&team).unwrap().len();

    let db = HistoryDb::open(&path, roomy()).unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();

    assert!(version > 22, "the upgrade stops past 22, at {version}");
    drop(db);
    let after: Vec<Record> = Reader::open(&path)
        .unwrap()
        .team_records(&team)
        .unwrap()
        .into_iter()
        .map(|stored| stored.record)
        .collect();
    assert_eq!(after.len(), before);
    assert!(matches!(
        after.as_slice(),
        [Record::Event(event)] if matches!(event.event, HistoryEvent::Notified { .. })
    ));
}

#[test]
fn the_upgraded_history_reads_back_a_record_of_a_reminder() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("../fixtures/history-before-serena-reminded.sql"))
        .unwrap();
    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    let other = TeamId::try_from("team-20261124-110000-t1t1".to_owned()).unwrap();
    let event = HistoryEvent::SerenaReminded {
        task: TaskId::try_from("task-20261124-100000-t1t1".to_owned()).unwrap(),
        run: RunId::try_from("run-20261124-100001-r1r1".to_owned()).unwrap(),
        searches: 40,
        cli: CliKind::Claude,
    };
    let record = Record::Event(Box::new(agent_core::history::EventRecord {
        at: super::support::at(600, 0),
        team: other.clone(),
        rounds: Vec::new(),
        event,
    }));
    db.write(&[record]).expect("the upgraded history takes a reminder");
    drop(db);

    let read = Reader::open(&path).unwrap().team_records(&other).unwrap();

    let events: Vec<&HistoryEvent> = read
        .iter()
        .filter_map(|stored| match &stored.record {
            Record::Event(event) => Some(&event.event),
            _ => None,
        })
        .collect();
    assert!(
        matches!(
            events.as_slice(),
            [HistoryEvent::SerenaReminded {
                searches: 40,
                cli: CliKind::Claude,
                ..
            }]
        ),
        "{events:?}"
    );
}
