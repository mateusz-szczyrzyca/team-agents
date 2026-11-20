//! An item follows its task, not the agent that started it: a member that is stopped does not hold the item its lead
//! delivered.

use agent_core::protocol::{Client, Command, ControlRequest, Reply, Target};
use agent_core::state::{ItemPhase, MergeMode};
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// Stops `task` as the human does with `agent stop <task>`.
async fn stop_task(rig: &Rig, task: &agent_core::ids::TaskId) {
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Task(task.clone()),
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "unexpected reply: {reply:?}");
}

// Rule: when the human stops a member and its lead ends DONE with a passed gate, the item the member held is published.
#[tokio::test]
async fn the_item_of_a_member_the_human_stopped_is_published_when_its_lead_delivers_it() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,work"),
            normal("claude-1", "hold"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    rig.wake_sleeping_teams();
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.status_is(&tasks[1], TaskStatus::Running).await;

    stop_task(&rig, &tasks[1]).await;
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;
    let state = rig
        .item_where("1.1", "published by the lead", |phase| {
            matches!(
                phase,
                ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. }
            )
        })
        .await;
    rig.stop().await.unwrap();

    let lead = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(lead.runs().len(), 2, "the lead slept, was woken, and delivered");
    let member = state.find_task(&tasks[1]).unwrap().member;
    assert_eq!(member.runs().len(), 1);
    assert_eq!(member.status(), TaskStatus::Stopped);
}
