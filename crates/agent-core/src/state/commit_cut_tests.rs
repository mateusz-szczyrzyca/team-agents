//! The runs the daemon cut for an order to commit, as the state keeps them: the clock of a round does not start again
//! after them (`[limits] round_minutes`). They are absent from states written before them.

use super::fixtures::{task, team_id, team_of};
use super::*;
use crate::ids::RunId;

/// A run ID of these tests.
fn run(n: u32) -> RunId {
    format!("run-20261126-0511{n:02}-c4km").parse().unwrap()
}

/// The state written before renewals; it has no commit cuts.
fn old_state() -> State {
    State::from_json(include_str!("../../tests/fixtures/state-before-renewals.json")).expect("a state of main loads")
}

/// A state with two teams: team 1 with tasks 1 to 3 and team 2 with tasks 4 to 6.
fn two_teams() -> State {
    let mut state = State::new();
    state.insert_team(team_of(1, 1)).unwrap();
    state.insert_team(team_of(2, 4)).unwrap();
    state
}

#[test]
fn a_state_written_before_the_commit_cuts_loads_with_none() {
    let state = old_state();
    let failed: TaskId = "task-20261120-144101-r7q2".parse().unwrap();

    assert!(!state.commit_cut(&failed, &run(1)));
}

#[test]
fn a_commit_cut_is_recorded_for_its_task_and_run_only() {
    let mut state = two_teams();

    state.record_commit_cut(task(2), run(1));

    assert!(state.commit_cut(&task(2), &run(1)));
    assert!(!state.commit_cut(&task(2), &run(2)));
    assert!(!state.commit_cut(&task(3), &run(1)));
}

#[test]
fn every_commit_cut_of_a_task_is_kept() {
    let mut state = two_teams();

    state.record_commit_cut(task(2), run(1));
    state.record_commit_cut(task(2), run(2));

    assert!(state.commit_cut(&task(2), &run(1)));
    assert!(state.commit_cut(&task(2), &run(2)));
}

#[test]
fn commit_cuts_survive_a_write_and_a_read() {
    let mut state = two_teams();
    state.record_commit_cut(task(2), run(1));

    let json = serde_json::to_string(&state).unwrap();
    let back = State::from_json(&json).unwrap();

    assert!(json.contains("\"commit_cuts\""), "{json}");
    assert!(back.commit_cut(&task(2), &run(1)));
    assert_eq!(back, state);
}

#[test]
fn states_that_differ_only_in_their_commit_cuts_are_not_equal() {
    let plain = two_teams();
    let mut cut = two_teams();
    cut.record_commit_cut(task(2), run(1));

    assert_ne!(cut, plain);
}

#[test]
fn a_state_without_commit_cuts_writes_no_commit_cut_key() {
    let json = serde_json::to_string(&old_state()).unwrap();

    assert!(!json.contains("commit_cuts"), "{json}");
}

#[test]
fn removing_a_team_forgets_the_commit_cuts_of_its_tasks_only() {
    let mut state = two_teams();
    state.record_commit_cut(task(2), run(1));
    state.record_commit_cut(task(5), run(2));

    state.remove_team(&team_id(1));

    assert!(!state.commit_cut(&task(2), &run(1)));
    assert!(state.commit_cut(&task(5), &run(2)));
}
