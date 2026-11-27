//! Split request decoding at the bus boundary.

use agent_runtime::agentbus::{Investigation, InvestigationKind};

#[test]
fn a_split_request_carries_its_validated_item() {
    let request: Investigation = serde_json::from_value(serde_json::json!({
        "id": 1, "from_agent": "lead", "from_task": "task-20261127-120000-abcd", "branch": "agents/lead",
        "question": "Split this", "kind": "split", "item": "1.1", "file": "/tmp/inv.md"
    }))
    .unwrap();
    assert_eq!(
        request.kind,
        InvestigationKind::Split {
            item: "1.1".parse().unwrap()
        }
    );
}

#[test]
fn a_split_request_without_an_item_is_refused() {
    let result = serde_json::from_value::<Investigation>(serde_json::json!({
        "id": 1, "from_agent": "lead", "from_task": "task-20261127-120000-abcd", "branch": "agents/lead",
        "question": "Split this", "kind": "split", "file": "/tmp/inv.md"
    }));
    assert!(result.unwrap_err().is_data());
}

#[test]
fn non_split_requests_accept_opaque_item_metadata() {
    for (kind, expected) in [
        ("", InvestigationKind::Question),
        ("question", InvestigationKind::Question),
        ("review", InvestigationKind::Review),
        ("future-kind", InvestigationKind::Unknown("future-kind".to_owned())),
    ] {
        let request: Investigation = serde_json::from_value(serde_json::json!({
            "id": 1, "from_agent": "lead", "from_task": "task-20261127-120000-abcd",
            "branch": "agents/lead", "question": "Investigate", "kind": kind,
            "item": "opaque méta", "file": "/tmp/inv.md"
        }))
        .unwrap();
        assert_eq!(request.kind, expected);
    }
}
