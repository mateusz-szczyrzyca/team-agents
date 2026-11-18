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
    assert!(result.unreadable.is_empty());
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
