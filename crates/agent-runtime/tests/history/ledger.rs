//! The ledger: every record has a place in the order it was written in, the events of a round are kept as typed JSON,
//! the records of a round are found by their links, and a file made before the ledger is given one by the migration
//! or, when it is read without one, by a view.

use std::num::NonZeroU32;

use agent_core::history::{CheckOutcome, EventRecord, HistoryEvent, Record, SequencedRecord};
use agent_core::ids::RoundId;
use agent_core::state::{CommitId, PrNumber};
use agent_runtime::history::schema::MIGRATIONS;
use agent_runtime::history::{HistoryDb, ReadError, Reader, Store};
use rusqlite::{Connection, ErrorCode};

use super::support::{World, at, complete, database, limit, roomy};

fn pr(n: u32) -> PrNumber {
    PrNumber::new(NonZeroU32::new(n).unwrap())
}

/// A commit of forty copies of `digit`.
fn head(digit: char) -> CommitId {
    digit.to_string().repeat(40).parse().unwrap()
}

/// `event` of the team of `world`, at `seconds`, for `rounds`.
fn record(world: &World, seconds: i64, rounds: &[RoundId], event: HistoryEvent) -> Record {
    Record::Event(Box::new(EventRecord {
        at: at(seconds, 0),
        team: world.team.team.clone(),
        rounds: rounds.to_vec(),
        event,
    }))
}

/// Events of several kinds for the first round of `world`, all at the same second, in an order that is not the order of
/// their kinds.
fn events(world: &World) -> Vec<Record> {
    let round = vec![world.rounds[0].round.clone()];
    let task = world.runs[0].start.task.clone();
    [
        HistoryEvent::PullOpened {
            number: pr(7),
            branch: "agents/a/b".parse().unwrap(),
            head: head('a'),
            title: "Title — 日本語".to_owned(),
            body: "line\n".repeat(50),
        },
        HistoryEvent::QuestionAsked {
            task: task.clone(),
            number: NonZeroU32::new(1).unwrap(),
            text: "## 1. Which?".to_owned(),
        },
        HistoryEvent::PullReady { pr: pr(7) },
        HistoryEvent::Checked {
            pr: Some(pr(7)),
            head: head('a'),
            issues: vec!["no test".to_owned()],
            outcome: CheckOutcome::SentBack {
                pass: NonZeroU32::new(1).unwrap(),
                of: 2,
            },
        },
        HistoryEvent::PullPushed {
            pr: pr(7),
            head: head('b'),
        },
    ]
    .into_iter()
    .map(|event| record(world, 500, &round, event))
    .collect()
}

/// What `reader` has of `records`, without the places.
fn plain(read: &[SequencedRecord]) -> Vec<Record> {
    read.iter().map(|stored| stored.record.clone()).collect()
}

#[test]
fn every_record_of_a_team_is_read_back_in_the_order_it_was_written() {
    for seed in 0..30 {
        let (_dir, path, mut db) = database();
        let world = complete(seed);
        let written = world.records();
        db.write(&written).unwrap();

        let read = Reader::open(&path).unwrap().team_records(&world.team.team).unwrap();

        assert_eq!(plain(&read), written, "seed={seed}");
        let places: Vec<u64> = read.iter().map(|stored| stored.sequence).collect();
        assert!(
            places.windows(2).all(|pair| pair[0] < pair[1]),
            "seed={seed}: {places:?}"
        );
        assert!(read.iter().all(|stored| stored.team == world.team.team), "seed={seed}");
    }
}

