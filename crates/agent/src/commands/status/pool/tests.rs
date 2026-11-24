use super::*;
use serde_json::json;

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
    let helpers_val = json!({
        "1": {
            "status": "running",
            "worker": "glm-1",
            "from_task": "task-20261124-114140-pq68",
            "step": "1.1"
        }
    });
    let helpers = helpers_val.as_object().unwrap();

    let rendered = render_pool(&workers, &[], Some(helpers)).unwrap();
    assert_eq!(
        rendered,
        "\
pool:
  glm-1  task-20261124-114140-pq68 (step 1.1)
  glm-2  free
"
    );
}
