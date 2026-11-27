//! The values of the variables that tell a checkpoint.

use std::str::FromStr;

use agent_core::prompt::Variables;

use super::*;

/// The value of the variable `name`.
fn value(values: &Variables, name: &str) -> String {
    let name = name.parse().expect("a variable name");
    values.get(&name).unwrap_or("<missing>").to_owned()
}

/// The time the checkpoints of these tests were taken.
fn at() -> Timestamp {
    Timestamp::from_str("2026-11-27T05:00:00Z").unwrap()
}

#[test]
fn a_checkpoint_with_a_content_gives_its_three_parts_its_cause_its_time_and_its_head() {
    let content = CheckpointContent {
        meant: "Task: Fix the parser".to_owned(),
        done: "R1 met".to_owned(),
        left: "next: wait for the merge".to_owned(),
    };
    let head = CommitId::from_str("a1b2c3d4e5f60718293a4b5c6d7e8f9012345678").unwrap();
    let values = checkpoint_state_variables(&CheckpointFacts {
        cause: CheckpointCause::RoundOverdue,
        at: at(),
        head: Some(&head),
        content: Some(&content),
    })
    .unwrap();

    assert_eq!(value(&values, "meant"), "Task: Fix the parser");
    assert_eq!(value(&values, "done"), "R1 met");
    assert_eq!(value(&values, "left"), "next: wait for the merge");
    assert_eq!(value(&values, "cause"), "round_overdue");
    assert_eq!(value(&values, "time"), "2026-11-27T05:00:00Z");
    assert_eq!(value(&values, "head"), "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678");
}

#[test]
fn a_checkpoint_with_no_content_and_no_head_says_so_in_place_of_its_parts() {
    let values = checkpoint_state_variables(&CheckpointFacts {
        cause: CheckpointCause::Commit,
        at: at(),
        head: None,
        content: None,
    })
    .unwrap();

    for part in ["meant", "done", "left"] {
        assert_eq!(value(&values, part), NO_CONTENT);
    }
    assert_eq!(value(&values, "head"), "none");
}
