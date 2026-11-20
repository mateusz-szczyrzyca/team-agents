//! Wake-up delivery metadata survives state round trips without requiring it in old state.

use agent_core::state::State;
use agent_core::state::WakeDelivery;
use serde_json::json;

#[test]
fn wake_delivery_counts_and_delivered_markers_survive_state_round_trips() {
    for status in ["pending", "delivered"] {
        let value = json!({
            "schema_version": agent_core::state::SCHEMA_VERSION,
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
    assert_eq!(
        stored,
        serde_json::from_str::<serde_json::Value>(
            &include_str!("fixtures/state-before-wake-delivery-activities.json")
                .replace("\"schema_version\": 11", "\"schema_version\": 12")
        )
        .unwrap()
    );
}

#[test]
fn recording_and_clearing_delivery_is_scoped_to_its_task() {
    let first: agent_core::ids::TaskId = "task-20261111-153001-k3x9".parse().unwrap();
    let second: agent_core::ids::TaskId = "task-20261111-153002-k3x9".parse().unwrap();
    let delivery = WakeDelivery::Pending {
        message: std::num::NonZeroU64::new(42),
        count: 1,
    };
    let delivered = WakeDelivery::Delivered {
        message: std::num::NonZeroU64::new(42),
        count: 2,
    };
    let mut state = State::new();
    state.record_wake_delivery(first.clone(), Some(delivery));
    state.record_wake_delivery(second.clone(), Some(delivered));
    assert_eq!(state.wake_delivery(&first), Some(delivery));
    assert_eq!(state.wake_delivery(&second), Some(delivered));
    state.record_wake_delivery(first.clone(), None);
    assert_eq!(state.wake_delivery(&first), None);
    assert_eq!(state.wake_delivery(&second), Some(delivered));
    state.record_wake_delivery(first.clone(), None);
    assert_eq!(state.wake_delivery(&first), None);
}
