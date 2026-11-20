//! A coordinator's already-merged claim completes the next item without publishing again.

use agent_core::state::{Completion, ItemPhase, MergeMode, RunState};
use agent_core::status::TaskStatus;

use super::rig::{CHAIN, Rig, investigator, normal};

#[tokio::test]
async fn delivery_claim_after_an_earlier_team_merge_finishes_two_items_with_one_pr() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate,done:DELIVERED: #1"),
            normal("claude-1", "commit-clean,done:Already delivered"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(&["inv-lead", "claude-1"], CHAIN, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.state_where("the coordinator's third run to settle", |state| {
        let lead = state.team(&team).unwrap().find(&tasks[0]).unwrap().1;
        lead.runs().len() >= 3
            && lead
                .runs()
                .last()
                .is_some_and(|run| matches!(run.state(), RunState::Ended { .. }))
    })
    .await;
    for _ in 0..3 {
        let reply = crate::client::Connection::connect(&rig.home.socket_file())
            .await
            .unwrap()
            .request(
                agent_core::protocol::Client::Cli,
                agent_core::protocol::Command::Status(agent_core::protocol::StatusRequest { target: None }),
            )
            .await
            .unwrap();
        assert!(matches!(reply, agent_core::protocol::Reply::Teams { .. }));
    }
    let state = rig.state();
    rig.stop().await.unwrap();
    let found = state.team(&team).unwrap();
    for id in ["1.1", "1.2"] {
        let item = found.item(id.parse().unwrap()).unwrap();
        assert!(
            matches!(item.phase(), ItemPhase::Done { completion: Completion::Merged { pr, .. } } if pr.number.get().get() == 1),
            "{id}: {:?}",
            item.phase()
        );
    }
    assert_eq!(rig.github.pulls().len(), 1);
    assert_eq!(found.find(&tasks[1]).unwrap().1.runs().len(), 2);
    assert_eq!(found.find(&tasks[0]).unwrap().1.status(), TaskStatus::Done);
    assert!(found.rounds().iter().all(|round| !round.is_open()));
    assert!(!rig.worktree(&tasks[1]).exists());
}

async fn assert_delivery_refused(
    lead: &'static str,
    member: &'static str,
    reason: &str,
) -> Result<(), crate::daemon::DaemonError> {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", lead),
            normal("claude-1", member),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(&["inv-lead", "claude-1"], CHAIN, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    let state = rig
        .state_where("delivery refusal or an unexpected second publication", |state| {
            let found = state.team(&team).unwrap();
            found.find(&tasks[0]).unwrap().1.runs().len() >= 4
                || matches!(
                    found.item("1.2".parse().unwrap()).unwrap().phase(),
                    ItemPhase::Done { .. }
                )
        })
        .await;
    assert_eq!(rig.github.pulls().len(), 1, "a refused claim must not publish an item");
    let prompt = rig.prompt(&tasks[0], 4).await;
    rig.stop().await?;
    assert!(prompt.contains(reason), "{reason}: {prompt}");
    let found = state.team(&team).unwrap();
    let round = found.round_of("1.2".parse().unwrap()).unwrap();
    assert_eq!(round.delivered_by(), None);
    assert!(round.is_open());
    assert!(matches!(
        found.item("1.2".parse().unwrap()).unwrap().phase(),
        ItemPhase::InProgress { .. }
    ));
    assert_eq!(rig.github.pulls().len(), 1);
    Ok(())
}

#[tokio::test]
async fn delivery_refuses_an_unknown_team_pr_and_tells_the_lead_why() {
    assert_delivery_refused(
        "sleep,integrate,done:DELIVERED: #99,sleep",
        "commit-clean,done:Already delivered",
        "PR #99 belongs to no other round of this team",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn delivery_refuses_private_commits_and_tells_the_lead_why() {
    assert_delivery_refused(
        "sleep,integrate,done:DELIVERED: #1,sleep",
        "commit-clean,work",
        "the item has commits outside the PR and base branches",
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn delivery_refuses_a_dirty_worktree_and_tells_the_lead_why() {
    assert_delivery_refused(
        "sleep,integrate,done:DELIVERED: #1,sleep",
        "commit-clean,dirty-done",
        "the item's worktree has uncommitted changes",
    )
    .await
    .unwrap();
}
