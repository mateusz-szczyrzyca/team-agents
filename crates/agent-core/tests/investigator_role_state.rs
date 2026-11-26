//! Tests for [`Role::Investigator`] backward compatibility and round-tripping.

use agent_core::kinds::Role;
use agent_core::state::{SCHEMA_VERSION, State};

#[test]
fn state_before_investigator_role_loads_and_identifies_reviewer_task() {
    let old = include_str!("fixtures/state-before-investigator-role.json");
    let state = State::from_json(old).unwrap();
    let task_id = "task-20261111-153002-k3x9".parse().unwrap();
    let found = state.find_task(&task_id).expect("reviewer task must be found");
    assert_eq!(found.role, Role::Reviewer);
    assert_eq!(SCHEMA_VERSION, 15);
}

#[test]
fn state_with_investigator_task_identifies_role_and_round_trips() {
    let json = r#"{
  "schema_version": 15,
  "teams": [
    {
      "id": "team-20261111-153000-a1b2",
      "repo": "team-agents",
      "created_at": "2026-11-11T15:30:00Z",
      "merge": "autopilot",
      "task_list": "Build it.\n\n1. Feature\n[ ] 1.1 Item\n    Do it.\n",
      "items": [
        {
          "id": "1.1",
          "phase": {
            "phase": "pending"
          }
        }
      ],
      "rounds": [],
      "crew": {
        "kind": "solo",
        "member": {
          "task": "task-20261111-153001-k3x9",
          "agent": {
            "tier": "normal",
            "name": "claude-1"
          },
          "status": "waiting",
          "runs": [],
          "questions": []
        }
      },
      "reviewers": [
        {
          "task": "task-20261111-153003-k3x9",
          "agent": {
            "tier": "investigator",
            "name": "inv-codex"
          },
          "role": "investigator",
          "status": "running",
          "runs": [],
          "questions": []
        }
      ]
    }
  ]
}"#;
    let state = State::from_json(json).unwrap();
    let task_id = "task-20261111-153003-k3x9".parse().unwrap();
    let found = state.find_task(&task_id).expect("investigator task must be found");
    assert_eq!(found.role, Role::Investigator);

    let serialized = state.to_json().unwrap();
    let round_tripped = State::from_json(&serialized).unwrap();
    assert_eq!(round_tripped, state);
    let found_again = round_tripped.find_task(&task_id).unwrap();
    assert_eq!(found_again.role, Role::Investigator);
}
