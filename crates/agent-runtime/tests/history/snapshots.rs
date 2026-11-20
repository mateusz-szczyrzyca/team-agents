//! Snapshots of worktrees in the history: a snapshot is written as a row of its own, read back by its round in the order of
//! its tasks and numbers, written once when it is handed in twice, and an older history is migrated to keep it.

use std::collections::BTreeMap;
use std::str::FromStr;

use agent_core::history::{Cut, Record, SnapshotRecord};
use agent_core::ids::{RoundId, TaskId};
use agent_core::log::{LogEvent, LogLevel, LogRecord};
use agent_core::state::CommitId;
use agent_runtime::history::{HistoryDb, Reader, Store};

use super::support::{at, complete, database, roomy};

/// The round the snapshots of these tests are kept under.
const ROUND: &str = "round-20261120-100000-r1r1";

/// A task whose name sorts first.
const FIRST_TASK: &str = "task-20261120-100000-aaaa";

/// A task whose name sorts after [`FIRST_TASK`].
const SECOND_TASK: &str = "task-20261120-100000-bbbb";

/// A commit ID of the tests, distinct by its last digit.
fn commit(digit: char) -> CommitId {
    CommitId::from_str(&digit.to_string().repeat(40)).expect("a commit ID")
}

/// A snapshot of `task` with `number`, kept under the round of these tests.
fn snapshot(task: &str, number: u32) -> Record {
    Record::Snapshot(Box::new(SnapshotRecord {
        round: RoundId::from_str(ROUND).expect("a round ID"),
        task: TaskId::from_str(task).expect("a task ID"),
        number,
        at: at(1_700_000_000 + i64::from(number), 0),
        commit: commit('c'),
        head: commit('d'),
        bytes: 5 * 1024 * 1024,
    }))
}

/// The snapshot `record` is, when it is one.
fn as_snapshot(record: &Record) -> Option<&SnapshotRecord> {
    match record {
        Record::Snapshot(snapshot) => Some(snapshot.as_ref()),
        _ => None,
    }
}

#[test]
fn snapshots_are_read_back_by_round_in_the_order_of_their_tasks_and_numbers() {
    let (_dir, path, mut db) = database();
    let written = [
        snapshot(SECOND_TASK, 1),
        snapshot(FIRST_TASK, 2),
        snapshot(FIRST_TASK, 1),
    ];
    db.write(&written).expect("the snapshots are written");
    drop(db);

    let round = RoundId::from_str(ROUND).expect("a round ID");
    let read = Reader::open(&path)
        .expect("the history opens")
        .snapshots_of_round(&round)
        .expect("the snapshots are read");

    let expected: Vec<SnapshotRecord> = [
        snapshot(FIRST_TASK, 1),
        snapshot(FIRST_TASK, 2),
        snapshot(SECOND_TASK, 1),
    ]
    .iter()
    .filter_map(|record| as_snapshot(record).cloned())
    .collect();
    assert_eq!(read, expected);
}

#[test]
fn a_snapshot_handed_in_twice_is_one_row() {
    let (_dir, path, mut db) = database();
    db.write(&[snapshot(FIRST_TASK, 1)]).expect("the first write");
    db.write(&[snapshot(FIRST_TASK, 1)]).expect("the second write");
    drop(db);

    let round = RoundId::from_str(ROUND).expect("a round ID");
    let read = Reader::open(&path)
        .expect("the history opens")
        .snapshots_of_round(&round)
        .expect("the snapshots are read");

    assert_eq!(read.len(), 1, "{read:?}");
}

#[test]
fn a_history_of_version_11_is_migrated_and_keeps_its_records_and_takes_snapshots() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("history.db");
    rusqlite::Connection::open(&path)
        .expect("the database")
        .execute_batch(include_str!("fixtures/history-before-snapshots.sql"))
        .expect("the version 11 schema");
    let world = complete(17);
    let mut db = HistoryDb::open(&path, roomy()).expect("the old history opens and migrates");
    db.write(&world.records()).expect("the records are written");
    drop(db);
    let round = world.rounds[0].round.clone();
    let before = Reader::open(&path)
        .expect("the history opens")
        .round_records(&round)
        .expect("the records are read");

    let mut db = HistoryDb::open(&path, roomy()).expect("the history opens");
    db.write(&[snapshot(FIRST_TASK, 1)]).expect("the snapshot is written");
    drop(db);

    let raw = rusqlite::Connection::open(&path).expect("the database");
    let version: u32 = raw
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .expect("the version");
    assert_eq!(version, 13);
    let reader = Reader::open(&path).expect("the history opens");
    assert_eq!(reader.round_records(&round).expect("the records are read"), before);
    let snapshot_round = RoundId::from_str(ROUND).expect("a round ID");
    assert_eq!(
        reader
            .snapshots_of_round(&snapshot_round)
            .expect("the snapshots are read")
            .len(),
        1
    );
}

#[test]
fn snapshot_warnings_are_kept_in_the_daemon_log() {
    let (_dir, path, mut db) = database();
    let warning = |event: LogEvent, message: &str| LogRecord {
        at: at(5, 0),
        module: "agent::daemon::owner".to_owned(),
        event,
        team: None,
        task: None,
        message: message.to_owned(),
        fields: BTreeMap::new(),
        cut: Cut::default(),
    };
    db.write_logs(&[
        warning(LogEvent::SnapshotLarge, "a big snapshot"),
        warning(LogEvent::SnapshotFailed, "a snapshot that failed"),
    ])
    .expect("the warnings are written");
    drop(db);

    let logs = Reader::open(&path)
        .expect("the history opens")
        .daemon_logs()
        .expect("the logs are read");

    let kinds: Vec<_> = logs.iter().map(|entry| (entry.event(), entry.level())).collect();
    assert_eq!(
        kinds,
        vec![
            (LogEvent::SnapshotLarge, LogLevel::Warn),
            (LogEvent::SnapshotFailed, LogLevel::Warn),
        ]
    );
}

#[test]
fn delivery_close_warning_migrates_old_history_without_losing_logs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute_batch(include_str!("fixtures/history-before-delivery-close.sql"))
        .unwrap();
    let warning = |event| LogRecord {
        at: at(5, 0),
        module: "agent::daemon::owner".to_owned(),
        event,
        team: None,
        task: None,
        message: "delivery lifecycle".to_owned(),
        fields: BTreeMap::new(),
        cut: Cut::default(),
    };
    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    db.write_logs(&[warning(LogEvent::SnapshotLarge), warning(LogEvent::DeliveryClosed)])
        .expect("both warning events are stored");
    drop(db);
    let logs = Reader::open(&path).unwrap().daemon_logs().unwrap();
    assert_eq!(
        logs.iter()
            .map(|entry| (entry.event(), entry.level()))
            .collect::<Vec<_>>(),
        vec![
            (LogEvent::SnapshotLarge, LogLevel::Warn),
            (LogEvent::DeliveryClosed, LogLevel::Warn)
        ]
    );
}
