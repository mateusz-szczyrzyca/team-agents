//! Automatic coordinator proposal execution through fake CLIs and GitHub.

use super::rig::{ONE, Rig, investigator, normal};
use agent_core::state::{ItemPhase, MergeMode};
use agent_core::status::TaskStatus;

fn proposal(checks: &str) -> serde_json::Value {
    serde_json::json!({"parent":"1.1", "scope":{"kind":"unchanged"}, "parts":[
        {"title":"First", "delivers":"Independent first deliverable", "checks":checks,
         "depends_on":[], "estimate":{"amount":100,"unit":"lines"}},
        {"title":"Second", "delivers":"Independent second deliverable", "checks":"Read second",
         "depends_on":["a"], "estimate":{"amount":100,"unit":"lines"}}
    ]})
}

fn put(rig: &Rig, name: &str, value: &serde_json::Value) {
    let dir = rig.home.root().join("fake-proposals");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{name}.json")), value.to_string()).unwrap();
}

#[tokio::test]
async fn good_coordinator_proposal_records_whole_split_without_questions_and_opens_second_round() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "proposal:good,integrate"),
            normal("claude-1", "work,ask-here"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    put(&rig, "good", &proposal("Read first"));
    rig.start().await;
    let (id, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;
    let state = rig.status_is(&tasks[1], TaskStatus::Questions).await;
    let team = state.team(&id).unwrap();
    assert_eq!(team.splits().len(), 1);
    assert_eq!(team.rounds().len(), 2);
    assert_eq!(team.rounds()[1].part().unwrap().to_string(), "b");
    assert_eq!(team.rounds()[0].part_pull_request().unwrap().number.get().get(), 1);
    assert_eq!(team.crew().coordinator().unwrap().questions().len(), 0);
    assert!(!rig.home.questions_file(&tasks[0], std::num::NonZeroU32::MIN).exists());
    assert!(matches!(
        team.item("1.1".parse().unwrap()).unwrap().phase(),
        ItemPhase::InProgress { .. }
    ));
    rig.stop().await.unwrap();
}
