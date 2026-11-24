//! Results of superseded runs cannot remove a current assignment.

use super::super::fixture::{Daemon, pair_state};
use super::super::teams::Served;
use super::*;
use crate::fixtures::{at, task};
use agent_runtime::request_workspace::{Stage, WorkspaceRecord, branch};

#[tokio::test]
async fn late_observation_keeps_the_current_serving_assignment() {
    let daemon = Daemon::new(&pair_state(), None);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let request = agent_runtime::agentbus::RequestId::new(std::num::NonZeroU64::MIN);
    let worker = "claude-2".parse().unwrap();
    let workspace = RequestWorkspace::try_from(WorkspaceRecord {
        request,
        requester: "claude-1".parse().unwrap(),
        requester_task: task(1),
        requester_round: "round-20261124-120000-cccc".parse().unwrap(),
        requester_repo: "example".parse().unwrap(),
        worker,
        worker_task: task(2),
        branch: branch(&"claude-2".parse().unwrap(), &task(2)).unwrap(),
        base: "0123456789abcdef0123456789abcdef01234567".parse().unwrap(),
        stage: Stage::Working,
    })
    .unwrap();
    let current: RunId = "run-20261124-120001-dddd".parse().unwrap();
    let old: RunId = "run-20261124-120000-dddd".parse().unwrap();
    let dir = owner.parts.store.home().task_dir(&task(1));
    std::fs::create_dir_all(&dir).unwrap();
    recovery::prepare(&dir, &workspace).await.unwrap();
    recovery::advance(
        &dir,
        request,
        Transition::Assigned {
            workspace,
            run: current,
        },
    )
    .await
    .unwrap();
    owner.serving.insert(
        task(2),
        Served {
            request,
            requester: "claude-1".parse().unwrap(),
            requester_task: task(1),
            worker: "claude-2".parse().unwrap(),
            started: at(0),
            base: None,
        },
    );
    assert!(
        owner
            .observe_owned_request(&task(2), &old, WorkerEnding::Failed, "late".into())
            .await
            .unwrap()
    );
    assert!(owner.serving.contains_key(&task(2)));
    assert_eq!(recovery::load(&dir, request).await.unwrap().unwrap().ending(), None);
}

#[tokio::test]
async fn pending_result_requires_an_observed_current_ending() {
    let daemon = Daemon::new(&pair_state(), None);
    let (owner, _sinks) = daemon.bare_owner().await;
    let request = agent_runtime::agentbus::RequestId::new(std::num::NonZeroU64::MIN);
    let worker = "claude-2".parse().unwrap();
    let workspace = RequestWorkspace::try_from(WorkspaceRecord {
        request,
        requester: "claude-1".parse().unwrap(),
        requester_task: task(1),
        requester_round: "round-20261124-120000-cccc".parse().unwrap(),
        requester_repo: "example".parse().unwrap(),
        worker,
        worker_task: task(2),
        branch: branch(&"claude-2".parse().unwrap(), &task(2)).unwrap(),
        base: "0123456789abcdef0123456789abcdef01234567".parse().unwrap(),
        stage: Stage::Working,
    })
    .unwrap();
    let current: RunId = "run-20261124-120001-dddd".parse().unwrap();
    let dir = owner.parts.store.home().task_dir(&task(1));
    std::fs::create_dir_all(&dir).unwrap();
    recovery::prepare(&dir, &workspace).await.unwrap();
    recovery::advance(
        &dir,
        request,
        Transition::Assigned {
            workspace,
            run: current,
        },
    )
    .await
    .unwrap();
    let pending = agent_runtime::worker_request::Pending {
        request,
        team: "team-20261124-120000-aaaa".parse().unwrap(),
        requester: "claude-1".parse().unwrap(),
        requester_task: task(1),
        worker: "claude-2".parse().unwrap(),
        worker_task: task(2),
        summary: "old result".into(),
        run_seconds: 1,
        base: None,
        stage: agent_runtime::worker_request::Stage::Ended,
    };
    assert!(!owner.owns_observed_result(&pending).await.unwrap());
    recovery::advance(
        &dir,
        request,
        Transition::Observed {
            run: "run-20261124-120001-dddd".parse().unwrap(),
            ending: WorkerEnding::Done,
        },
    )
    .await
    .unwrap();
    assert!(owner.owns_observed_result(&pending).await.unwrap());
}
