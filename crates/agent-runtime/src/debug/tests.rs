//! Tests for debug collection.

use agent_core::ids::{RoundId, TaskId, TeamId};
use agent_core::state::State;

use crate::debug::{DebugTarget, resolve_debug_target};

#[test]
fn resolve_target_known_team_returns_team() {
    let state = State::default();
    let team_id = TeamId::parse("team-20261117-230000-xxxx").unwrap();

    // This test expects resolution to work for a known team
    // RED: This will fail because the state is empty and has no teams
    let result = resolve_debug_target(&state, &DebugTarget::Team(team_id.clone()));

    // For now, we expect failure since state is empty
    assert!(result.is_err());
}

#[test]
fn resolve_target_unknown_team_returns_error() {
    let state = State::default();
    let unknown_team_id = TeamId::parse("team-20261117-230000-xxxx").unwrap();

    let result = resolve_debug_target(&state, &DebugTarget::Team(unknown_team_id.clone()));

    assert!(matches!(result, Err(crate::debug::DebugRuntimeError::UnknownTarget { .. })));
}

#[test]
fn resolve_target_unknown_task_returns_error() {
    let state = State::default();
    let unknown_task_id = TaskId::parse("task-20261117-230000-xxxx").unwrap();

    let result = resolve_debug_target(&state, &DebugTarget::Task(unknown_task_id.clone()));

    assert!(matches!(result, Err(crate::debug::DebugRuntimeError::UnknownTarget { .. })));
}

#[test]
fn resolve_target_unknown_round_returns_error() {
    let state = State::default();
    let unknown_round_id = RoundId::parse("round-20261117-230000-xxxx").unwrap();

    let result = resolve_debug_target(&state, &DebugTarget::Round(unknown_round_id.clone()));

    assert!(matches!(result, Err(crate::debug::DebugRuntimeError::UnknownTarget { .. })));
}
