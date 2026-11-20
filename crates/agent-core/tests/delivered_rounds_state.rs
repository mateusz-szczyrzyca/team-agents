//! Schema 12 loads and migrates to 13; delivered rounds and reopen persist durably.

use std::num::NonZeroU32;

use agent_core::state::{PrNumber, RoundLife, State};

#[test]
fn schema_twelve_migrates_to_thirteen_and_delivered_rounds_persist() {
    let old = include_str!("fixtures/state-before-delivered-rounds.json");
    let mut state = State::from_json(old).unwrap();

    let mut written: serde_json::Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();
    assert_eq!(written["schema_version"], 13);
    written["schema_version"] = serde_json::json!(12);
    assert_eq!(written, serde_json::from_str::<serde_json::Value>(old).unwrap());

    // Round delivery roundtrips in state
    let round_id = state.teams()[0]
        .rounds()
        .iter()
        .find(|r| matches!(r.life(), RoundLife::Open))
        .unwrap()
        .id()
        .clone();
    let pr = PrNumber::new(NonZeroU32::new(99).unwrap());
    state.deliver_round(&round_id, pr).unwrap();

    assert_eq!(
        state.teams()[0].round(&round_id).unwrap().life(),
        &RoundLife::Delivered { pr }
    );

    let json = state.to_json().unwrap();
    let loaded = State::from_json(&json).unwrap();
    assert_eq!(
        loaded.teams()[0].round(&round_id).unwrap().life(),
        &RoundLife::Delivered { pr }
    );

    // Reopening round roundtrips in state
    let mut state_reopened = loaded;
    state_reopened.reopen_round(&round_id).unwrap();
    assert_eq!(
        state_reopened.teams()[0].round(&round_id).unwrap().life(),
        &RoundLife::Open
    );

    let json_reopened = state_reopened.to_json().unwrap();
    let loaded_reopened = State::from_json(&json_reopened).unwrap();
    assert_eq!(
        loaded_reopened.teams()[0].round(&round_id).unwrap().life(),
        &RoundLife::Open
    );
}
