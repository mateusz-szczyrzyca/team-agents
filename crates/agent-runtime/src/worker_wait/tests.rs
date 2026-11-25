//! The note of since when a request waits.
use std::num::NonZeroU64;

use super::*;

fn request(number: u64) -> RequestId {
    RequestId::new(NonZeroU64::new(number).unwrap())
}

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

#[tokio::test]
async fn a_noted_wait_is_read_back() {
    let dir = tempfile::tempdir().unwrap();

    write_since(dir.path(), request(3), at("2026-11-25T10:00:00Z"))
        .await
        .unwrap();

    assert_eq!(
        read_since(dir.path(), request(3)).await.unwrap(),
        Some(at("2026-11-25T10:00:00Z"))
    );
    assert_eq!(read_since(dir.path(), request(4)).await.unwrap(), None);
}

#[tokio::test]
async fn the_note_is_a_file_with_the_request_and_the_time() {
    let dir = tempfile::tempdir().unwrap();

    write_since(dir.path(), request(12), at("2026-11-25T10:00:00Z"))
        .await
        .unwrap();

    let text = std::fs::read_to_string(dir.path().join("worker-wait-12.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"request": "H12", "since": "2026-11-25T10:00:00Z"})
    );
}

#[tokio::test]
async fn a_second_note_keeps_the_first_time() {
    let dir = tempfile::tempdir().unwrap();

    write_since(dir.path(), request(3), at("2026-11-25T10:00:00Z"))
        .await
        .unwrap();
    write_since(dir.path(), request(3), at("2026-11-25T10:05:00Z"))
        .await
        .unwrap();

    assert_eq!(
        read_since(dir.path(), request(3)).await.unwrap(),
        Some(at("2026-11-25T10:00:00Z"))
    );
}

#[tokio::test]
async fn clearing_removes_the_note_and_a_missing_note_is_no_failure() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        wait_path(dir.path(), request(3)),
        r#"{"request":"H3","since":"2026-11-25T10:00:00Z"}"#,
    )
    .unwrap();

    clear(dir.path(), request(3)).await.unwrap();
    clear(dir.path(), request(3)).await.unwrap();

    assert!(!wait_path(dir.path(), request(3)).exists());
}

#[tokio::test]
async fn a_file_that_is_no_note_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = wait_path(dir.path(), request(3));
    std::fs::write(&path, "not json").unwrap();

    let error = read_since(dir.path(), request(3)).await.unwrap_err();

    assert!(
        matches!(&error, WaitFileError::Parse { path: found, .. } if *found == path),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_note_in_a_directory_that_is_gone_cannot_be_written() {
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("gone");

    let error = write_since(&gone, request(3), at("2026-11-25T10:00:00Z"))
        .await
        .unwrap_err();

    assert!(matches!(error, WaitFileError::Write { .. }), "{error:?}");
}

#[tokio::test]
async fn a_note_longer_than_the_limit_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = wait_path(dir.path(), request(3));
    std::fs::write(&path, "x".repeat(NOTE_LIMIT + 1)).unwrap();

    let error = read_since(dir.path(), request(3)).await.unwrap_err();

    assert!(
        matches!(&error, WaitFileError::Read { path: found, source: TextFileError::TooLarge { .. } } if *found == path),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_note_that_cannot_be_removed_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = wait_path(dir.path(), request(3));
    std::fs::create_dir(&path).unwrap();

    let error = clear(dir.path(), request(3)).await.unwrap_err();

    assert!(
        matches!(&error, WaitFileError::Remove { path: found, .. } if *found == path),
        "{error:?}"
    );
}

#[tokio::test]
async fn the_notes_of_every_task_directory_are_found() {
    let dir = tempfile::tempdir().unwrap();
    for (task, number) in [("task-a", 3), ("task-a", 4), ("task-b", 12)] {
        let task_dir = dir.path().join(task);
        std::fs::create_dir_all(&task_dir).unwrap();
        write_since(&task_dir, request(number), at("2026-11-25T10:00:00Z"))
            .await
            .unwrap();
    }
    let other = dir.path().join("task-c");
    std::fs::create_dir_all(&other).unwrap();
    for name in [
        "worker-wait-0.json",
        "worker-wait-x.json",
        "worker-wait-5.txt",
        "notes.json",
    ] {
        std::fs::write(other.join(name), "{}").unwrap();
    }
    std::fs::write(dir.path().join("worker-wait-9.json"), "{}").unwrap();

    let mut found = find_notes(dir.path()).await.unwrap();
    found.sort_by_key(|note| note.request);

    assert_eq!(
        found,
        [
            FoundNote {
                task_dir: dir.path().join("task-a"),
                request: request(3)
            },
            FoundNote {
                task_dir: dir.path().join("task-a"),
                request: request(4)
            },
            FoundNote {
                task_dir: dir.path().join("task-b"),
                request: request(12)
            },
        ]
    );
}

#[tokio::test]
async fn no_tasks_directory_holds_no_notes() {
    let dir = tempfile::tempdir().unwrap();

    assert_eq!(find_notes(&dir.path().join("tasks")).await.unwrap(), []);
}

#[tokio::test]
async fn a_tasks_path_that_is_a_file_cannot_be_looked_through() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tasks");
    std::fs::write(&path, "").unwrap();

    let error = find_notes(&path).await.unwrap_err();

    assert!(
        matches!(&error, WaitFileError::List { path: found, .. } if *found == path),
        "{error:?}"
    );
}
