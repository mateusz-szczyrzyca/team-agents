//! The newest runs of a task: the last ones, oldest first, with a flag for the older ones left out.

use std::num::NonZeroUsize;

use agent_runtime::history::{Reader, Store};

use super::support::{database, world};

/// A world whose runs are all of one task, and the task.
fn runs_of_one_task() -> (super::support::World, agent_core::ids::TaskId) {
    let mut written = (1..200)
        .map(world)
        .find(|world| world.runs.len() >= 3)
        .expect("a seed with three runs");
    let task = written.runs[0].start.task.clone();
    for run in &mut written.runs {
        run.start.task = task.clone();
    }
    (written, task)
}

#[test]
fn the_newest_runs_of_a_task_are_the_last_ones_and_a_flag_says_older_ones_are_left_out() {
    let (_dir, path, mut db) = database();
    let (written, task) = runs_of_one_task();
    db.write(&written.records()).unwrap();
    let reader = Reader::open(&path).unwrap();
    let all = reader.runs_of_task(&task).unwrap();

    let newest = reader
        .newest_runs_of_task(&task, NonZeroUsize::new(2).unwrap())
        .unwrap();

    assert_eq!(newest.runs, all[all.len() - 2..]);
    assert!(newest.left_out);
}

#[test]
fn a_limit_that_covers_every_run_leaves_none_out() {
    let (_dir, path, mut db) = database();
    let (written, task) = runs_of_one_task();
    db.write(&written.records()).unwrap();
    let reader = Reader::open(&path).unwrap();
    let all = reader.runs_of_task(&task).unwrap();

    for limit in [all.len(), all.len() + 5] {
        let newest = reader
            .newest_runs_of_task(&task, NonZeroUsize::new(limit).unwrap())
            .unwrap();

        assert_eq!(newest.runs, all);
        assert!(!newest.left_out);
    }
}

#[test]
fn a_task_with_no_run_has_no_newest_runs() {
    let (_dir, path, mut db) = database();
    let (written, _) = runs_of_one_task();
    db.write(&written.records()).unwrap();
    let other = world(9000).runs[0].start.task.clone();
    let reader = Reader::open(&path).unwrap();

    let newest = reader
        .newest_runs_of_task(&other, NonZeroUsize::new(3).unwrap())
        .unwrap();

    assert_eq!(newest.runs, []);
    assert!(!newest.left_out);
}
