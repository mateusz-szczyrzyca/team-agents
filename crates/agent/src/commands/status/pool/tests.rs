use std::num::NonZeroU64;

use super::*;
use agent_runtime::agentbus::{DelegationStatus, RequestId};

#[test]
fn empty_pool_is_none() {
    assert_eq!(render_pool(&[], &[], None), None);
}

#[test]
fn free_workers_say_so() {
    let workers = vec!["glm-1".parse().unwrap(), "glm-2".parse().unwrap()];
    let rendered = render_pool(&workers, &[], None).unwrap();
    assert_eq!(
        rendered,
        "\
pool:
  glm-1  free
  glm-2  free
"
    );
}

#[test]
fn worker_serving_a_team_request_shows_team_and_step() {
    let workers = vec!["glm-1".parse().unwrap(), "glm-2".parse().unwrap()];
    let helpers = vec![HelperEntry {
        id: RequestId::new(NonZeroU64::new(1).unwrap()),
        from: "claude-1".parse().unwrap(),
        from_task: "task-20261124-114140-pq68".parse().unwrap(),
        repo: "example".parse().unwrap(),
        branch: "agents/claude-1/test".parse().unwrap(),
        status: DelegationStatus::Running,
        worker: Some("glm-1".parse().unwrap()),
        requires: vec![],
        step: Some("1.1".to_owned()),
        request: "do something".to_owned(),
    }];

    let rendered = render_pool(&workers, &[], Some(&helpers)).unwrap();
    assert_eq!(
        rendered,
        "\
pool:
  glm-1  task-20261124-114140-pq68 (step 1.1)
  glm-2  free
"
    );
}

#[test]
fn worker_serving_a_team_request_with_matched_team_shows_team_id_and_step() {
    let workers = vec!["glm-1".parse().unwrap(), "glm-2".parse().unwrap()];
    let helpers = vec![HelperEntry {
        id: RequestId::new(NonZeroU64::new(1).unwrap()),
        from: "claude-1".parse().unwrap(),
        from_task: "task-20261124-114140-pq68".parse().unwrap(),
        repo: "example".parse().unwrap(),
        branch: "agents/claude-1/test".parse().unwrap(),
        status: DelegationStatus::Running,
        worker: Some("glm-1".parse().unwrap()),
        requires: vec![],
        step: Some("1.1".to_owned()),
        request: "do something".to_owned(),
    }];
    let team: TeamSummary = serde_json::from_value(serde_json::json!({
        "id": "team-20261124-114140-team",
        "repo": "example",
        "created_at": "2026-11-24T00:00:00Z",
        "merge": "autopilot",
        "items": {"pending": 0, "in_progress": 0, "in_review": 0, "approved": 0, "done": 0},
        "members": [{
            "task": "task-20261124-114140-pq68",
            "agent": {"tier": "normal", "name": "claude-1"},
            "role": "member",
            "status": "running"
        }]
    }))
    .unwrap();

    let rendered = render_pool(&workers, &[team], Some(&helpers)).unwrap();
    assert_eq!(
        rendered,
        "\
pool:
  glm-1  team-20261124-114140-team (step 1.1)
  glm-2  free
"
    );
}

#[test]
fn worker_in_active_legacy_crew_shows_team_id() {
    let workers = vec!["glm-1".parse().unwrap(), "glm-2".parse().unwrap()];
    let team: TeamSummary = serde_json::from_value(serde_json::json!({
        "id": "team-20261124-114140-legc",
        "repo": "example",
        "created_at": "2026-11-24T00:00:00Z",
        "merge": "autopilot",
        "items": {"pending": 0, "in_progress": 0, "in_review": 0, "approved": 0, "done": 0},
        "members": [{
            "task": "task-20261124-114140-w111",
            "agent": {"tier": "worker", "name": "glm-1"},
            "role": "worker",
            "status": "waiting"
        }]
    }))
    .unwrap();

    let rendered = render_pool(&workers, &[team], None).unwrap();
    assert_eq!(
        rendered,
        "\
pool:
  glm-1  team-20261124-114140-legc
  glm-2  free
"
    );
}

