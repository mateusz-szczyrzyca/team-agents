//! The removals that have not ended, per task: a task is held back while one of its removals runs.
use tokio::task::JoinSet;

use super::*;

fn task(number: u8) -> TaskId {
    format!("task-20261119-13142{number}-ab{number}c").parse().unwrap()
}

#[tokio::test]
async fn a_task_is_pending_from_its_removal_until_the_removal_ends() {
    let mut jobs: JoinSet<()> = JoinSet::new();
    let job = jobs.spawn_blocking(|| ()).id();
    let mut pending = PendingRemovals::default();

    pending.start(job, &task(1));
    assert!(pending.contains(&task(1)));

    pending.finish(job);
    assert!(!pending.contains(&task(1)));
}

#[tokio::test]
async fn a_task_stays_pending_until_all_its_removals_end() {
    let mut jobs: JoinSet<()> = JoinSet::new();
    let first = jobs.spawn_blocking(|| ()).id();
    let second = jobs.spawn_blocking(|| ()).id();
    let mut pending = PendingRemovals::default();
    pending.start(first, &task(1));
    pending.start(second, &task(1));

    pending.finish(first);
    assert!(pending.contains(&task(1)));

    pending.finish(second);
    assert!(!pending.contains(&task(1)));
}

#[tokio::test]
async fn finishing_an_unknown_job_changes_nothing() {
    let mut jobs: JoinSet<()> = JoinSet::new();
    let known = jobs.spawn_blocking(|| ()).id();
    let unknown = jobs.spawn_blocking(|| ()).id();
    let mut pending = PendingRemovals::default();
    pending.start(known, &task(2));

    pending.finish(unknown);

    assert!(pending.contains(&task(2)));
}
