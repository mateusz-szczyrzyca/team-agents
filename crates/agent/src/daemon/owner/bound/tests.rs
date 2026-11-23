//! The size of a run's context, read from its outcome file first and from the history only when the file is missing.

use std::path::Path;

use agent_core::ids::RunId;

use super::{SizeRead, size_of_run};

/// The directory of a run in `root`, made if it is not there yet.
fn run_dir(root: &Path) -> std::path::PathBuf {
    let dir = root.join("run");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A run ID of the tests.
fn run() -> RunId {
    "run-20261123-200000-tst1".parse().unwrap()
}

#[tokio::test]
async fn an_outcome_file_tells_the_size_even_when_there_is_no_history() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = run_dir(dir.path());
    std::fs::write(
        run_dir.join("outcome.json"),
        r#"{"last_context":200000,"final_message":"done"}"#,
    )
    .unwrap();
    let missing = dir.path().join("no-history.db");

    let read = size_of_run(&run_dir, &missing, &run()).await;

    assert!(matches!(read, SizeRead::Told(200_000)), "{read:?}");
}

#[tokio::test]
async fn an_outcome_file_whose_size_is_null_tells_no_size() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = run_dir(dir.path());
    std::fs::write(
        run_dir.join("outcome.json"),
        r#"{"last_context":null,"final_message":null}"#,
    )
    .unwrap();

    let read = size_of_run(&run_dir, &dir.path().join("no-history.db"), &run()).await;

    assert!(matches!(read, SizeRead::NotTold), "{read:?}");
}

#[tokio::test]
async fn a_corrupt_outcome_file_is_unreadable_and_names_its_cause() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = run_dir(dir.path());
    std::fs::write(run_dir.join("outcome.json"), "not an outcome").unwrap();

    let read = size_of_run(&run_dir, &dir.path().join("no-history.db"), &run()).await;

    assert!(matches!(read, SizeRead::Unreadable(_)), "{read:?}");
}

#[tokio::test]
async fn a_run_with_no_outcome_file_and_no_history_tells_no_size() {
    let dir = tempfile::tempdir().unwrap();
    let run_dir = run_dir(dir.path());

    let read = size_of_run(&run_dir, &dir.path().join("no-history.db"), &run()).await;

    assert!(matches!(read, SizeRead::NotTold), "{read:?}");
}
