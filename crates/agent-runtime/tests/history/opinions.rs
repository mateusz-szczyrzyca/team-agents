//! Schema-eight compatibility and dormant opinion history records.
use super::support::{complete, roomy};
use agent_core::history::{EventRecord, HistoryEvent, Record, RunWhy};
use agent_core::state::PrNumber;
use agent_runtime::history::{HistoryDb, Reader, Store};
use std::num::NonZeroU32;

#[test]
fn migrates_schema_eight_with_records_links_and_search() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch(include_str!("../fixtures/history-before-second-opinions.sql"))
        .unwrap();
    drop(raw);
    let old: Vec<_> = agent_runtime::history::schema::MIGRATIONS
        .iter()
        .copied()
        .filter(|m| m.version <= 8)
        .collect();
    let world = complete(23);
    let mut db = HistoryDb::open_with(&path, &old, roomy()).unwrap();
    db.write(&world.records()).unwrap();
    drop(db);
    let before = Reader::open(&path)
        .unwrap()
        .round_records(&world.rounds[0].round)
        .unwrap();
    let raw = rusqlite::Connection::open(&path).unwrap();
    let search_before: i64 = raw
        .query_row("SELECT count(*) FROM history_search", [], |r| r.get(0))
        .unwrap();
    drop(raw);
    let db = HistoryDb::open(&path, roomy()).unwrap();
    assert_eq!(
        Reader::open(&path)
            .unwrap()
            .round_records(&world.rounds[0].round)
            .unwrap(),
        before
    );
    let raw = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        raw.query_row::<i64, _, _>("SELECT count(*) FROM history_search", [], |r| r.get(0))
            .unwrap(),
        search_before
    );
    assert_eq!(
        raw.pragma_query_value::<u32, _>(None, "user_version", |r| r.get(0))
            .unwrap(),
        9
    );
    assert_eq!(
        raw.query_row::<i64, _, _>("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0))
            .unwrap(),
        0
    );
    drop(raw);
    drop(db);
}

#[test]
fn roundtrips_opinion_keywords_and_renders_their_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let mut world = complete(23);
    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    db.write(&world.records()).unwrap();
    let pr = PrNumber::new(NonZeroU32::MIN);
    let by = "inv-opus".parse::<agent_core::ids::AgentName>().unwrap();
    let events = [
        HistoryEvent::SecondOpinionAsked {
            pr,
            number: NonZeroU32::MIN,
            pass: 1,
            by: by.clone(),
            why: "café lead reason".into(),
        },
        HistoryEvent::FindingConfirmed {
            pr,
            number: NonZeroU32::MIN,
            pass: 1,
            by: by.clone(),
            why: "café evidence confirms".into(),
        },
        HistoryEvent::FindingNotConfirmed {
            pr,
            number: NonZeroU32::new(2).unwrap(),
            pass: 1,
            by,
            why: "café evidence rejects".into(),
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
    let result = db.write(&records);
    assert!(matches!(result, Ok(())), "{result:?}");
    let actual: Vec<_> = Reader::open(&path)
        .unwrap()
        .round_records(&world.rounds[0].round)
        .unwrap()
        .into_iter()
        .filter_map(|row| match row.record {
            Record::Event(event) => Some(Record::Event(event)),
            _ => None,
        })
        .collect();
    assert_eq!(actual, records);
    world.runs[0].start.run = "run-20261111-193009-p9s2".parse().unwrap();
    world.runs[0].start.why = RunWhy::SecondOpinion { pr, pass: 1 };
    let start = Record::Run(Box::new(world.runs[0].start.clone()));
    let result = db.write(std::slice::from_ref(&start));
    assert!(matches!(result, Ok(())), "{result:?}");
    let stored = Reader::open(&path)
        .unwrap()
        .round_records(&world.rounds[0].round)
        .unwrap();
    assert_eq!(stored.last().unwrap().record, start);
    let selection = agent_core::story::Selection::Round(world.rounds[0].round.clone());
    let text = agent_core::story::render(
        &stored,
        &selection,
        agent_core::story::StoryOptions {
            prompts: agent_core::story::PromptView::Preview,
            steps: agent_core::story::StepView::LeftOut,
        },
    );
    for keyword in [
        "second_opinion_asked",
        "finding_confirmed",
        "finding_not_confirmed",
        "café evidence confirms",
        "café evidence rejects",
        "a second opinion on pull request",
    ] {
        assert!(text.contains(keyword), "missing {keyword}: {text}");
    }
    let json = agent_core::story::json_lines(&stored, &selection).join("\n");
    assert!(json.contains("second_opinion"));
    for event in records {
        let Record::Event(record) = event else {
            panic!("expected an event")
        };
        let keyword = record.event.kind().to_string();
        assert_eq!(
            keyword.parse::<agent_core::history::LostKind>().unwrap().as_str(),
            keyword
        );
        assert_eq!(record.event.identities(), vec!["inv-opus"]);
        assert!(record.event.key().unwrap().contains("finding"));
    }
}

#[test]
fn migrates_schema_nine_preserving_existing_opinion_requests_and_search() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let old: Vec<_> = agent_runtime::history::schema::MIGRATIONS
        .iter()
        .copied()
        .filter(|migration| migration.version <= 9)
        .collect();
    let world = complete(23);
    let mut db = HistoryDb::open_with(&path, &old, roomy()).unwrap();
    db.write(&world.records()).unwrap();
    let request = Record::Event(Box::new(EventRecord {
        at: world.team.at,
        team: world.team.team.clone(),
        rounds: vec![world.rounds[0].round.clone()],
        event: HistoryEvent::SecondOpinionAsked {
            pr: PrNumber::new(NonZeroU32::MIN),
            number: NonZeroU32::MIN,
            pass: 1,
            by: "claude-1".parse().unwrap(),
            why: "café legacy reason".into(),
        },
    }));
    db.write(std::slice::from_ref(&request)).unwrap();
    drop(db);
    let before = Reader::open(&path)
        .unwrap()
        .round_records(&world.rounds[0].round)
        .unwrap();
    let raw = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        raw.pragma_query_value::<u32, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        9
    );
    let search_before: i64 = raw
        .query_row("SELECT count(*) FROM history_search", [], |row| row.get(0))
        .unwrap();
    drop(raw);
    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    let raw = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        raw.pragma_query_value::<u32, _>(None, "user_version", |row| row.get(0))
            .unwrap(),
        10
    );
    assert_eq!(
        Reader::open(&path)
            .unwrap()
            .round_records(&world.rounds[0].round)
            .unwrap(),
        before
    );
    assert_eq!(
        raw.query_row::<i64, _, _>("SELECT count(*) FROM history_search", [], |row| row.get(0))
            .unwrap(),
        search_before
    );
    assert_eq!(
        raw.query_row::<i64, _, _>("SELECT count(*) FROM pragma_foreign_key_check", [], |row| row.get(0))
            .unwrap(),
        0
    );
    let event = serde_json::from_value(serde_json::json!({
        "event": "second_opinion_ended", "pr": 1, "number": 1, "pass": 1,
        "by": "inv-first", "why": "the run failed", "attempt": 1
    }))
    .unwrap();
    let ended = Record::Event(Box::new(EventRecord {
        at: world.team.at,
        team: world.team.team.clone(),
        rounds: vec![world.rounds[0].round.clone()],
        event,
    }));
    db.write(std::slice::from_ref(&ended)).unwrap();
    assert_eq!(
        Reader::open(&path)
            .unwrap()
            .round_records(&world.rounds[0].round)
            .unwrap()
            .last()
            .unwrap()
            .record,
        ended
    );
}
