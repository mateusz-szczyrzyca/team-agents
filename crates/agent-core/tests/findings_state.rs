//! Schema migration keeps old rounds on their original protocol; new ledgers survive a restart.
use agent_core::state::{SCHEMA_VERSION, State};

/// A state written by the schema immediately before findings were introduced.
const OLD: &str = include_str!("fixtures/state-before-findings.json");
#[test]
fn reads_the_previous_schema_without_enabling_new_rules_for_open_rounds() {
    let state = State::from_json(OLD).unwrap();
    assert_eq!(SCHEMA_VERSION, 3);
    assert!(
        state
            .teams()
            .iter()
            .flat_map(agent_core::state::Team::items)
            .all(|item| item.findings().is_none())
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&state.to_json().unwrap()).unwrap()["schema_version"],
        3
    );
}
#[test]
fn a_new_findings_ledger_roundtrips_with_its_item() {
    let mut state = State::from_json(OLD).unwrap();
    let team = state.teams()[0].id().clone();
    let id = state.teams()[0].items()[0].id();
    state.team_mut(&team).unwrap().item_mut(id).unwrap().start_findings();
    assert!(state.team(&team).unwrap().item(id).unwrap().findings().is_some());
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}
