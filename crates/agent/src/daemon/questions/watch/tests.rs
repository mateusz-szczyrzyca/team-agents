use std::time::Duration;

use super::*;
use crate::fixtures::task;

/// The longest a test waits for an event that has to come.
const PATIENCE: Duration = Duration::from_secs(30);

/// A watch on the directory of task 1 in a temporary home.
fn watching() -> (tempfile::TempDir, PathBuf, Watcher, Saves) {
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(task(1).as_str());
    std::fs::create_dir_all(&dir).unwrap();
    let (mut watcher, events) = Watcher::new().unwrap();
    watcher.watch(&dir).unwrap();
    (home, dir, watcher, events)
}

async fn next(events: &mut Saves) -> Saved {
    let report = tokio::time::timeout(PATIENCE, events.recv())
        .await
        .expect("no questions file was reported")
        .expect("the watch ended");
    match report {
        Report::Saved(saved) => saved,
        Report::Rescan => panic!("a save was expected, not a rescan"),
    }
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
async fn quiet(events: &mut Saves) -> Vec<Saved> {
    let mut all = Vec::new();
    while let Ok(Some(Report::Saved(saved))) = tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
        all.push(saved);
    }
    all
}

#[tokio::test]
async fn the_save_of_another_task_is_reported_though_the_owner_is_busy_with_saves_of_one() {
    let home = tempfile::tempdir().unwrap();
    let make = |n: u32| {
        let dir = home.path().join(task(n).as_str());
        std::fs::create_dir_all(&dir).unwrap();
        dir
    };
    let (first, second) = (make(1), make(2));
    let (mut watcher, mut events) = Watcher::new().unwrap();
    watcher.watch(&first).unwrap();
    watcher.watch(&second).unwrap();

    for n in 0..100 {
        std::fs::write(first.join("questions-1.md"), format!("save {n}")).unwrap();
    }
    std::fs::write(second.join("questions-1.md"), "the only save").unwrap();

    let reported = quiet(&mut events).await;
    assert!(reported.contains(&saved(2, 1)), "{reported:?}");
}

#[tokio::test]
async fn many_saves_of_one_file_wait_as_one_until_the_owner_takes_it() {
    let (home, first, mut watcher, events) = watching();
    let second = home.path().join(task(2).as_str());
    std::fs::create_dir_all(&second).unwrap();
    watcher.watch(&second).unwrap();

    for n in 0..50 {
        std::fs::write(first.join("questions-1.md"), format!("save {n}")).unwrap();
    }
    std::fs::write(second.join("questions-1.md"), "text").unwrap();
    // The events come in the order of the writes: when the last file is queued, every repeat before it was seen.
    let deadline = std::time::Instant::now() + PATIENCE;
    while !events.queue.lock().unwrap().saves.contains(&saved(2, 1)) {
        assert!(std::time::Instant::now() < deadline, "the last save was never queued");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let queued: Vec<Saved> = events.queue.lock().unwrap().saves.iter().cloned().collect();
    assert_eq!(queued, [saved(1, 1), saved(2, 1)]);
}

#[tokio::test]
async fn a_file_saved_again_after_the_owner_took_it_is_queued_again() {
    let (_home, dir, _watcher, mut events) = watching();
    std::fs::write(dir.join("questions-1.md"), "one").unwrap();
    assert_eq!(next(&mut events).await, saved(1, 1));

    std::fs::write(dir.join("questions-1.md"), "two").unwrap();

    assert_eq!(next(&mut events).await, saved(1, 1));
}

// ---- the queue is bounded, and what it cannot hold is read again -------------------------------------------------

/// What the system says when `round` of the questions file of task `n` was written.
fn written(n: u32, round: u32) -> notify::Result<notify::Event> {
    let path = PathBuf::from("/home")
        .join(task(n).as_str())
        .join(format!("questions-{round}.md"));
    Ok(notify::Event::new(notify::EventKind::Create(notify::event::CreateKind::File)).add_path(path))
}

/// The next report of `saves`, which has to be there: a report that never comes is a failure, not a hang.
async fn report(saves: &mut Saves) -> Option<Report> {
    tokio::time::timeout(Duration::from_secs(3600), saves.recv())
        .await
        .expect("no report came")
}

/// Nothing is reported within a long wait; the clock of the test is paused, so the wait costs nothing.
async fn nothing_more(saves: &mut Saves) {
    let report = tokio::time::timeout(Duration::from_secs(3600), saves.recv()).await;
    assert!(report.is_err(), "{report:?}");
}

#[tokio::test(start_paused = true)]
async fn a_flood_of_saves_of_different_files_does_not_fill_the_queue_without_end() {
    let (reporter, mut saves) = channel();
    let flood = 1_000;
    for round in 1..=flood {
        reporter.event(written(1, round));
    }
    drop(reporter);

    let mut kept = Vec::new();
    while let Some(Report::Saved(saved)) = saves.recv().await {
        kept.push(saved);
    }

    assert!(
        kept.len() < usize::try_from(flood).unwrap(),
        "{} saves were kept",
        kept.len()
    );
    assert_eq!(
        kept.first(),
        Some(&saved(1, 1)),
        "the saves kept are the first ones, in order"
    );
}

#[tokio::test(start_paused = true)]
async fn what_the_queue_could_not_hold_is_told_as_a_rescan_after_the_saves() {
    let (reporter, mut saves) = channel();
    for round in 1..=1_000 {
        reporter.event(written(1, round));
    }

    let mut last = None;
    while let Ok(Some(report)) = tokio::time::timeout(Duration::from_secs(1), saves.recv()).await {
        last = Some(report);
    }

    assert_eq!(last, Some(Report::Rescan));
}

#[tokio::test(start_paused = true)]
async fn an_error_of_the_watch_asks_for_a_rescan_once_however_often_it_comes() {
    let (reporter, mut saves) = channel();
    reporter.event(written(1, 1));
    reporter.event(Err(notify::Error::generic("the system lost track")));
    reporter.event(Err(notify::Error::generic("again")));

    assert_eq!(report(&mut saves).await, Some(Report::Saved(saved(1, 1))));
    assert_eq!(report(&mut saves).await, Some(Report::Rescan));
    nothing_more(&mut saves).await;
}

#[tokio::test(start_paused = true)]
async fn a_report_that_waits_when_the_watch_ends_is_still_given() {
    let (reporter, mut saves) = channel();
    reporter.event(written(1, 1));
    drop(reporter);

    assert_eq!(report(&mut saves).await, Some(Report::Saved(saved(1, 1))));
    assert_eq!(report(&mut saves).await, None);
}
