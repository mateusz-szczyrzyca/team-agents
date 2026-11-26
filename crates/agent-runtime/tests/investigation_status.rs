//! Bus lifecycle statuses remain typed and forward compatible.

use agent_runtime::agentbus::InvestigationStatus;

#[test]
fn bus_statuses_decode_without_losing_future_values() {
    for (raw, expected) in [
        ("queued", InvestigationStatus::Queued),
        ("waitroom", InvestigationStatus::Waitroom),
        ("starting", InvestigationStatus::Starting),
        ("running", InvestigationStatus::Running),
        ("waiting_human", InvestigationStatus::WaitingHuman),
        ("done", InvestigationStatus::Done),
        ("failed", InvestigationStatus::Failed),
        ("rejected", InvestigationStatus::Rejected),
        ("future", InvestigationStatus::Unknown("future".to_owned())),
    ] {
        let status: InvestigationStatus = serde_json::from_value(serde_json::json!(raw)).unwrap();
        assert_eq!(status, expected);
    }
}
