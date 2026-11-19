//! Integration tests for disk space monitoring, target cleanup and run refusals.

use std::str::FromStr;
use std::time::Duration;

use agent_core::config::AbsolutePath;
use agent_core::ids::AgentName;
use agent_core::protocol::{AgentList, Client, Command, FailureKind, RunRequest};
use agent_core::state::{ItemPhase, MergeMode, TaskListText};

use super::rig::{CHAIN, ONE, Rig, investigator, normal};
use crate::client::{ClientError, Connection};

/// A published lead and two approving reviewers.
async fn rig() -> Rig {
    let rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    std::fs::write(rig.repo.join(".git/info/exclude"), ".fake-*\n").unwrap();
    rig
}

/// Waits for a notification containing `needle`.
async fn wait_for_notification(rig: &Rig, needle: &str) -> String {
    rig.wait_for(needle, || rig.notifications().iter().any(|text| text.contains(needle)))
        .await;
    rig.notifications()
        .into_iter()
        .find(|text| text.contains(needle))
        .unwrap()
}

#[tokio::test]
async fn run_request_refused_below_min_free_space() {
    let mut rig = rig().await;
    // Set free space below min_free_gb (20 GB).
    rig.free_space.set(10 * 1024 * 1024 * 1024);
    rig.start().await;

    let names: Vec<AgentName> = vec!["claude-1".parse().unwrap()];
    let request = RunRequest {
        repo: "example".parse().unwrap(),
        checkout: AbsolutePath::from_str(rig.repo.to_str().unwrap()).unwrap(),
        agents: AgentList::try_from(names).unwrap(),
        task_list: TaskListText::new(ONE.to_owned()).unwrap(),
        coordinator: None,
        merge: MergeMode::Autopilot,
    };
    let result = Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, Command::Run(request))
        .await;

    match result {
        Err(ClientError::Refused(failure)) => {
            assert_eq!(failure.kind, FailureKind::Refused);
            assert!(
                failure
                    .message
                    .contains("not enough free disk space: 10 GB free, minimum is 20 GB"),
                "unexpected message: {}",
                failure.message
            );
            assert!(
                failure
                    .message
                    .contains("remove build directories or unused worktrees in"),
                "unexpected message: {}",
                failure.message
            );
        }
        other => panic!("expected refused, got {other:?}"),
    }

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn low_disk_notifies_once_and_removes_reviewer_target_on_settle() {
    let mut rig = rig().await;
    // Set space below low_free_gb (100 GB) but above min_free_gb (20 GB).
    rig.free_space.set(50 * 1024 * 1024 * 1024);
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;

    // Wait until lead finishes and reviewers are active / approved.
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;

    // Check that low disk notification was delivered.
    let notice = wait_for_notification(&rig, "Low free disk space").await;
    assert!(notice.contains("Low free disk space"));
    assert!(notice.contains("50 GB free"));
    assert!(notice.contains("configured limit of 100 GB"));

    // Verify reviewer target removal on settle.
    let reviewer = state.team(&team).unwrap().reviewers()[0].task().clone();
    let tree = rig.worktree(&reviewer);
    assert!(tree.exists());
    assert!(!tree.join("target").exists(), "reviewer target should not exist");

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn queued_tasks_wait_below_min_free_space_and_resume_when_space_back() {
    let mut rig = rig().await;
    rig.start().await;
    rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;

    // 1.1 finishes and merges.
    rig.item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;

    // Drop free space below min_free_gb (20 GB).
    rig.free_space.set(10 * 1024 * 1024 * 1024);

    // Wait for the disk wait notification.
    let notice = wait_for_notification(&rig, "Rounds waiting for free disk space").await;
    assert!(notice.contains("Rounds waiting for free disk space"));
    assert!(notice.contains("10 GB free"));
    assert!(notice.contains("minimum threshold of 20 GB"));

    // Task 1.2 should not start yet; its task is queued waiting for disk space.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let state = rig.state();
    let task_1 = state.teams()[0].crew().roster()[0].1.task();
    assert_eq!(
        state.find_task(task_1).unwrap().member.status(),
        agent_core::status::TaskStatus::Queued,
        "task should be queued waiting for disk space"
    );

    // Restore space above min_free_gb.
    rig.free_space.set(200 * 1024 * 1024 * 1024);

    // 1.2 should now start and finish.
    rig.item_where("1.2", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;

    rig.stop().await.unwrap();
}
