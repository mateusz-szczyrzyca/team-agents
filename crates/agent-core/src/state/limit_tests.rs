//! The refusals of a provider as the state keeps them: how many refusals in a row a task had, and until when an agent
//! refused by its provider is kept out. Both are absent from states written before them.

use super::*;
use crate::ids::AgentName;

/// The task of these tests: the failed member of the fixture before renewals.
fn task() -> TaskId {
    "task-20261120-144101-r7q2".parse().unwrap()
}

/// The agent of these tests.
fn agent(name: &str) -> AgentName {
    name.parse().unwrap()
}

/// A moment of these tests.
fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

/// The state written before renewals, which holds the task; it has no refusals.
fn old_state() -> State {
    State::from_json(include_str!("../../tests/fixtures/state-before-renewals.json")).expect("a state of main loads")
}

#[test]
fn a_state_written_before_the_refusals_loads_with_none() {
    let state = old_state();

    assert_eq!(state.limit_refusals(&task()), 0);
    assert_eq!(state.refused_until(&agent("claude-1")), None);
}

#[test]
fn the_refusals_of_a_task_are_recorded_and_forgotten_at_zero() {
    let mut state = old_state();

    state.record_limit_refusals(task(), 2);
    assert_eq!(state.limit_refusals(&task()), 2);

    state.record_limit_refusals(task(), 0);
    assert_eq!(state.limit_refusals(&task()), 0);
}

#[test]
fn an_agent_refused_until_a_time_is_kept_out_until_it() {
    let mut state = old_state();
    let until = at("2026-11-25T10:07:00Z");

    state.record_refused_until(agent("claude-1"), Some(until));
    assert_eq!(state.refused_until(&agent("claude-1")), Some(until));
    assert_eq!(state.refused_until(&agent("claude-2")), None);

    state.record_refused_until(agent("claude-1"), None);
    assert_eq!(state.refused_until(&agent("claude-1")), None);
}

#[test]
fn refusals_and_refused_agents_survive_a_write_and_a_read() {
    let mut state = old_state();
    let until = at("2026-11-25T10:07:00Z");
    state.record_limit_refusals(task(), 3);
    state.record_refused_until(agent("claude-1"), Some(until));

    let json = serde_json::to_string(&state).unwrap();
    let back = State::from_json(&json).unwrap();

    assert_eq!(back.limit_refusals(&task()), 3);
    assert_eq!(back.refused_until(&agent("claude-1")), Some(until));
    assert_eq!(back, state);
}

#[test]
fn a_state_without_refusals_writes_no_refusal_key() {
    let json = serde_json::to_string(&old_state()).unwrap();

    assert!(!json.contains("limit_refusals"), "{json}");
    assert!(!json.contains("refused_agents"), "{json}");
}

#[test]
fn an_agent_is_refused_only_before_its_return_time() {
    let mut state = old_state();
    let until = at("2026-11-25T10:07:00Z");
    state.record_refused_until(agent("claude-1"), Some(until));

    assert!(state.refused_at(&agent("claude-1"), at("2026-11-25T10:06:59Z")));
    assert!(!state.refused_at(&agent("claude-1"), until));
    assert!(!state.refused_at(&agent("claude-2"), at("2026-11-25T10:06:59Z")));
}

#[test]
fn the_agents_refused_at_a_moment_are_listed_in_name_order() {
    let mut state = old_state();
    let until = at("2026-11-25T10:07:00Z");
    state.record_refused_until(agent("codex-1"), Some(until));
    state.record_refused_until(agent("claude-1"), Some(until));
    state.record_refused_until(agent("claude-2"), Some(at("2026-11-25T10:00:00Z")));

    let refused = state.refused_agents_at(at("2026-11-25T10:05:00Z"));

    assert_eq!(refused, vec![agent("claude-1"), agent("codex-1")]);
}