#[test]
fn a_round_has_the_start_of_its_team_its_opening_and_what_the_runs_that_worked_for_it_wrote() {
    for seed in 0..30 {
        let (_dir, path, mut db) = database();
        let world = complete(seed);
        db.write(&world.records()).unwrap();
        let round = world.rounds[0].round.clone();
        let working: Vec<_> = world
            .runs
            .iter()
            .filter(|run| run.start.rounds.contains(&round))
            .map(|run| run.start.run.clone())
            .collect();
        let expected: Vec<Record> = world
            .records()
            .into_iter()
            .filter(|record| match record {
                Record::Team(_) => true,
                Record::Round(opening) => opening.round == round,
                Record::Run(start) => working.contains(&start.run),
                Record::Launch(launch) => working.contains(&launch.run),
                Record::Step(step) => working.contains(&step.run),
                Record::RunEnd(end) => working.contains(&end.run),
                Record::Changes(changes) => working.contains(&changes.run),
                Record::Session(session) => working.contains(&session.run),
                Record::Event(_) => false,
            })
            .collect();

        let read = Reader::open(&path).unwrap().round_records(&round).unwrap();

        assert_eq!(plain(&read), expected, "seed={seed}");
    }
}

#[test]
fn a_round_the_history_does_not_know_has_no_records() {
    let (_dir, path, mut db) = database();
    let world = complete(1);
    db.write(&world.records()).unwrap();
    let other: RoundId = "round-20990101-000000-zzzz".parse().unwrap();

    assert_eq!(Reader::open(&path).unwrap().round_records(&other).unwrap(), []);
}

#[test]
fn events_are_read_back_equal_and_in_the_order_written_whatever_their_time_and_kind() {
    for seed in 0..30 {
        let (_dir, path, mut db) = database();
        let world = complete(seed);
        let mut written = world.records();
        db.write(&written).unwrap();
        let mut later = events(&world);
        later.push(record(&world, 100, &[], HistoryEvent::TeamEnded {}));
        db.write(&later[..2]).unwrap();
        db.write(&later[2..]).unwrap();
        written.extend(later);

        let read = Reader::open(&path).unwrap().team_records(&world.team.team).unwrap();

        assert_eq!(plain(&read), written, "seed={seed}");
        let places: Vec<u64> = read.iter().map(|stored| stored.sequence).collect();
        assert!(
            places.windows(2).all(|pair| pair[0] < pair[1]),
            "seed={seed}: {places:?}"
        );
    }
}

#[test]
fn an_event_belongs_to_the_rounds_it_names_and_one_without_rounds_to_its_team_only() {
    let (_dir, path, mut db) = database();
    let world = complete(3);
    db.write(&world.records()).unwrap();
    let round = world.rounds[0].round.clone();
    db.write(&events(&world)).unwrap();
    db.write(&[record(&world, 900, &[], HistoryEvent::TeamEnded {})])
        .unwrap();
    let reader = Reader::open(&path).unwrap();

    let of_round = reader.round_records(&round).unwrap();
    let of_team = reader.team_records(&world.team.team).unwrap();

    let events_of_round = of_round
        .iter()
        .filter(|stored| matches!(stored.record, Record::Event(_)))
        .count();
    assert_eq!(events_of_round, 5);
    assert!(
        of_round
            .iter()
            .filter(|stored| matches!(stored.record, Record::Event(_)))
            .all(|stored| stored.rounds == [round.clone()])
    );
    assert_eq!(
        of_team
            .iter()
            .filter(|stored| matches!(stored.record, Record::Event(_)))
            .count(),
        6
    );
    assert!(of_team.last().is_some_and(|stored| matches!(
        &stored.record,
        Record::Event(event) if event.event == HistoryEvent::TeamEnded {} && event.rounds.is_empty()
    )));
}

#[test]
fn an_event_with_a_key_is_one_record_however_often_it_is_written_and_one_without_is_every_time() {
    let (_dir, path, mut db) = database();
    let world = complete(4);
    db.write(&world.records()).unwrap();
    let round = [world.rounds[0].round.clone()];
    let ready = record(&world, 10, &round, HistoryEvent::PullReady { pr: pr(7) });
    let notice = |seconds| {
        record(
            &world,
            seconds,
            &round,
            HistoryEvent::Notified {
                task: None,
                title: "t".to_owned(),
                body: "b".to_owned(),
            },
        )
    };

    db.write(&[ready.clone(), notice(11)]).unwrap();
    db.write(&[ready.clone(), notice(11)]).unwrap();

    let read = Reader::open(&path).unwrap().team_records(&world.team.team).unwrap();
    let kept: Vec<Record> = plain(&read).into_iter().skip(world.records().len()).collect();
    assert_eq!(kept, [ready, notice(11), notice(11)]);
}

