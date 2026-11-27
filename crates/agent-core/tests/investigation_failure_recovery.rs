//! Compatibility and persistence of recoverable investigation failure decisions.

use std::num::NonZeroU32;

use agent_core::state::State;

#[test]
fn a_state_before_investigation_failure_decisions_loads() {
    let state = State::from_json(include_str!("fixtures/state-before-investigation-failures.json")).unwrap();
    let team = &state.teams()[0];
    assert_eq!(team.investigations().len(), 1);
    assert_eq!(team.investigation_failure(NonZeroU32::MIN), None);
    let saved = serde_json::to_value(&state).unwrap();
    assert_eq!(saved["teams"][0].get("investigation_failures"), None);
}

#[test]
fn the_first_failure_decision_survives_later_stop_reports_and_reload() {
    let mut state = State::from_json(include_str!("fixtures/state-before-investigation-failures.json")).unwrap();
    let id = state.teams()[0].id().clone();
    let mut team = state.team_mut(&id).unwrap();
    team.record_investigation_failure(NonZeroU32::MIN, "no answer in 30 minutes".to_owned());
    team.record_investigation_failure(NonZeroU32::MIN, "stopped by the human".to_owned());
    let restored: State = serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
    assert_eq!(
        restored.teams()[0].investigation_failure(NonZeroU32::MIN),
        Some("no answer in 30 minutes")
    );
}
