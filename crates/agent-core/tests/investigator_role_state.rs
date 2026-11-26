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

#[test]
fn team_summary_preserves_investigator_and_legacy_reviewer_roles() {
    for role in [None, Some(Role::Reviewer), Some(Role::Investigator)] {
        let mut json: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/state-before-investigator-role.json")).unwrap();
        if let Some(role) = role {
            json["teams"][0]["reviewers"][0]["role"] = serde_json::to_value(role).unwrap();
        }
        let state = State::from_json(&json.to_string()).unwrap();
        let summary = agent_core::protocol::TeamSummary::from(&state.teams()[0]);
        let roles: Vec<_> = summary.members.iter().map(|member| member.role).collect();
        assert_eq!(roles, vec![Role::Solo, role.unwrap_or(Role::Reviewer)]);
    }
}

#[test]
fn round_members_preserve_investigator_and_legacy_reviewer_roles() {
    for role in [None, Some(Role::Reviewer), Some(Role::Investigator)] {
        let mut json: serde_json::Value =
            serde_json::from_str(include_str!("fixtures/state-before-investigator-role.json")).unwrap();
        if let Some(role) = role {
            json["teams"][0]["reviewers"][0]["role"] = serde_json::to_value(role).unwrap();
        }
        let mut team: agent_core::state::Team = serde_json::from_value(json["teams"][0].clone()).unwrap();
        let round: agent_core::ids::RoundId = "round-20261111-153004-k3x9".parse().unwrap();
        let at = "2026-11-11T15:31:00Z".parse().unwrap();
        team.open_round(round.clone(), Some("1.1".parse().unwrap()), at)
            .unwrap();
        let task = team.reviewers()[0].task().clone();
        team.begin_run(
            &task,
            agent_core::state::NewRun {
                id: "run-20261111-153005-k3x9".parse().unwrap(),
                at,
                session: None,
                rounds: vec![round.clone()],
            },
        )
        .unwrap();
        let members: Vec<_> = team
            .members_in(&round)
            .into_iter()
            .map(|(role, member)| (role, member.task().clone()))
            .collect();
        assert_eq!(members, vec![(role.unwrap_or(Role::Reviewer), task)]);
    }
}
