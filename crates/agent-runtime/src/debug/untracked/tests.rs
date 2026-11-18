//! Tests for untracked file metadata collection.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use super::*;

#[tokio::test]
async fn collects_fifty_newest_untracked_files_sorted_by_mtime() {
    let dir = tempfile::tempdir().expect("tempdir");
    let worktree = dir.path();

    let mut paths = Vec::new();
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);

    // Create 60 untracked files with increasing modification times
    for i in 0..60 {
        let name = format!("file_{i:02}.txt");
        let path = worktree.join(&name);
        fs::write(&path, format!("content {i}")).expect("write file");
        let file_time = base_time + Duration::from_secs(i * 10);
        let file = fs::OpenOptions::new().write(true).open(&path).expect("open file");
        let times = fs::FileTimes::new().set_modified(file_time);
        file.set_times(times).expect("set times");
        paths.push(PathBuf::from(name));
    }

    let result = collect(worktree, &paths).await;

    assert_eq!(result.summary_facts.len(), 50, "must collect exactly 50 newest files");

    // The newest should be file_59.txt down to file_10.txt
    assert_eq!(result.summary_facts[0].path, PathBuf::from("file_59.txt"));
    assert_eq!(result.summary_facts[49].path, PathBuf::from("file_10.txt"));
    assert!(result.summary_facts[0].size_bytes > 0);
    assert!(result.summary_facts[0].modified.is_some());
    assert!(
        result.artifact_text.contains("file_00.txt"),
        "artifact text must contain all untracked files"
    );
    assert!(result.artifact_text.contains("file_59.txt"));
}

#[tokio::test]
async fn empty_untracked_files_returns_empty_collection() {
    let dir = tempfile::tempdir().expect("tempdir");
    let result = collect(dir.path(), &[]).await;
    assert_eq!(result.summary_facts, Vec::<UntrackedFileSummaryFacts>::new());
    assert_eq!(
        result.unreadable,
        Vec::<agent_core::diagnostic_assessment::ReadFailure>::new()
    );
}

#[tokio::test]
async fn unreadable_untracked_path_records_failure_reason() {
    let dir = tempfile::tempdir().expect("tempdir");
    let worktree = dir.path();
    let valid_name = "present.txt";
    fs::write(worktree.join(valid_name), b"content").expect("write");
    let missing_name = "missing.txt";

    let result = collect(worktree, &[PathBuf::from(valid_name), PathBuf::from(missing_name)]).await;
    assert_eq!(result.summary_facts.len(), 1);
    assert_eq!(result.summary_facts[0].path, PathBuf::from(valid_name));
    assert_eq!(result.unreadable.len(), 1);
    assert_eq!(
        result.unreadable[0].section,
        agent_core::debug_summary::SummarySection::GitWorktrees
    );
    assert!(
        result.unreadable[0].part.contains(missing_name),
        "{}",
        result.unreadable[0].part
    );
}

