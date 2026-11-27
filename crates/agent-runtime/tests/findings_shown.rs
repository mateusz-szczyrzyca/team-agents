//! Delivery records remain compatible and reject corrupt or oversized data.
use agent_runtime::findings::{MAX_SHOWN_BYTES, ShownFindings, ShownFindingsError};

#[tokio::test]
async fn shown_findings_missing_is_empty_and_saved_ids_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("findings-shown.json");
    let mut shown = ShownFindings::read(&path).await.unwrap();
    assert_eq!(shown.ids().len(), 0);
    shown
        .save(&path, ["F-8".parse().unwrap(), "F-3".parse().unwrap()])
        .await
        .unwrap();
    shown.save(&path, ["F-8".parse().unwrap()]).await.unwrap();
    assert!(path.is_file(), "delivery record must exist before launch");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).unwrap(),
        serde_json::json!({"shown":["F-3","F-8"]})
    );
    assert_eq!(ShownFindings::read(&path).await.unwrap(), shown);
}

#[tokio::test]
async fn shown_findings_rejects_corrupt_invalid_and_oversized_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("findings-shown.json");
    for text in ["broken", "{\"shown\":[\"F-0\"]}"] {
        std::fs::write(&path, text).unwrap();
        assert!(
            matches!(ShownFindings::read(&path).await, Err(ShownFindingsError::Json {path: failed,..}) if failed == path)
        );
    }
    std::fs::write(&path, vec![b' '; MAX_SHOWN_BYTES + 1]).unwrap();
    assert!(
        matches!(ShownFindings::read(&path).await, Err(ShownFindingsError::Read {path: failed,..}) if failed == path)
    );
}

#[tokio::test]
async fn shown_findings_reports_read_and_write_failures_with_the_record_path() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        matches!(ShownFindings::read(dir.path()).await, Err(ShownFindingsError::Read {path,..}) if path == dir.path())
    );
    let path = dir.path().join("missing/findings-shown.json");
    let mut shown = ShownFindings::default();
    assert!(
        matches!(shown.save(&path, ["F-1".parse().unwrap()]).await, Err(ShownFindingsError::Write {path: failed,..}) if failed == path)
    );
}
