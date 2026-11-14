//! Deterministic checks of the fake CLI counter's publication boundary.

use std::panic::{AssertUnwindSafe, catch_unwind};

use super::Rig;

#[tokio::test]
async fn an_empty_run_counter_waits_for_publication_and_then_reports_the_count() {
    let rig = Rig::new(&[], 5, false).await;
    let task = "task-20261114-120000-abcd".parse().unwrap();
    let worktree = rig.worktree(&task);
    std::fs::create_dir_all(&worktree).unwrap();
    let counter = worktree.join(".fake-runs");
    assert_eq!(rig.runs(&task), 0);

    // Reproduce the shell redirection's empty-file window, without racing a process or waiting on a clock.
    for empty in ["", " \n"] {
        std::fs::write(&counter, empty).unwrap();
        let observed = catch_unwind(AssertUnwindSafe(|| rig.runs(&task)));
        assert!(matches!(observed, Ok(0)), "unpublished counter {empty:?}: {observed:?}");
    }
    for count in [0, 1, 2, u32::MAX] {
        std::fs::write(&counter, format!("{count}\n")).unwrap();
        assert_eq!(rig.runs(&task), count);
    }
}

#[tokio::test]
async fn a_malformed_nonempty_run_counter_still_panics() {
    let rig = Rig::new(&[], 5, false).await;
    let task = "task-20261114-120000-abcd".parse().unwrap();
    let worktree = rig.worktree(&task);
    std::fs::create_dir_all(&worktree).unwrap();
    for malformed in ["not-a-number", "-1", "4294967296"] {
        std::fs::write(worktree.join(".fake-runs"), malformed).unwrap();
        let observed = catch_unwind(AssertUnwindSafe(|| rig.runs(&task)));
        drop(observed.expect_err("malformed nonempty counters must not silently become zero"));
    }
}
