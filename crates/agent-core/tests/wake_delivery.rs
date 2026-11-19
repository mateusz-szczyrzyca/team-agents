//! Wake-up delivery metadata survives state round trips without requiring it in old state.

use agent_core::state::State;
use serde_json::json;

#[test]
fn wake_delivery_counts_and_delivered_markers_survive_state_round_trips() {
    for status in ["pending", "delivered"] {
        let value = json!({
            "schema_version": 9,
            "teams": [],
            "wake_deliveries": {
                "task-20261111-153001-k3x9": {"state": status, "message": 42, "count": 2}
            }
        });
        let parsed = State::from_json(&value.to_string());
        assert!(
            matches!(&parsed, Ok(state) if state.teams().is_empty()),
            "delivery metadata must load: {parsed:?}"
        );
        let stored: serde_json::Value = serde_json::from_str(&parsed.unwrap().to_json().unwrap()).unwrap();
        assert_eq!(stored, value);
    }
}

#[test]
fn state_written_before_wake_delivery_loads_without_inventing_a_delivery() {
    let old = include_str!("fixtures/state-before-wake-delivery.json");
    let state = State::from_json(old).unwrap();
    let stored: serde_json::Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();
    assert_eq!(stored, serde_json::from_str::<serde_json::Value>(old).unwrap());
}