#[test]
fn worker_in_stopped_legacy_team_is_free() {
    let workers = vec!["glm-1".parse().unwrap(), "glm-2".parse().unwrap()];
    let team: TeamSummary = serde_json::from_value(serde_json::json!({
        "id": "team-20261124-114140-legc",
        "repo": "example",
        "created_at": "2026-11-24T00:00:00Z",
        "merge": "autopilot",
        "items": {"pending": 0, "in_progress": 0, "in_review": 0, "approved": 0, "done": 0},
        "members": [
            {
                "task": "task-20261124-114140-lead",
                "agent": {"tier": "normal", "name": "claude-1"},
                "role": "coordinator",
                "status": "stopped"
            },
            {
                "task": "task-20261124-114140-w111",
                "agent": {"tier": "worker", "name": "glm-1"},
                "role": "worker",
                "status": "stopped"
            }
        ]
    }))
    .unwrap();

    let rendered = render_pool(&workers, &[team], None).unwrap();
    assert_eq!(
        rendered,
        "\
pool:
  glm-1  free
  glm-2  free
"
    );
}

#[test]
fn worker_with_unsettled_workspace_shows_team_and_step_when_helper_is_done() {
    let workers = vec!["glm-1".parse().unwrap(), "glm-2".parse().unwrap()];
    let helpers = vec![HelperEntry {
        id: RequestId::new(NonZeroU64::new(1).unwrap()),
        from: "claude-1".parse().unwrap(),
        from_task: "task-20261124-114140-pq68".parse().unwrap(),
        repo: "example".parse().unwrap(),
        branch: "agents/claude-1/test".parse().unwrap(),
        status: DelegationStatus::Done,
        worker: Some("glm-1".parse().unwrap()),
        requires: vec![],
        step: Some("1.1".to_owned()),
        request: "do something".to_owned(),
    }];
    let team: TeamSummary = serde_json::from_value(serde_json::json!({
        "id": "team-20261124-114140-team",
        "repo": "example",
        "created_at": "2026-11-24T00:00:00Z",
        "merge": "autopilot",
        "items": {"pending": 0, "in_progress": 0, "in_review": 0, "approved": 0, "done": 0},
        "members": [{
            "task": "task-20261124-114140-pq68",
            "agent": {"tier": "normal", "name": "claude-1"},
            "role": "member",
            "status": "running"
        }]
    }))
    .unwrap();
    let unsettled = vec![UnsettledWorker {
        worker: Some("glm-1".parse().unwrap()),
        from_task: "task-20261124-114140-pq68".to_owned(),
        step: Some("1.1".to_owned()),
        damaged: false,
    }];

    let rendered = render_pool_with_unsettled(&workers, &[team], Some(&helpers), Some(&unsettled)).unwrap();
    assert_eq!(
        rendered,
        "\
pool:
  glm-1  team-20261124-114140-team (step 1.1)
  glm-2  free
"
    );
}

#[test]
fn damaged_workspace_record_renders_unsettled() {
    let workers = vec!["glm-1".parse().unwrap(), "glm-2".parse().unwrap()];
    let unsettled = vec![UnsettledWorker {
        worker: Some("glm-1".parse().unwrap()),
        from_task: String::new(),
        step: None,
        damaged: true,
    }];

    let rendered = render_pool_with_unsettled(&workers, &[], None, Some(&unsettled)).unwrap();
    assert_eq!(
        rendered,
        "\
pool:
  glm-1  unsettled
  glm-2  free
"
    );
}

#[test]
fn terminal_workers_remain_assigned_to_an_active_legacy_crew() {
    let workers = vec!["glm-1".parse().unwrap()];
    for status in ["done", "failed", "stopped"] {
        let team: TeamSummary = serde_json::from_value(serde_json::json!({
            "id": "team-20261124-114140-team", "repo": "example",
            "created_at": "2026-11-24T00:00:00Z", "merge": "autopilot",
            "items": {"pending": 0, "in_progress": 0, "in_review": 0, "approved": 0, "done": 0},
            "members": [
                {"task": "task-20261124-114140-lead", "agent": {"tier": "normal", "name": "claude-1"},
                 "role": "coordinator", "status": "running"},
                {"task": "task-20261124-114140-work", "agent": {"tier": "worker", "name": "glm-1"},
                 "role": "worker", "status": status}
            ]
        }))
        .unwrap();
        assert_eq!(
            render_pool(&workers, &[team], None),
            Some("pool:\n  glm-1  team-20261124-114140-team\n".to_owned()),
            "{status}"
        );
    }
}
