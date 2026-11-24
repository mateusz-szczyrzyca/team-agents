//! The mark of a reminder the bus took: written once, read back, and refused where its directory is gone.

use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::ids::{RunId, TaskId};
use agent_runtime::store::Home;

use super::{Claim, MarkError, claim, is_told, mark_told, release};

/// The task of the fixture.
fn task() -> TaskId {
    TaskId::from_str("task-20261124-100000-t1t1").expect("the task")
}

/// The run of the fixture.
fn run() -> RunId {
    RunId::from_str("run-20261124-100000-r1r1").expect("the run")
}

/// A home in a temporary directory.
fn home(dir: &tempfile::TempDir) -> Home {
    Home::new(&AbsolutePath::from_str(dir.path().to_str().expect("a UTF-8 path")).expect("an absolute path"))
}

#[tokio::test]
async fn a_run_is_not_told_until_the_mark_is_written() {
    let dir = tempfile::tempdir().expect("a directory");
    let home = home(&dir);
    std::fs::create_dir_all(home.run_dir(&task(), &run())).expect("the run's directory");

    let before = is_told(&home, &task(), &run()).await.expect("the look");
    mark_told(&home, &task(), &run()).await.expect("the mark");
    let after = is_told(&home, &task(), &run()).await.expect("the look");

    assert_eq!((before, after), (false, true));
}

#[tokio::test]
async fn the_mark_of_one_run_does_not_tell_another_run() {
    let dir = tempfile::tempdir().expect("a directory");
    let home = home(&dir);
    let other = RunId::from_str("run-20261124-100100-r2r2").expect("the other run");
    std::fs::create_dir_all(home.run_dir(&task(), &run())).expect("the run's directory");

    mark_told(&home, &task(), &run()).await.expect("the mark");

    assert!(!is_told(&home, &task(), &other).await.expect("the look"));
}

#[tokio::test]
async fn a_mark_is_refused_when_the_directory_of_the_run_is_gone() {
    let dir = tempfile::tempdir().expect("a directory");
    let home = home(&dir);

    let refused = mark_told(&home, &task(), &run()).await;

    assert!(matches!(refused, Err(MarkError::Write { .. })), "{refused:?}");
}

#[tokio::test]
async fn a_run_is_claimed_once_and_its_mark_tells_it() {
    let dir = tempfile::tempdir().expect("a directory");
    let home = home(&dir);
    std::fs::create_dir_all(home.run_dir(&task(), &run())).expect("the run's directory");

    let first = claim(&home, &task(), &run()).await.expect("the claim");
    let told = is_told(&home, &task(), &run()).await.expect("the look");
    let second = claim(&home, &task(), &run()).await.expect("the claim");

    assert_eq!((first, told, second), (Claim::Taken, true, Claim::AlreadyTold));
}

#[tokio::test]
async fn a_claim_that_was_released_can_be_taken_again() {
    let dir = tempfile::tempdir().expect("a directory");
    let home = home(&dir);
    std::fs::create_dir_all(home.run_dir(&task(), &run())).expect("the run's directory");
    claim(&home, &task(), &run()).await.expect("the claim");

    release(&home, &task(), &run()).await.expect("the release");

    assert!(!is_told(&home, &task(), &run()).await.expect("the look"));
    assert_eq!(claim(&home, &task(), &run()).await.expect("the claim"), Claim::Taken);
}

#[tokio::test]
async fn a_claim_is_refused_when_the_directory_of_the_run_is_gone() {
    let dir = tempfile::tempdir().expect("a directory");
    let home = home(&dir);

    let refused = claim(&home, &task(), &run()).await;

    assert!(matches!(refused, Err(MarkError::Write { .. })), "{refused:?}");
}

#[tokio::test]
async fn a_release_of_a_claim_that_was_never_made_is_refused() {
    let dir = tempfile::tempdir().expect("a directory");
    let home = home(&dir);
    std::fs::create_dir_all(home.run_dir(&task(), &run())).expect("the run's directory");

    let refused = release(&home, &task(), &run()).await;

    assert!(matches!(refused, Err(MarkError::Release { .. })), "{refused:?}");
}
