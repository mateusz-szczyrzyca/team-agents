//! The schema before task permissions remains readable without granting permissions.
use super::*;

#[test]
fn schema_nine_migrates_without_allowances_and_writes_schema_ten() {
    let old = include_str!("../../tests/fixtures/state-before-task-allowances.json");
    let state = State::from_json(old).unwrap();
    assert_eq!(SCHEMA_VERSION, 10);
    for team in state.teams() {
        assert_eq!(team.allowances(), []);
    }
    assert_eq!(
        state.to_json().unwrap(),
        old.replacen("\"schema_version\": 9", "\"schema_version\": 10", 1)
    );
    assert_eq!(State::from_json(&state.to_json().unwrap()).unwrap(), state);
}