#[test]
fn two_teams_may_have_an_event_with_the_same_key() {
    let (_dir, path, mut db) = database();
    let first = complete(5);
    let second = complete(6);
    db.write(&first.records()).unwrap();
    db.write(&second.records()).unwrap();
    let ready = |world: &World| record(world, 20, &[], HistoryEvent::PullReady { pr: pr(7) });

    db.write(&[ready(&first), ready(&second)]).unwrap();

    let reader = Reader::open(&path).unwrap();
    for world in [&first, &second] {
        let read = reader.team_records(&world.team.team).unwrap();
        assert_eq!(read.last().map(|stored| stored.record.clone()), Some(ready(world)));
    }
}

#[test]
fn a_text_of_an_event_over_the_limit_is_cut_at_a_character_and_its_field_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let mut db = HistoryDb::open(&path, limit(10)).unwrap();
    let world = complete(8);
    db.write(&world.records()).unwrap();
    let round = [world.rounds[0].round.clone()];
    let opened = HistoryEvent::PullOpened {
        number: pr(9),
        branch: "agents/a/b".parse().unwrap(),
        head: head('a'),
        title: "日本語日本語".to_owned(),
        body: "short".to_owned(),
    };
    db.write(&[record(&world, 30, &round, opened)]).unwrap();

    let read = Reader::open(&path).unwrap().team_records(&world.team.team).unwrap();

    let stored = read.last().unwrap();
    let Record::Event(event) = &stored.record else {
        panic!("not an event: {stored:?}");
    };
    let HistoryEvent::PullOpened { title, body, .. } = &event.event else {
        panic!("not an opening: {event:?}");
    };
    assert_eq!(title, "日本語");
    assert_eq!(body, "short");
    assert_eq!(stored.cut.fields().collect::<Vec<_>>(), ["title"]);
}

#[test]
fn a_lost_event_whose_detail_is_over_the_limit_is_cut_and_read_back_through_its_round_and_team() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let mut db = HistoryDb::open(&path, limit(10)).unwrap();
    let world = complete(8);
    db.write(&world.records()).unwrap();
    let round = world.rounds[0].round.clone();
    let lost = HistoryEvent::Lost {
        what: "step".to_owned(),
        cause: "write_disk_full".to_owned(),
        detail: "日本語日本語".to_owned(),
    };
    db.write(&[record(&world, 30, std::slice::from_ref(&round), lost)])
        .unwrap();
    let reader = Reader::open(&path).unwrap();

    for read in [
        reader.round_records(&round).unwrap(),
        reader.team_records(&world.team.team).unwrap(),
    ] {
        let stored = read.last().unwrap();
        let Record::Event(event) = &stored.record else {
            panic!("not an event: {stored:?}");
        };
        let HistoryEvent::Lost { what, cause, detail } = &event.event else {
            panic!("not a lost event: {event:?}");
        };
        assert_eq!(
            (what.as_str(), cause.as_str(), detail.as_str()),
            ("step", "write_disk_full", "日本語")
        );
        assert_eq!(stored.cut.fields().collect::<Vec<_>>(), ["detail"]);
    }
}

/// A file of version two with `world` written to it, and its path.
fn version_two(world: &World) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let mut db = HistoryDb::open_with(&path, &MIGRATIONS[..2], roomy()).unwrap();
    db.write(&world.records()).unwrap();
    (dir, path)
}

