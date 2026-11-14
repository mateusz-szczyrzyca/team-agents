//! A file that something other than the writer changed: each row that breaks a rule of its record is refused when it
//! is read, with the table and the column named. The rules the database checks itself are switched off for the
//! tampering, to prove the reader does not trust them alone.

use agent_core::history::Record;
use agent_runtime::history::{HistoryDb, ReadError, Reader, Store, WriteKind};
use rusqlite::Connection;

use super::support::{at, complete, database};

/// A database with the complete world of seed 1 in it, and the run's ID.
fn written() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    agent_core::ids::RunId,
    agent_core::ids::TeamId,
) {
    let (dir, path, mut db) = database();
    let world = complete(1);
    db.write(&world.records()).unwrap();
    (dir, path, world.runs[0].start.run.clone(), world.team.team.clone())
}

/// Runs `sql` on the database with its own checks off.
fn tamper(path: &std::path::Path, sql: &str) {
    let raw = Connection::open(path).unwrap();
    raw.execute_batch("PRAGMA ignore_check_constraints = ON; PRAGMA foreign_keys = OFF;")
        .unwrap();
    raw.execute_batch(sql).unwrap();
}

/// Asserts that `error` is a refused value in `column` of `table`.
fn assert_value(error: &ReadError, table: &str, column: &str, case: &str) {
    assert!(
        matches!(error, ReadError::Value { table: t, column: c, .. } if *t == table && *c == column),
        "{case}: {error:?}"
    );
}

/// What a case reads after the damage.
#[derive(Debug, Clone, Copy)]
enum Read {
    /// The steps of the run.
    Steps,
    /// The run.
    Run,
    /// The team.
    Team,
    /// What the run changed.
    Changes,
}

#[test]
fn a_row_with_a_value_its_column_cannot_hold_is_refused_when_read_with_the_table_and_column() {
    // (the case, the damage, what is read, the table of the damaged row, its column)
    let cases = [
        (
            "an unknown kind of step",
            "UPDATE steps SET kind = 'bogus' WHERE n = 1",
            Read::Steps,
            "steps",
            "kind",
        ),
        (
            "a message with no text",
            "UPDATE steps SET text = NULL WHERE n = 1",
            Read::Steps,
            "steps",
            "text",
        ),
        (
            "a reason that needs an item and has none",
            "UPDATE runs SET why = 'item', why_item = NULL",
            Read::Run,
            "runs",
            "why_item",
        ),
        (
            "an unknown merge mode",
            "UPDATE teams SET merge_mode = 'merge'",
            Read::Team,
            "teams",
            "merge_mode",
        ),
        (
            "a draw whose candidates are not a list",
            "UPDATE plan SET how = 'drawn', drawn_among = '\"x\"'",
            Read::Team,
            "plan",
            "drawn_among",
        ),
        (
            "a time that is not a time",
            "UPDATE runs SET at = 'yesterday'",
            Read::Run,
            "runs",
            "at",
        ),
        (
            "a report without its outcome",
            "UPDATE run_ends SET how = 'reported', outcome = NULL",
            Read::Run,
            "run_ends",
            "outcome",
        ),
        (
            "a rename without its source",
            "UPDATE changed_files SET renamed_from = NULL",
            Read::Changes,
            "changed_files",
            "renamed_from",
        ),
        (
            "arguments that are not texts",
            "UPDATE launches SET args = '[1, 2]'",
            Read::Run,
            "launches",
            "args",
        ),
        (
            "a round that is not an ID",
            "UPDATE run_rounds SET round = 'round-x'",
            Read::Run,
            "run_rounds",
            "round",
        ),
        (
            "a negative count of lines",
            "UPDATE changed_files SET lines_added = -1",
            Read::Changes,
            "changed_files",
            "lines_added",
        ),
        (
            "a cut field that no record has",
            "UPDATE runs SET cut = 'nonsense'",
            Read::Run,
            "runs",
            "cut",
        ),
    ];
    for (case, damage, read, table, column) in cases {
        let (_dir, path, run, team) = written();
        tamper(&path, damage);
        let reader = Reader::open(&path).unwrap();

        let error = match read {
            Read::Steps => reader.steps(&run).unwrap_err(),
            Read::Run => reader.run(&run).unwrap_err(),
            Read::Team => reader.team(&team).unwrap_err(),
            Read::Changes => reader.changes(&run).unwrap_err(),
        };

        assert_value(&error, table, column, case);
    }
}

#[test]
fn a_run_that_ended_before_it_started_is_refused_when_read_and_when_written() {
    let (_dir, path, run, _team) = written();
    tamper(&path, "UPDATE run_ends SET at = '2000-01-01T00:00:00.000000000Z'");

    let error = Reader::open(&path).unwrap().run(&run).unwrap_err();

    assert_value(&error, "run_ends", "at", "ended before it started");
}

#[test]
fn a_run_that_ends_before_it_started_is_refused_by_the_writer_and_one_that_ends_when_it_started_is_not() {
    let (_dir, path, mut db) = database();
    let world = complete(2);
    let run = &world.runs[0];
    db.write(&[Record::Run(Box::new(run.start.clone()))]).unwrap();
    let mut early = run.end.clone().unwrap();
    early.at = at(-1, 0);
    let mut early_by_a_second = run.end.clone().unwrap();
    early_by_a_second.at =
        agent_core::state::Timestamp::from_utc(run.start.at.as_utc() - time::Duration::seconds(1)).unwrap();
    let mut on_time = run.end.clone().unwrap();
    on_time.at = run.start.at;

    let refused = db.write(&[Record::RunEnd(Box::new(early_by_a_second))]).unwrap_err();
    let refused_far = db.write(&[Record::RunEnd(Box::new(early))]).unwrap_err();
    db.write(&[Record::RunEnd(Box::new(on_time.clone()))]).unwrap();

    assert_eq!(refused.kind(), WriteKind::Refused);
    assert_eq!(refused_far.kind(), WriteKind::Refused);
    let row = Reader::open(&path).unwrap().run(&run.start.run).unwrap().unwrap();
    assert_eq!(row.end.unwrap().value, on_time);
}

#[test]
fn a_database_whose_rows_are_damaged_still_opens_for_writing_and_reads_the_rows_that_are_not() {
    let (_dir, path, run, team) = written();
    tamper(&path, "UPDATE steps SET kind = 'bogus' WHERE n = 1");

    let reader = Reader::open(&path).unwrap();

    assert!(reader.team(&team).unwrap().is_some());
    assert!(reader.run(&run).unwrap().is_some());
    drop(HistoryDb::open(&path, super::support::roomy()).unwrap());
}
