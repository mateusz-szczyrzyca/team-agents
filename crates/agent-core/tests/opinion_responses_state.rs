//! Compatibility with the daemon before durable opinion responses.
use agent_core::state::{SCHEMA_VERSION, State};
use std::num::NonZeroU32;
#[test]
fn schema_seven_active_fix_pass_migrates_without_inventing_answers() {
    let old = include_str!("fixtures/state-before-opinion-responses.json");
    let state = State::from_json(old).unwrap();
    let ledger = state.teams()[0].items()[1].findings().unwrap();
    assert_eq!(ledger.fixed_answers(NonZeroU32::MIN), Default::default());
    assert_eq!(ledger.opinions().len(), 0);
    assert_eq!(SCHEMA_VERSION, 8);
    assert_eq!(
        state.to_json().unwrap(),
        old.replacen("\"schema_version\": 7", "\"schema_version\": 8", 1)
    );
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}
