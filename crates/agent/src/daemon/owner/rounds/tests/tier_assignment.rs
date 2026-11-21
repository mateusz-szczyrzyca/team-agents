//! Tier requirements at the daemon boundary, including first-part handoffs and restart.

use super::rig::{ONE, Rig, investigator, normal};
use agent_core::kinds::Tier;
use agent_core::state::{ItemPhase, MergeMode};
use agent_core::status::TaskStatus;

#[tokio::test]
async fn investigator_item_uses_the_only_qualified_coordinator() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask-here"),
            normal("claude-1", "sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (id, tasks) = rig
        .run(
            &["inv-lead", "claude-1"],
            &format!("{ONE}\nTier: investigator\n"),
            MergeMode::Autopilot,
            None,
        )
        .await;
    let state = rig.status_is(&tasks[0], TaskStatus::Questions).await;
    assert!(matches!(state.team(&id).unwrap().items()[0].phase(),
        ItemPhase::InProgress {assignee, ..} if assignee.as_str() == "inv-lead"));
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn first_investigator_part_reassigns_the_normal_parent_to_the_coordinator() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "proposal:investigator,ask-here"),
            normal("claude-1", "sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    let dir = rig.home.root().join("fake-proposals");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("investigator.json"),
        serde_json::json!({
            "parent":"1.1", "scope":{"kind":"unchanged"}, "parts":[
                {"title":"First", "delivers":"First document", "checks":"Read first", "depends_on":[],
                 "tier":"investigator", "estimate":{"amount":100,"unit":"lines"}},
                {"title":"Second", "delivers":"Second document", "checks":"Read second", "depends_on":["a"],
                 "estimate":{"amount":100,"unit":"lines"}}
            ]
        })
        .to_string(),
    )
    .unwrap();
    rig.start().await;
    let (id, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;
    let state = rig.status_is(&tasks[0], TaskStatus::Questions).await;
    let team = state.team(&id).unwrap();
    assert_eq!(team.rounds().len(), 1);
    assert_eq!(team.rounds()[0].part().unwrap().to_string(), "a");
    assert!(matches!(team.items()[0].phase(), ItemPhase::InProgress {assignee, ..} if assignee.as_str() == "inv-lead"));
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn absent_investigator_waits_without_lower_assignment_and_survives_restart() {
    let mut rig = Rig::new(
        &[
            normal("lead", "sleep"),
            normal("claude-1", "sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (id, _) = rig
        .run(
            &["lead", "claude-1"],
            &format!("{ONE}\nTier: investigator\n"),
            MergeMode::Autopilot,
            Some("lead"),
        )
        .await;
    let item = "1.1".parse().unwrap();
    let state = rig
        .state_where("durable tier wait", |state| {
            state.team(&id).unwrap().tier_waits().get(&item) == Some(&Tier::Investigator)
        })
        .await;
    assert_eq!(state.team(&id).unwrap().items()[0].phase(), &ItemPhase::Pending);
    rig.stop().await.unwrap();
    rig.start().await;
    let state = rig
        .state_where("restored tier wait", |state| {
            state.team(&id).unwrap().tier_waits().get(&item) == Some(&Tier::Investigator)
        })
        .await;
    assert_eq!(state.team(&id).unwrap().items()[0].phase(), &ItemPhase::Pending);
    assert_eq!(state.team(&id).unwrap().rounds().len(), 0);
    rig.stop().await.unwrap();
}
