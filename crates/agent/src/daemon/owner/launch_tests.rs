//! Shared launch accounting includes request-owned worker runs.

use std::num::NonZeroU64;

use agent_core::decide::slot::Going;
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::RequestId;

use super::fixture::{Daemon, pair_state};
use super::teams::Served;
use crate::fixtures::{at, task, team_id, worker};

#[tokio::test]
async fn pool_runs_hold_worker_slots_and_prevent_hand_over_to_their_agent() {
    let daemon = Daemon::new(&pair_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.serving.insert(
        task(4),
        Served {
            request: RequestId::new(NonZeroU64::new(1).unwrap()),
            requester: "claude-1".parse().unwrap(),
            requester_task: task(1),
            worker: "claude-2".parse().unwrap(),
            started: at(0),
            base: None,
        },
    );

    assert_eq!(
        owner.run_holding(&worker("claude-2")),
        Some(Going {
            agent: "claude-2".parse().unwrap(),
            team: team_id(0),
            task: task(4),
        })
    );
    assert!(owner.agents_in_runs().contains(&"claude-2"));
}

#[tokio::test]
async fn legacy_worker_run_recorded_as_serving_is_counted_once() {
    let mut state = pair_state();
    state
        .find_task_mut(&task(2))
        .unwrap()
        .1
        .transition(TaskStatus::Running)
        .unwrap();
    let daemon = Daemon::new(&state, None);
    let config = std::fs::read_to_string(daemon.home.config_file()).unwrap();
    std::fs::write(
        daemon.home.config_file(),
        format!("{config}\n[limits]\nruns_per_worker = 2\n"),
    )
    .unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.serving.insert(
        task(2),
        Served {
            request: RequestId::new(NonZeroU64::new(1).unwrap()),
            requester: "claude-1".parse().unwrap(),
            requester_task: task(1),
            worker: "claude-2".parse().unwrap(),
            started: at(0),
            base: None,
        },
    );
    assert_eq!(owner.run_holding(&worker("claude-2")), None);
}

#[tokio::test]
async fn launch_guard_refuses_full_process_slots_and_low_disk_then_recovers() {
    use agent_runtime::disk::{FakeFreeSpace, GIBIBYTE, SharedFreeSpace};

    let daemon = Daemon::new(&pair_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    for _ in 0..super::MAX_RUNS.get() {
        owner.runs.spawn(std::future::pending());
    }
    assert!(!owner.launch_allowed().await);
    owner.runs.abort_all();
    while let Some(joined) = owner.runs.join_next().await {
        assert!(joined.unwrap_err().is_cancelled());
    }
    let space = FakeFreeSpace::new(0);
    owner.parts.free_space = SharedFreeSpace::new(space.clone());
    assert!(!owner.launch_allowed().await);
    space.set(200 * GIBIBYTE);
    assert!(owner.launch_allowed().await);
}

#[tokio::test]
async fn pool_dispatch_keeps_requests_unclaimed_until_launch_guards_allow_them() {
    use agent_runtime::disk::{FakeFreeSpace, GIBIBYTE, SharedFreeSpace};
    use std::os::unix::fs::PermissionsExt;

    let scripts = tempfile::tempdir().unwrap();
    let bus = scripts.path().join("bus");
    let claimed = scripts.path().join("claimed");
    std::fs::write(&bus, r#"#!/bin/sh
case "$*" in
 *helper-list*) printf '%s\n' '[{"id":1,"from":"claude-1","from_task":"task-20261122-120000-aaaa","repo":"team-agents","branch":"main","status":"queued"}]' ;;
 *helper-next*) touch "$(dirname "$0")/claimed" ;;
 *) exit 1 ;;
esac
"#).unwrap();
    std::fs::set_permissions(&bus, std::fs::Permissions::from_mode(0o755)).unwrap();
    let daemon = Daemon::new(&pair_state(), Some(&bus));
    let config = std::fs::read_to_string(daemon.home.config_file()).unwrap();
    std::fs::write(
        daemon.home.config_file(),
        config.replace(
            "[tiers]",
            "[agents.glm-1]\ncli = \"claude\"\n[tiers]\nworkers = [\"glm-1\"]",
        ),
    )
    .unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let space = FakeFreeSpace::new(0);
    owner.parts.free_space = SharedFreeSpace::new(space.clone());

    owner.serve_pool_requests().await.unwrap();
    assert!(!claimed.exists(), "low disk must not claim a queued request");
    assert!(owner.serving.is_empty());
    space.set(200 * GIBIBYTE);
    for _ in 0..super::MAX_RUNS.get() {
        owner.runs.spawn(std::future::pending());
    }
    owner.serve_pool_requests().await.unwrap();
    assert!(!claimed.exists(), "full process slots must not claim a queued request");
    owner.runs.abort_all();
    while let Some(joined) = owner.runs.join_next().await {
        assert!(joined.unwrap_err().is_cancelled());
    }
    owner.serve_pool_requests().await.unwrap();
    assert!(
        claimed.exists(),
        "the queued request becomes claimable when both guards recover"
    );
}

#[tokio::test]
async fn claimed_legacy_request_does_not_hold_a_run_slot_before_its_worker_starts() {
    let daemon = Daemon::new(&pair_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner.serving.insert(
        task(2),
        Served {
            request: RequestId::new(NonZeroU64::new(1).unwrap()),
            requester: "claude-1".parse().unwrap(),
            requester_task: task(1),
            worker: "claude-2".parse().unwrap(),
            started: at(0),
            base: None,
        },
    );

    assert_eq!(owner.run_holding(&worker("claude-2")), None);
    assert!(!owner.agents_in_runs().contains(&"claude-2"));
}

#[tokio::test]
async fn pool_recovery_does_not_requeue_a_starting_request_owned_by_a_state_team() {
    use std::os::unix::fs::PermissionsExt;

    let scripts = tempfile::tempdir().unwrap();
    let bus = scripts.path().join("bus");
    let reset = scripts.path().join("reset");
    let request = serde_json::json!([{
        "id": 1, "from": "claude-1", "from_task": task(1),
        "helper_task": task(2), "worker": "claude-2", "repo": "team-agents",
        "branch": "main", "status": "starting"
    }]);
    std::fs::write(
        &bus,
        format!("#!/bin/sh\ncase \"$*\" in\n*helper-list*) printf '%s\\n' '{request}' ;;\n*helper-set*) touch '{}' ;;\n*) exit 1 ;;\nesac\n", reset.display()),
    ).unwrap();
    std::fs::set_permissions(&bus, std::fs::Permissions::from_mode(0o755)).unwrap();
    let daemon = Daemon::new(&pair_state(), Some(&bus));
    let config = std::fs::read_to_string(daemon.home.config_file()).unwrap();
    std::fs::write(
        daemon.home.config_file(),
        config.replace("[tiers]", "[tiers]\nworkers = [\"claude-2\"]"),
    )
    .unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner.serve_pool_requests().await.unwrap();

    assert!(!reset.exists(), "pool recovery must leave a state-team claim alone");
}
