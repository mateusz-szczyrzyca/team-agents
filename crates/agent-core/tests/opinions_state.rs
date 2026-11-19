//! The immediately previous daemon's state remains readable without inventing opinions.
use agent_core::state::{SCHEMA_VERSION, State};

#[test]
fn schema_six_findings_migrate_without_consuming_a_second_opinion() {
    let old = include_str!("fixtures/state-before-second-opinions.json");
    let state = State::from_json(old).unwrap();
    assert_eq!(SCHEMA_VERSION, 8);
    let ledger = state.teams()[0].items()[1].findings().unwrap();
    assert_eq!(ledger.opinions().len(), 0);
    assert_eq!(ledger.open().len(), 1);
    assert!(ledger.disputes().is_some());
    assert_eq!(
        state.to_json().unwrap(),
        old.replacen("\"schema_version\": 6", "\"schema_version\": 8", 1)
    );
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}
