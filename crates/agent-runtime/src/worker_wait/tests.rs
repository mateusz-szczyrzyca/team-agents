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
