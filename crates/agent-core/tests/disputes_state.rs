//! The previous daemon's active reviews retain their original protocol.
use agent_core::state::{SCHEMA_VERSION, State};

#[test]
fn schema_five_active_review_retains_reviewers_and_does_not_require_lead_answers() {
    let old = include_str!("fixtures/state-before-disputes.json");
    let state = State::from_json(old).unwrap();
    assert_eq!(SCHEMA_VERSION, 8);
    let item = state.teams()[0].item("1.2".parse().unwrap()).unwrap();
    assert_eq!(item.findings().unwrap().disputes(), None);
    assert!(item.findings().unwrap().verification().is_some());
    assert_eq!(
        state.to_json().unwrap(),
        old.replacen("\"schema_version\": 5", "\"schema_version\": 8", 1)
    );
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}
