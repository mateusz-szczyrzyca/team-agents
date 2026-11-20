//! Schema 13 remains readable without inventing delivery claims or rewriting history.

use super::*;

#[test]
fn schema_thirteen_migrates_without_delivery_and_writes_fourteen() {
    let old = include_str!("../../tests/fixtures/state-before-delivery.json");
    let state = State::from_json(old).unwrap();
    assert!(
        state
            .teams()
            .iter()
            .flat_map(Team::rounds)
            .all(|round| round.delivered_by().is_none())
    );
    let mut written: serde_json::Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();
    assert_eq!(written["schema_version"], 14);
    written["schema_version"] = serde_json::json!(13);
    assert_eq!(written, serde_json::from_str::<serde_json::Value>(old).unwrap());
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}
