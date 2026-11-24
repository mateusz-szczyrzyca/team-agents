//! The reason a declared failure is kept under is saved as the word `failed`, and the states written before the reason
//! existed still load: the fixture of the state before it is read by the migration tests.

use agent_core::decide::ladder::Trouble;
use agent_core::state::State;

#[test]
fn a_declared_failure_is_saved_as_the_word_failed_and_read_back() {
    assert_eq!(serde_json::to_string(&Trouble::Failed).unwrap(), "\"failed\"");
    assert_eq!(serde_json::from_str::<Trouble>("\"failed\"").unwrap(), Trouble::Failed);
}

#[test]
fn a_state_written_before_the_declared_failure_reason_still_loads() {
    let fixture = include_str!("fixtures/state-before-no-tool-call.json");

    let state = State::from_json(fixture).unwrap();

    assert!(
        state
            .teams()
            .iter()
            .flat_map(|team| team.crew().roster())
            .all(|(_, member)| member
                .renewals()
                .is_none_or(|renewals| renewals.reason() != Trouble::Failed)),
        "the fixture predates the reason, so no renewal of it has it"
    );
}
