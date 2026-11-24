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

// A state written before the ladder took declared failures holds a failed member; it loads as it was and stays failed:
// the daemon does not send it up the ladder by itself at its next start, only `agent resume` starts it again.
#[test]
fn a_failed_member_of_a_state_written_before_the_ladder_loads_as_failed() {
    let fixture = include_str!("fixtures/state-before-no-tool-call.json");

    let state = State::from_json(fixture).unwrap();

    let failed: Vec<_> = state
        .teams()
        .iter()
        .flat_map(|team| team.crew().roster())
        .filter(|(_, member)| member.status() == agent_core::status::TaskStatus::Failed)
        .collect();
    assert_eq!(failed.len(), 1, "the fixture holds one failed member");
}

// The state of a team as the version before the ladder took declared failures wrote it, with a failed member whose only
// run reported FAILED: it loads, the member is failed, and it holds no rung of the ladder.
#[test]
fn a_team_state_of_the_old_format_with_a_failed_member_loads() {
    let fixture = include_str!("fixtures/state-before-declared-failure-ladder.json");

    let state = State::from_json(fixture).unwrap();

    let members = state.teams()[0].crew().roster();
    let failed: Vec<_> = members
        .iter()
        .filter(|(_, member)| member.status() == agent_core::status::TaskStatus::Failed)
        .collect();
    assert_eq!(failed.len(), 1, "one failed member among {} tasks", members.len());
    assert_eq!(failed[0].1.agent().name(), "claude-1");
    assert!(failed[0].1.renewals().is_none(), "no rung was used");
}
