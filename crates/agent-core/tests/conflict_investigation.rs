//! Automatic conflict investigation configuration and additive persisted item state.

use agent_core::config::Config;
use agent_core::state::State;

#[test]
fn a_state_without_conflict_investigation_loads() {
    let state = State::from_json(include_str!("fixtures/state-before-conflict-investigation.json")).unwrap();
    let stored: serde_json::Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();
    assert_eq!(stored["teams"][0]["items"][0]["merge_passes"], 2);
    assert_eq!(stored["teams"][0]["items"][0].get("conflict_investigation"), None);
}

#[test]
fn a_conflict_investigation_survives_a_state_round_trip() {
    let mut stored: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/state-before-conflict-investigation.json")).unwrap();
    let investigation =
        serde_json::json!({"head": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "id": 7, "answered": true});
    stored["teams"][0]["items"][0]["conflict_investigation"] = investigation.clone();
    let state = State::from_json(&stored.to_string()).unwrap();
    let restored: serde_json::Value = serde_json::from_str(&state.to_json().unwrap()).unwrap();
    assert_eq!(
        restored["teams"][0]["items"][0]["conflict_investigation"],
        investigation
    );
}

#[test]
fn automatic_conflict_investigation_defaults_to_enabled_and_can_be_disabled() {
    let base = "[agents.claude-1]\ncli = \"claude\"\n[tiers]\nnormal = [\"claude-1\"]\n";
    assert!(Config::from_toml_str(base).unwrap().merge().investigator_pass);
    let disabled = Config::from_toml_str(&format!("{base}\n[merge]\ninvestigator_pass = false\n")).unwrap();
    assert!(!disabled.merge().investigator_pass);
}
