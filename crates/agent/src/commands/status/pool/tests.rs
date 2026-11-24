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
