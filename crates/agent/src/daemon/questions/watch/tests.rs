use std::time::Duration;

use super::*;
use crate::fixtures::task;

/// The longest a test waits for an event that has to come.
const PATIENCE: Duration = Duration::from_secs(30);

/// A watch on the directory of task 1 in a temporary home.
fn watching() -> (tempfile::TempDir, PathBuf, Watcher, mpsc::Receiver<Saved>) {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(task(1).as_str());
    std::fs::create_dir_all(&dir).unwrap();
    let (saved, events) = mpsc::channel(16);
    let mut watcher = Watcher::new(saved).unwrap();
    watcher.watch(&dir).unwrap();
    (home, dir, watcher, events)
}

async fn next(events: &mut mpsc::Receiver<Saved>) -> Saved {
    tokio::time::timeout(PATIENCE, events.recv())
        .await
        .expect("no questions file was reported")
        .expect("the watch ended")
}

fn saved(n: u32, round: u32) -> Saved {
    Saved {
        task: task(n),
        round: NonZeroU32::new(round).unwrap(),
    }
}

#[tokio::test]
async fn a_questions_file_written_in_place_is_reported_with_its_task_and_round() {
    let (_home, dir, _watcher, mut events) = watching();

    std::fs::write(dir.join("questions-2.md"), "text").unwrap();

    assert_eq!(next(&mut events).await, saved(1, 2));
}

#[tokio::test]
async fn a_questions_file_renamed_over_the_old_one_is_reported() {
    let (_home, dir, _watcher, mut events) = watching();
    std::fs::write(dir.join("questions-1.md"), "old").unwrap();
    assert_eq!(next(&mut events).await, saved(1, 1));
    std::fs::write(dir.join(".questions-1.md.swp"), "new").unwrap();

    std::fs::rename(dir.join(".questions-1.md.swp"), dir.join("questions-1.md")).unwrap();

    assert_eq!(next(&mut events).await, saved(1, 1));
}

#[tokio::test]
async fn other_files_of_the_directory_are_not_reported() {
    let (_home, dir, _watcher, mut events) = watching();

    std::fs::write(dir.join("activity.jsonl"), "{}").unwrap();
    std::fs::write(dir.join("questions-x.md"), "no round").unwrap();
    std::fs::write(dir.join("questions-0.md"), "round 0").unwrap();
    std::fs::write(dir.join("questions-3.md"), "text").unwrap();

    // Events come in the order the files were written: the first one reported is the only file that is a questions file.
    assert_eq!(next(&mut events).await, saved(1, 3));
}

#[tokio::test]
async fn a_directory_that_is_let_go_reports_no_more() {
    let (_home, dir, mut watcher, mut events) = watching();
    watcher.unwatch(&dir).unwrap();

    std::fs::write(dir.join("questions-1.md"), "text").unwrap();
    // Another directory, still watched, shows when the system has told everything it was going to tell.
    let other = dir.with_file_name(task(2).as_str());
    std::fs::create_dir_all(&other).unwrap();
    watcher.watch(&other).unwrap();
    std::fs::write(other.join("questions-4.md"), "text").unwrap();

    assert_eq!(next(&mut events).await, saved(2, 4));
}

#[tokio::test]
async fn a_directory_that_is_not_there_cannot_be_watched() {
    let (home, _dir, mut watcher, _events) = watching();
    let missing = home.path().join("nothing");

    let error = watcher.watch(&missing).unwrap_err();

    assert!(
        matches!(&error, WatchError::Watch { path, .. } if *path == missing),
        "{error:?}"
    );
}

#[tokio::test]
async fn letting_go_a_directory_that_was_never_watched_is_an_error_that_names_it() {
    let (home, _dir, mut watcher, _events) = watching();
    let other = home.path().join("other");
    std::fs::create_dir_all(&other).unwrap();

    let error = watcher.unwatch(&other).unwrap_err();

    assert!(
        matches!(&error, WatchError::Unwatch { path, .. } if *path == other),
        "{error:?}"
    );
}

/// Everything `events` reports until it has been quiet for a moment.
async fn quiet(events: &mut mpsc::Receiver<Saved>) -> Vec<Saved> {
    let mut all = Vec::new();
    while let Ok(Some(saved)) = tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
        all.push(saved);
    }
    all
}

#[tokio::test]
async fn the_save_of_another_task_is_reported_though_the_queue_is_full_of_saves_of_one() {
    let home = tempfile::tempdir().unwrap();
    let make = |n: u32| {
        let dir = home.path().join(task(n).as_str());
        std::fs::create_dir_all(&dir).unwrap();
        dir
    };
    let (first, second) = (make(1), make(2));
    let (queue, mut events) = mpsc::channel(4);
    let mut watcher = Watcher::new(queue).unwrap();
    watcher.watch(&first).unwrap();
    watcher.watch(&second).unwrap();

    for n in 0..100 {
        std::fs::write(first.join("questions-1.md"), format!("save {n}")).unwrap();
    }
    std::fs::write(second.join("questions-1.md"), "the only save").unwrap();

    let reported = quiet(&mut events).await;
    assert!(reported.contains(&saved(2, 1)), "{reported:?}");
}
