use std::str::FromStr;

use super::*;

/// A record of the tests.
fn record(session: Option<&str>, context: u64) -> ContextRecord {
    ContextRecord {
        run: RunId::from_str("run-20261125-100000-a1b2").expect("a run ID of the tests"),
        session: session.map(|id| SessionId::from_str(id).expect("a session ID of the tests")),
        context,
    }
}

#[tokio::test]
async fn a_record_written_is_the_record_read_and_leaves_no_temporary_file() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let written = record(Some("0c4e9a52-7a11-4d0b-8f3e-5b2c6d1e9a07"), 130_000);

    write_context(dir.path(), &written)
        .await
        .expect("the record is written");

    assert_eq!(
        read_context(dir.path()).await.expect("the record is read"),
        Some(written)
    );
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .expect("the directory")
        .map(|entry| entry.expect("an entry").file_name())
        .collect();
    assert_eq!(names, [std::ffi::OsString::from(CONTEXT_FILE)]);
}

#[tokio::test]
async fn a_session_that_is_not_named_is_written_as_null() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    write_context(dir.path(), &record(None, 7))
        .await
        .expect("the record is written");

    let text = std::fs::read_to_string(dir.path().join(CONTEXT_FILE)).expect("the file");
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(
        value,
        serde_json::json!({"run": "run-20261125-100000-a1b2", "session": null, "context": 7})
    );
}

#[tokio::test]
async fn a_directory_without_the_file_has_no_record() {
    let dir = tempfile::tempdir().expect("a temporary directory");

    assert_eq!(
        read_context(dir.path()).await.expect("a missing file is no error"),
        None
    );
}

#[tokio::test]
async fn a_file_that_is_not_a_record_is_a_parse_error_with_its_path() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    std::fs::write(
        dir.path().join(CONTEXT_FILE),
        r#"{"run":"run-20261125-100000-a1b2","context":"many"}"#,
    )
    .expect("the file");

    let error = read_context(dir.path()).await.expect_err("the context is no number");

    assert!(
        matches!(&error, ContextError::Parse { path, .. } if *path == dir.path().join(CONTEXT_FILE)),
        "{error:?}"
    );
}

#[tokio::test]
async fn a_task_directory_that_does_not_exist_cannot_be_written_to() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let missing = dir.path().join("no-such-task");

    let error = write_context(&missing, &record(None, 1))
        .await
        .expect_err("there is no directory");

    assert!(matches!(&error, ContextError::Write { .. }), "{error:?}");
}
