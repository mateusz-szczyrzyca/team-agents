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
    let base_time = SystemTime::now() - Duration::from_secs(3600);

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
}

#[tokio::test]
async fn empty_untracked_files_returns_empty_collection() {
    let dir = tempfile::tempdir().expect("tempdir");
    let result = collect(dir.path(), &[]).await;
    assert!(result.summary_facts.is_empty());
}