#[tokio::test]
async fn distinct_non_utf8_names_stay_distinct_in_artifact_and_failures() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let worktree = dir.path();
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let first = OsStr::from_bytes(b"f\xFE");
    let second = OsStr::from_bytes(b"f\xFF");
    for name in [first, second] {
        let path = worktree.join(name);
        fs::write(&path, b"same").expect("write");
        let file = fs::OpenOptions::new().write(true).open(&path).expect("open file");
        file.set_times(fs::FileTimes::new().set_modified(base_time))
            .expect("set times");
    }
    let missing_first = OsStr::from_bytes(b"gone\xFE");
    let missing_second = OsStr::from_bytes(b"gone\xFF");
    let paths = [
        PathBuf::from(first),
        PathBuf::from(second),
        PathBuf::from(missing_first),
        PathBuf::from(missing_second),
    ];

    let result = collect(worktree, &paths).await;

    assert!(result.artifact_text.contains(r#""f\xFE""#), "{}", result.artifact_text);
    assert!(result.artifact_text.contains(r#""f\xFF""#), "{}", result.artifact_text);
    assert_eq!(result.unreadable.len(), 2, "{:?}", result.unreadable);
    assert_ne!(result.unreadable[0].part, result.unreadable[1].part);
    assert!(
        result.unreadable[0].part.contains(r#""gone\xFE""#),
        "{}",
        result.unreadable[0].part
    );
    assert!(
        result.unreadable[1].part.contains(r#""gone\xFF""#),
        "{}",
        result.unreadable[1].part
    );
}

/// Seconds in one day, to count whole days from the epoch.
const SECONDS_PER_DAY: u64 = 86_400;

#[tokio::test]
async fn modification_time_beyond_year_9999_is_unreadable_and_sorts_last() {
    let dir = tempfile::tempdir().expect("tempdir");
    let worktree = dir.path();
    // 10000-01-02T00:00:00Z: the second day past the last instant the state timestamps can hold.
    let beyond = SystemTime::UNIX_EPOCH + Duration::from_secs(SECONDS_PER_DAY * 2_932_898);
    let known = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    for (name, time) in [("future.txt", beyond), ("known.txt", known)] {
        let path = worktree.join(name);
        fs::write(&path, b"x").expect("write");
        let file = fs::OpenOptions::new().write(true).open(&path).expect("open file");
        file.set_times(fs::FileTimes::new().set_modified(time))
            .expect("set times");
    }

    let result = collect(worktree, &[PathBuf::from("future.txt"), PathBuf::from("known.txt")]).await;

    assert_eq!(result.summary_facts.len(), 2);
    assert_eq!(result.summary_facts[0].path, PathBuf::from("known.txt"));
    assert!(result.summary_facts[0].modified.is_some());
    assert_eq!(result.summary_facts[1].path, PathBuf::from("future.txt"));
    assert_eq!(result.summary_facts[1].modified, None);
    assert_eq!(result.unreadable.len(), 1, "{:?}", result.unreadable);
    assert!(
        result.unreadable[0].part.contains("future.txt"),
        "{}",
        result.unreadable[0].part
    );
    assert!(
        result
            .artifact_text
            .contains("- \"future.txt\" (1 bytes, modified unknown)"),
        "{}",
        result.artifact_text
    );
}

#[tokio::test]
async fn directory_entries_are_not_listed_as_untracked_files() {
    let dir = tempfile::tempdir().expect("tempdir");
    let worktree = dir.path();
    fs::create_dir(worktree.join("sub")).expect("mkdir");
    fs::write(worktree.join("file.txt"), b"x").expect("write");

    let result = collect(worktree, &[PathBuf::from("sub"), PathBuf::from("file.txt")]).await;

    assert_eq!(result.summary_facts.len(), 1);
    assert_eq!(result.summary_facts[0].path, PathBuf::from("file.txt"));
    assert!(result.unreadable.is_empty(), "{:?}", result.unreadable);
    assert!(
        result.artifact_text.starts_with("### Untracked Files (1)\n"),
        "{}",
        result.artifact_text
    );
}

#[tokio::test]
async fn modification_time_before_year_0000_is_unreadable_and_unknown() {
    let dir = tempfile::tempdir().expect("tempdir");
    let worktree = dir.path();
    // 0000-01-01T00:00:00Z is 62_167_219_200 seconds before the epoch; one day before it is year -1.
    let before = SystemTime::UNIX_EPOCH - Duration::from_secs(62_167_219_200 + SECONDS_PER_DAY);
    fs::write(worktree.join("ancient.txt"), b"x").expect("write");
    let file = fs::OpenOptions::new()
        .write(true)
        .open(worktree.join("ancient.txt"))
        .expect("open file");
    file.set_times(fs::FileTimes::new().set_modified(before))
        .expect("set times");

    let result = collect(worktree, &[PathBuf::from("ancient.txt")]).await;

    assert_eq!(result.summary_facts.len(), 1);
    assert_eq!(result.summary_facts[0].modified, None);
    assert_eq!(result.unreadable.len(), 1, "{:?}", result.unreadable);
    assert!(
        result.unreadable[0].part.contains("ancient.txt"),
        "{}",
        result.unreadable[0].part
    );
    assert!(
        result
            .artifact_text
            .contains("- \"ancient.txt\" (1 bytes, modified unknown)"),
        "{}",
        result.artifact_text
    );
}
