//! Conflict request decoding at the bus boundary.

use agent_runtime::agentbus::{Investigation, InvestigationKind};

#[test]
fn a_conflict_request_carries_its_validated_item() {
    let request: Investigation = serde_json::from_value(serde_json::json!({
        "id": 1, "from_agent": "lead", "from_task": "task-20261127-120000-abcd",
        "branch": "agents/lead", "question": "Resolve this", "kind": "conflict",
        "item": "5.1", "file": "/tmp/inv.md"
    }))
    .unwrap();
    assert_eq!(
        request.kind,
        InvestigationKind::Conflict {
            item: "5.1".parse().unwrap()
        }
    );
}

#[test]
fn conflict_requests_refuse_missing_and_invalid_items() {
    for item in [None, Some(""), Some("opaque méta"), Some("../5.1")] {
        let mut raw = serde_json::json!({
            "id": 1, "from_agent": "lead", "from_task": "task-20261127-120000-abcd",
            "branch": "agents/lead", "question": "Resolve this", "kind": "conflict", "file": "/tmp/inv.md"
        });
        if let Some(item) = item {
            raw["item"] = serde_json::json!(item);
        }
        assert!(serde_json::from_value::<Investigation>(raw).unwrap_err().is_data());
    }
}
