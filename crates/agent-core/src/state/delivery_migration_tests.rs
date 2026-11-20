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

#[test]
fn schema_fourteen_from_0_34_reads_an_open_delivery_claim_unchanged() {
    let old = include_str!("../../tests/fixtures/state-with-open-delivery-0.34.0.json");
    let state = State::from_json(old).unwrap();
    let waiting: Vec<_> = state.teams()[0].delivery_waits().collect();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].delivered_by().unwrap().number.get().get(), 82);
    assert_eq!(waiting[0].delivery_closed_at(), None);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&state.to_json().unwrap()).unwrap(),
        serde_json::from_str::<serde_json::Value>(old).unwrap()
    );
}
