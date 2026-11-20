//! Old clients deny unpublished bases; explicit permission crosses the wire.
use agent_core::protocol::RunRequest;

#[test]
fn run_permission_is_optional_and_explicit_permission_roundtrips() {
    let old = serde_json::json!({"repo":"example", "checkout":"/tmp/example", "agents":["codex-1"],
        "task_list":"Fix the typo.", "merge":"autopilot"});
    let request: RunRequest = serde_json::from_value(old.clone()).unwrap();
    assert!(!request.allow_unpushed_base);
    assert_eq!(serde_json::to_value(&request).unwrap(), old);
    let mut allowed = old;
    allowed["allow_unpushed_base"] = serde_json::json!(true);
    let request: RunRequest = serde_json::from_value(allowed.clone()).unwrap();
    assert!(request.allow_unpushed_base);
    assert_eq!(serde_json::to_value(request).unwrap(), allowed);
}