#[test]
fn the_migration_gives_the_records_of_a_version_two_file_a_sequence_in_the_order_of_time() {
    for seed in 0..20 {
        let world = complete(seed);
        let (_dir, path) = version_two(&world);

        drop(HistoryDb::open(&path, roomy()).unwrap());

        let read = Reader::open(&path).unwrap().team_records(&world.team.team).unwrap();
        let written = world.records();
        assert_eq!(read.len(), written.len(), "seed={seed}");
        assert!(
            written.iter().all(|record| plain(&read).contains(record)),
            "seed={seed}"
        );
        let times: Vec<_> = read.iter().map(|stored| stored.at).collect();
        assert!(
            times.windows(2).all(|pair| pair[0] <= pair[1]),
            "seed={seed}: {times:?}"
        );
        let places: Vec<u64> = read.iter().map(|stored| stored.sequence).collect();
        assert_eq!(
            places,
            (1..=u64::try_from(read.len()).unwrap()).collect::<Vec<_>>(),
            "seed={seed}"
        );
        let raw = Connection::open(&path).unwrap();
        assert_eq!(
            raw.pragma_query_value::<u32, _>(None, "user_version", |row| row.get(0))
                .unwrap(),
            3
        );
    }
}

#[test]
fn a_file_of_version_two_is_read_with_a_ledger_made_on_the_fly_and_is_not_changed() {
    for seed in 0..20 {
        let world = complete(seed);
        let (_dir, path) = version_two(&world);
        let before = std::fs::read(&path).unwrap();

        let read = Reader::open(&path).unwrap().team_records(&world.team.team).unwrap();

        let written = world.records();
        assert_eq!(read.len(), written.len(), "seed={seed}");
        assert!(
            written.iter().all(|record| plain(&read).contains(record)),
            "seed={seed}"
        );
        let times: Vec<_> = read.iter().map(|stored| stored.at).collect();
        assert!(
            times.windows(2).all(|pair| pair[0] <= pair[1]),
            "seed={seed}: {times:?}"
        );
        let round = world.rounds[0].round.clone();
        assert!(
            Reader::open(&path)
                .unwrap()
                .round_records(&round)
                .unwrap()
                .iter()
                .any(|stored| matches!(&stored.record, Record::Round(opening) if opening.round == round)),
            "seed={seed}"
        );
        assert_eq!(std::fs::read(&path).unwrap(), before, "seed={seed}");
        let raw = Connection::open(&path).unwrap();
        assert_eq!(
            raw.pragma_query_value::<u32, _>(None, "user_version", |row| row.get(0))
                .unwrap(),
            2
        );
    }
}

