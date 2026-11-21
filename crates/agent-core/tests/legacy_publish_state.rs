//! State written before publish_tries and publish_question existed loads with default 0 and None.

use agent_core::state::State;

#[test]
fn state_before_publish_tries_loads_with_default_zero_and_none() {
    let text = include_str!("fixtures/state-before-publish-tries.json");
    let state = State::from_json(text).expect("state before publish_tries should load cleanly");
    for team in state.teams() {
        for item in team.items() {
            assert_eq!(item.publish_tries(), 0);
            assert_eq!(item.publish_question(), None);
        }
    }
}
