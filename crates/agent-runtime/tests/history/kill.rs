//! A writer killed in the middle of a batch (a child process, `SIGKILL`) leaves a database that opens, passes
//! `PRAGMA integrity_check` and holds no half record: each record is in whole or not at all.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use agent_runtime::history::{HistoryDb, Reader, Store};
use rusqlite::Connection;

use super::support::{roomy, world};

/// The variable that tells the test of the child that it is the child, and where its database is.
const CHILD_DIR: &str = "HISTORY_KILL_TEST_DIR";

/// How many worlds the child writes in one transaction.
const BATCH: u64 = 60;

/// How many batches the parent waits for before it kills the child.
const BATCHES_BEFORE_KILL: usize = 3;

/// What the child runs: with the variable set, a writer that never stops; without it, one batch written and read back,
/// so that the test is also a check on its own.
#[test]
fn the_writer_the_kill_test_kills() {
    let Ok(dir) = std::env::var(CHILD_DIR) else {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.db");
        let mut db = HistoryDb::open(&path, roomy()).unwrap();
        let records: Vec<_> = (1..=5).flat_map(|n| world(n).records()).collect();

        db.write(&records).unwrap();

        let reader = Reader::open(&path).unwrap();
        for n in 1..=5 {
            assert_eq!(reader.team(&world(n).team.team).unwrap().unwrap().value, world(n).team);
        }
        return;
    };
    let mut db = HistoryDb::open(&std::path::Path::new(&dir).join("history.db"), roomy()).unwrap();
    let mut first = 1_u64;
    loop {
        let records: Vec<_> = (first..first + BATCH).flat_map(|n| world(n).records()).collect();
        db.write(&records).unwrap();
        println!("committed {} {}", first, first + BATCH - 1);
        first += BATCH;
    }
}

#[test]
fn a_writer_killed_in_the_middle_of_a_batch_leaves_a_database_that_opens_and_holds_no_half_record() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["kill::the_writer_the_kill_test_kills", "--exact", "--nocapture"])
        .env(CHILD_DIR, dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut last_committed = 0_u64;
    let mut committed = 0_usize;
    let lines = BufReader::new(child.stdout.take().unwrap()).lines();
    for line in lines {
        let line = line.unwrap();
        if let Some(numbers) = line.strip_prefix("committed ") {
            last_committed = numbers.split(' ').nth(1).unwrap().parse().unwrap();
            committed += 1;
            if committed >= BATCHES_BEFORE_KILL {
                break;
            }
        }
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(
        committed >= BATCHES_BEFORE_KILL,
        "the child ended before it wrote {BATCHES_BEFORE_KILL} batches"
    );

    let path = dir.path().join("history.db");
    let raw = Connection::open(&path).unwrap();
    let integrity: String = raw
        .pragma_query_value(None, "integrity_check", |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
    drop(raw);
    let reader = Reader::open(&path).unwrap();
    let mut present = 0_u64;
    for n in 1..last_committed + 3 * BATCH {
        let expected = world(n);
        match reader.team(&expected.team.team).unwrap() {
            Some(team) => {
                present += 1;
                assert_eq!(team.value, expected.team, "world {n}: a half team");
                let runs = reader.runs_of_team(&expected.team.team).unwrap();
                assert_eq!(
                    runs.len(),
                    expected.runs.len(),
                    "world {n}: runs missing from a team that is there"
                );
                for (row, expected_run) in runs.iter().zip(&expected.runs) {
                    assert_eq!(row.start.value, expected_run.start, "world {n}");
                    let steps = reader.steps(&expected_run.start.run).unwrap();
                    assert_eq!(
                        steps.len(),
                        expected_run.steps.len(),
                        "world {n}: steps missing from a run that is there"
                    );
                    let changes = reader
                        .changes(&expected_run.start.run)
                        .unwrap()
                        .map(|stored| stored.value);
                    assert_eq!(changes, expected_run.changes, "world {n}: half changes");
                }
            }
            None => assert!(n > last_committed, "world {n} was committed and is gone"),
        }
    }
    assert!(
        present >= last_committed,
        "{present} worlds present, {last_committed} were committed"
    );
    // The database opens for writing again, and a batch is whole after the crash.
    let mut db = HistoryDb::open(&path, roomy()).unwrap();
    db.write(&world(1_000_000).records()).unwrap();
}