#[test]
fn the_ledger_refuses_in_sql_an_unknown_kind_a_payload_of_another_kind_and_a_key_twice() {
    let (_dir, path, mut db) = database();
    let world = complete(9);
    db.write(&world.records()).unwrap();
    let raw = Connection::open(&path).unwrap();
    let team = world.team.team.as_str();
    let insert = |kind: &str, key: Option<&str>, payload: Option<&str>| {
        raw.execute(
            "INSERT INTO history_records (kind, key, at, team, payload, cut) VALUES (?1, ?2, '2026-11-11T19:30:00.000000000Z', ?3, ?4, '')",
            rusqlite::params![kind, key, team, payload],
        )
    };
    let violation = |result: Result<usize, rusqlite::Error>| result.unwrap_err().sqlite_error_code();

    assert_eq!(
        violation(insert("teleported", Some("k"), None)),
        Some(ErrorCode::ConstraintViolation)
    );
    assert_eq!(
        violation(insert("pull_ready", Some("pr 1"), None)),
        Some(ErrorCode::ConstraintViolation)
    );
    assert_eq!(
        violation(insert(
            "pull_ready",
            Some("pr 1"),
            Some(r#"{"event":"pull_closed","pr":1}"#)
        )),
        Some(ErrorCode::ConstraintViolation)
    );
    assert_eq!(
        violation(insert("pull_ready", Some("pr 1"), Some("not json"))),
        Some(ErrorCode::ConstraintViolation)
    );
    assert_eq!(
        violation(insert("run", Some("run-x"), Some(r#"{"event":"run"}"#))),
        Some(ErrorCode::ConstraintViolation)
    );
    assert_eq!(
        insert("pull_ready", Some("pr 1"), Some(r#"{"event":"pull_ready","pr":1}"#)).unwrap(),
        1
    );
    assert_eq!(
        violation(insert(
            "pull_ready",
            Some("pr 1"),
            Some(r#"{"event":"pull_ready","pr":1}"#)
        )),
        Some(ErrorCode::ConstraintViolation)
    );
}

#[test]
fn a_ledger_row_that_was_changed_by_hand_is_refused_when_it_is_read() {
    let (_dir, path, mut db) = database();
    let world = complete(10);
    db.write(&world.records()).unwrap();
    db.write(&events(&world)).unwrap();
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(
        "PRAGMA ignore_check_constraints = ON; \
         UPDATE history_records SET payload = '{\"event\":\"pull_closed\",\"pr\":7}' WHERE kind = 'pull_ready'",
    )
    .unwrap();

    let read = Reader::open(&path).unwrap().team_records(&world.team.team);

    assert!(matches!(
        read,
        Err(ReadError::Value {
            table: "history_records",
            column: "payload",
            ..
        })
    ));
}

#[test]
fn an_event_linked_to_a_round_of_another_team_is_refused_on_write_and_on_read() {
    use agent_runtime::history::{RecordKind, WriteError};

    let (_dir, path, mut db) = database();
    let first = complete(11);
    let second = complete(12);
    db.write(&first.records()).unwrap();
    db.write(&second.records()).unwrap();
    let foreign_round = second.rounds[0].round.clone();
    let cross_event = record(
        &first,
        50,
        std::slice::from_ref(&foreign_round),
        HistoryEvent::PullReady { pr: pr(7) },
    );

    let err = db.write(&[cross_event]).unwrap_err();
    assert!(
        matches!(
            err,
            WriteError::Rule {
                record: RecordKind::Event,
                ..
            }
        ),
        "{err:?}"
    );
    let reader = Reader::open(&path).unwrap();
    assert!(
        reader
            .round_records(&foreign_round)
            .unwrap()
            .iter()
            .all(|stored| stored.team == second.team.team)
    );

    db.write(&[record(&first, 51, &[], HistoryEvent::PullReady { pr: pr(8) })])
        .unwrap();
    let raw = Connection::open(&path).unwrap();
    raw.execute(
        "INSERT INTO history_record_rounds (sequence, position, round) \
         SELECT sequence, 1, ?1 FROM history_records WHERE team = ?2 AND kind = 'pull_ready'",
        rusqlite::params![foreign_round.as_str(), first.team.team.as_str()],
    )
    .unwrap();

    for read in [
        reader.round_records(&foreign_round),
        reader.team_records(&first.team.team),
    ] {
        assert!(
            matches!(
                read,
                Err(ReadError::Value {
                    table: "history_records",
                    column: "round",
                    ..
                })
            ),
            "{read:?}"
        );
    }
}

#[test]
fn many_steps_of_a_run_are_read_in_order_and_a_missing_step_row_is_reported() {
    use agent_core::cli::ActivityEvent;
    use agent_core::history::Step;

    let (_dir, path, mut db) = database();
    let world = complete(13);
    let round = world.rounds[0].round.clone();
    let run = world.runs[0].start.clone();
    db.write(&[
        Record::Team(Box::new(world.team.clone())),
        Record::Round(Box::new(world.rounds[0].clone())),
        Record::Run(Box::new(run.clone())),
    ])
    .unwrap();
    let steps: Vec<Record> = (1..=500_u32)
        .map(|n| {
            Record::Step(Box::new(Step {
                run: run.run.clone(),
                number: NonZeroU32::new(n).unwrap(),
                at: at(100 + i64::from(n), 0),
                event: ActivityEvent::Info(format!("step {n}")),
            }))
        })
        .collect();
    db.write(&steps).unwrap();

    let reader = Reader::open(&path).unwrap();
    let read = reader.round_records(&round).unwrap();
    let read_steps: Vec<Record> = plain(&read).into_iter().skip(3).collect();
    assert_eq!(read_steps, steps);

    let raw = Connection::open(&path).unwrap();
    raw.execute(
        "DELETE FROM steps WHERE run = ?1 AND n = 250",
        rusqlite::params![run.run.as_str()],
    )
    .unwrap();

    let broken = reader.round_records(&round);
    assert!(
        matches!(
            broken,
            Err(ReadError::Value {
                table: "history_records",
                column: "key",
                ..
            })
        ),
        "{broken:?}"
    );
}
