//! The moment a queued task began to wait for its busy agent: kept while it waits, ended by a run or a hand-over, and
//! absent from states written before it.

use super::*;
use crate::ids::AgentName;

/// The task of these tests: the failed member of the fixture before renewals.
fn task() -> TaskId {
    "task-20261120-144101-r7q2".parse().unwrap()
}

/// The state written before renewals, which holds the task.
fn old_state() -> State {
    State::from_json(include_str!("../../tests/fixtures/state-before-renewals.json")).expect("a state of main loads")
}

/// A moment of these tests.
fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

#[test]
fn a_state_written_before_the_wait_loads_with_no_wait() {
    let state = old_state();

    let found = state.find_task(&task()).expect("the task is there");
    assert_eq!(found.member.waiting_since(), None);
}

#[test]
fn the_moment_a_wait_began_is_kept_until_the_wait_ends() {
    let mut state = old_state();
    let first = at("2026-11-21T10:00:00Z");

    assert_eq!(state.start_waiting(&task(), first), Some(first));
    assert_eq!(state.start_waiting(&task(), at("2026-11-21T10:05:00Z")), Some(first));
    assert_eq!(state.find_task(&task()).unwrap().member.waiting_since(), Some(first));
}

#[test]
fn a_wait_of_no_task_is_refused_with_none() {
    let mut state = old_state();
    let other = "task-20261120-144101-zzzz".parse().unwrap();

    assert_eq!(state.start_waiting(&other, at("2026-11-21T10:00:00Z")), None);
}

#[test]
fn a_run_ends_the_wait() {
    let mut state = old_state();
    state.start_waiting(&task(), at("2026-11-21T10:00:00Z"));
    let (_, mut member) = state.find_task_mut(&task()).expect("the task");

    member
        .begin_run(
            "run-20261121-100100-a1b2".parse().unwrap(),
            at("2026-11-21T10:01:00Z"),
            None,
        )
        .expect("the run begins");

    assert_eq!(state.find_task(&task()).unwrap().member.waiting_since(), None);
}

#[test]
fn a_hand_over_ends_the_wait_for_the_agent_that_takes_the_task() {
    let mut state = old_state();
    state.start_waiting(&task(), at("2026-11-21T10:00:00Z"));
    let holder: AgentName = "claude-2".parse().expect("an agent name");

    state
        .hand_over_task(&task(), Participant::Normal(holder), at("2026-11-21T10:06:00Z"))
        .expect("the hand-over");

    assert_eq!(state.find_task(&task()).unwrap().member.waiting_since(), None);
}

#[test]
fn a_wait_is_written_and_read_back() {
    let mut state = old_state();
    state.start_waiting(&task(), at("2026-11-21T10:00:00Z"));

    let text = state.to_json().expect("the state is written");
    let read = State::from_json(&text).expect("the state is read back");

    assert_eq!(
        read.find_task(&task()).unwrap().member.waiting_since(),
        Some(at("2026-11-21T10:00:00Z"))
    );
}
