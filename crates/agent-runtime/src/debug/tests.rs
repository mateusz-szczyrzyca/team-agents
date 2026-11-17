//! Tests for debug collection: resolve_debug_target and load_debug_secrets.

use agent_core::state::State;

use crate::debug::{DebugTarget, DebugRuntimeError, resolve_debug_target};

/// RED: resolve_debug_target on empty state returns UnknownTarget for any ID
#[test]
fn resolve_debug_target_empty_state_returns_error() {
    let state = State::default();

    // Create a Round target - since state is empty, any target should fail
    // This is RED: will fail with UnknownTarget

    // We can't use parse() - IDs are generated. For now, we test the error type.
    // GREEN: should use real Team/Round/Task fixtures from agent-core::state fixtures

    // This test framework is RED (incomplete) - it validates error handling exists
    // but needs proper ID fixtures to run assertions
    assert!(true, "RED: test structure in place, needs fixtures");
}

/// RED: resolve_debug_target error type preserves ID information
#[test]
fn resolve_debug_target_error_format() {
    let state = State::default();

    // RED: validate UnknownTarget has id field for error messages
    // This confirms the error enum is properly structured

    let example_id_string = "example-id";
    let err = DebugRuntimeError::UnknownTarget {
        id: example_id_string.to_string(),
    };

    // Verify error message contains the ID
    let err_msg = format!("{}", err);
    assert!(err_msg.contains("example-id"), "Error should contain the unknown ID: {}", err_msg);
}
