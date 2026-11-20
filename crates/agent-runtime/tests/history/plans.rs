//! The plan versions a task keeps: what is written when a plan differs from the last one read, what is not written
//! again, and what the reader gives back.

use std::num::NonZeroU32;

use agent_core::ids::{RunId, TaskId, TeamId};
use agent_core::state::Timestamp;
use agent_runtime::history::{HistoryDb, PlanRead, PlanRecord, Reader, Store};

use super::support::{at, database, world};

/// Task 2 of the sample world: the member's task.
fn task() -> TaskId {
    world(1).runs[1].start.task.clone()
}

/// The team of task 2.
fn team() -> TeamId {
    world(1).runs[1].start.team.clone()
}

/// Run 2 of the sample world: the member's run.
fn run() -> RunId {
    world(1).runs[1].start.run.clone()
}

/// A record of a plan `text` read at `seconds` at the end of `run` of `task` in `team`.
fn version(task: &TaskId, team: &TeamId, run: &RunId, seconds: i64, text: &str) -> PlanRecord {
    PlanRecord {
        task: task.clone(),
        team: team.clone(),
        run: Some(run.clone()),
        at: at(seconds, 0),
        read: PlanRead::Version { text: text.to_owned() },
    }
}

/// A database with the sample world of seed 1 in it: a team, its rounds and its runs.
fn with_world() -> (tempfile::TempDir, std::path::PathBuf, HistoryDb) {
    let (dir, path, mut db) = database();
    let world = world(1);
    db.write(&world.records()).unwrap();
    (dir, path, db)
}

#[test]
fn a_plan_version_is_written_at_the_end_of_a_run_and_read_back() {
    let (_dir, path, mut db) = with_world();
    let record = version(&task(), &team(), &run(), 40, "1. Work\n[ ] 1.1 Write it\n");

    db.write_plans(std::slice::from_ref(&record)).unwrap();
    drop(db);

    let read = Reader::open(&path).unwrap().plan_versions(&task()).unwrap();

    assert_eq!(read, vec![record]);
}

#[test]
fn an_unchanged_plan_makes_no_second_version() {
    let (_dir, path, mut db) = with_world();
    let task = task();
    let first = version(&task, &team(), &run(), 40, "1. Work\n[ ] 1.1 Write it\n");
    db.write_plans(std::slice::from_ref(&first)).unwrap();
    db.write_plans(std::slice::from_ref(&first)).unwrap();
    drop(db);

    let read = Reader::open(&path).unwrap().plan_versions(&task).unwrap();

    assert_eq!(read, vec![first]);
}

#[test]
fn a_missing_and_an_unreadable_plan_are_recorded_as_such() {
    let (_dir, path, mut db) = with_world();
    let task = task();
    let team = team();
    let missing = PlanRecord {
        task: task.clone(),
        team: team.clone(),
        run: None,
        at: at(50, 0),
        read: PlanRead::Missing,
    };
    let unreadable = PlanRecord {
        task: task.clone(),
        team,
        run: Some(run()),
        at: at(60, 0),
        read: PlanRead::Unreadable {
            line: Some(NonZeroU32::new(2).expect("a line")),
            detail: "line 2: malformed step line".to_owned(),
        },
    };
    db.write_plans(&[missing.clone(), unreadable.clone()]).unwrap();
    drop(db);

    let read = Reader::open(&path).unwrap().plan_versions(&task).unwrap();

    assert_eq!(read, vec![missing, unreadable]);
}

#[test]
fn the_last_version_of_another_run_is_the_one_to_compare_with() {
    let (_dir, path, mut db) = with_world();
    let task = task();
    let team = team();
    let first = version(&task, &team, &run(), 40, "1. Work\n[ ] 1.1 Write it\n");
    let second = version(&task, &team, &run(), 45, "1. Work\n[x] 1.1 Write it\n");
    db.write_plans(&[first.clone(), second.clone()]).unwrap();
    drop(db);

    let reader = Reader::open(&path).unwrap();
    let of_the_run = reader.last_plan_version_before(&task, Some(&run())).unwrap();
    let elsewhere = reader
        .last_plan_version_before(&task, Some(&"run-20261111-193009-aaaa".parse().expect("a run")))
        .unwrap();

    assert_eq!(of_the_run, None);
    assert_eq!(elsewhere, Some(second));
}

#[test]
fn plans_of_a_task_that_is_not_there_are_refused() {
    let (_dir, _path, mut db) = with_world();
    let absent: TaskId = "task-20261111-193009-zzzz".parse().unwrap();
    let record = version(&absent, &team(), &run(), 40, "1. Work\n");

    let refused = db.write_plans(&[record]);

    assert!(matches!(refused, Err(agent_runtime::history::WriteError::Rule { .. })));
}

#[test]
fn a_plan_version_keeps_the_time_it_is_read_at() {
    let (_dir, path, mut db) = with_world();
    let record = version(&task(), &team(), &run(), 41, "1. Work\n[x] 1.1 Write it\n");
    let when: Timestamp = at(41, 0);
    db.write_plans(std::slice::from_ref(&record)).unwrap();
    drop(db);

    let read = Reader::open(&path).unwrap().plan_versions(&task()).unwrap();

    assert_eq!(read.first().map(|record| record.at), Some(when));
}
