//! Schema 13 as written before part rounds remains readable without rewriting its history.

use agent_core::state::State;

#[test]
fn schema_13_before_part_rounds_keeps_its_complete_saved_document() {
    let text = include_str!("fixtures/state-v0-29-before-part-rounds.json");
    let state = State::from_json(text).unwrap();
    assert_eq!(state.to_json().unwrap(), text);
    assert!(state.teams().iter().all(|team| team.splits().is_empty()));
    assert!(
        state
            .teams()
            .iter()
            .flat_map(agent_core::state::Team::rounds)
            .all(|round| round.part().is_none())
    );
}
