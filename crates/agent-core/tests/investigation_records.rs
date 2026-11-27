//! Compatibility and durable retry exclusions for investigation requests.

use agent_core::state::{InvestigationRecord, State};

#[test]
fn a_state_before_investigation_records_loads() {
    let state = State::from_json(include_str!("fixtures/state-before-investigation-records.json")).unwrap();
    assert_eq!(state.teams().len(), 1);
    assert_eq!(state.teams()[0].investigations(), &[]);
}

#[test]
fn current_attempt_and_failed_investigators_survive_state_reload() {
    let mut state = State::from_json(include_str!("fixtures/state-before-investigation-records.json")).unwrap();
    let team = state.teams()[0].id().clone();
    let record = InvestigationRecord {
        id: std::num::NonZeroU32::MIN,
        task: "task-20261111-153002-k3x9".parse().unwrap(),
        started: "2026-11-11T15:32:00Z".parse().unwrap(),
        failed_with: vec!["inv-codex".parse().unwrap()],
    };
    state.team_mut(&team).unwrap().record_investigation(record.clone());
    assert_eq!(state.teams()[0].investigations(), std::slice::from_ref(&record));
    let restored: State = serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
    assert_eq!(restored.teams()[0].investigations(), &[record]);
}
