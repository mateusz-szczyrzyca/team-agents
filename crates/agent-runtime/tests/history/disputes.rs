//! Finding decisions survive a populated version-seven database migration.
use super::support::{complete, database};
use agent_core::history::{EventRecord, HistoryEvent, Record};
use agent_core::state::PrNumber;
use agent_runtime::history::{HistoryDb, Reader, Store};
use std::num::NonZeroU32;

#[test]
fn finding_decisions_round_trip_and_previous_database_keeps_records_and_search() {
    let (_dir, path, mut db) = database();
    let world = complete(13);
    db.write(&world.records()).unwrap();
    let fields = || {
        (
            PrNumber::new(NonZeroU32::MIN),
            NonZeroU32::MIN,
            2,
            "inv-codex".parse::<agent_core::ids::AgentName>().unwrap(),
            "café evidence".to_owned(),
        )
    };
    let (pr, number, pass, by, why) = fields();
    let events = [
        HistoryEvent::FindingExplained {
            pr,
            number,
            pass,
            by: by.clone(),
            why: why.clone(),
        },
        HistoryEvent::FindingAccepted {
            pr,
            number,
            pass,
            by: by.clone(),
            why: why.clone(),
        },
        HistoryEvent::FindingRejected {
            pr,
            number,
            pass,
            by: by.clone(),
            why: why.clone(),
        },
        HistoryEvent::FindingRaised {
            pr,
            number,
            pass,
            by,
            why,
        },
    ];
    let records: Vec<_> = events
        .into_iter()
        .map(|event| {
            Record::Event(Box::new(EventRecord {
                at: world.team.at,
                team: world.team.team.clone(),
                rounds: vec![world.rounds[0].round.clone()],
                event,
            }))
        })
        .collect();
    let write = db.write(&records);
    assert!(matches!(write, Ok(())), "{write:?}");
    let stored = Reader::open(&path)
        .unwrap()
        .round_records(&world.rounds[0].round)
        .unwrap();
    let actual: Vec<_> = stored
        .into_iter()
        .filter_map(|r| match r.record {
            Record::Event(e) => Some(Record::Event(e)),
            _ => None,
        })
        .collect();
    assert_eq!(actual, records);
}

#[test]
fn previous_history_fixture_migrates_with_round_links_search_and_records_intact() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(include_str!("../fixtures/history-before-disputes.sql"))
        .unwrap();
    drop(raw);
    let old: Vec<_> = agent_runtime::history::schema::MIGRATIONS
        .iter()
        .copied()
        .filter(|m| m.version <= 7)
        .collect();
    let world = complete(17);
    let mut db = HistoryDb::open_with(&path, &old, super::support::roomy()).unwrap();
    db.write(&world.records()).unwrap();
    drop(db);
    let before = Reader::open(&path)
        .unwrap()
        .round_records(&world.rounds[0].round)
        .unwrap();
    let search_before: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row("SELECT count(*) FROM history_search", [], |r| r.get(0))
        .unwrap();
    drop(HistoryDb::open(&path, super::support::roomy()).unwrap());
    let raw = rusqlite::Connection::open(&path).unwrap();
    let version: u32 = raw.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
    assert_eq!(version, 10);
    assert_eq!(
        Reader::open(&path)
            .unwrap()
            .round_records(&world.rounds[0].round)
            .unwrap(),
        before
    );
    let search_after: i64 = raw
        .query_row("SELECT count(*) FROM history_search", [], |r| r.get(0))
        .unwrap();
    assert_eq!(search_before, search_after);
    let violations: i64 = raw
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(violations, 0);
}
