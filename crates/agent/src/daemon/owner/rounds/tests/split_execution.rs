//! Approved parts keep their parent open and get separate daemon rounds and pull requests.

use super::rig::{ONE, Rig, investigator, normal};
use crate::client::Connection;
use crate::daemon::owner::fixture::{answers, n};
use agent_core::protocol::{AnswerRequest, Client, Command};
use agent_core::state::{ItemPhase, MergeMode};
use agent_core::status::TaskStatus;

fn approval(count: usize) -> String {
    let parts = (0..count)
        .map(|index| {
            serde_json::json!({
                "title":format!("Document {index}"), "delivers":format!("Independent document {index}"),
                "checks":format!("Read document {index}"), "depends_on":if index == 0 { vec![] } else { vec!["a"] }
            })
        })
        .collect::<Vec<_>>();
    let plan = serde_json::json!({"parent":"1.1", "parts":parts});
    format!("split\n```json\n{plan}\n```")
}

#[tokio::test]
async fn approved_parts_merge_in_distinct_rounds_and_branches_and_close_only_the_last() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate"),
            normal("claude-1", "work,ask-here,work"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team_id, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    rig.status_is(&tasks[1], TaskStatus::Waiting).await;
    let reply = Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: tasks[0].clone(),
                number: n(1),
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();
    let state = rig.state();
    assert_eq!(state.team(&team_id).unwrap().splits().len(), 1, "{reply:?}");
    let state = rig.status_is(&tasks[1], TaskStatus::Questions).await;
    let team = state.team(&team_id).unwrap();
    assert_eq!(team.rounds().len(), 2);
    assert_eq!(team.rounds()[0].part_pull_request().unwrap().number.get().get(), 1);
    assert_eq!(team.rounds()[1].part().unwrap().to_string(), "b");
    assert!(
        matches!(team.item("1.1".parse().unwrap()).unwrap().phase(), ItemPhase::InProgress { branch, .. } if branch.as_str().ends_with("-1.1b"))
    );
    let prompt = rig.prompt(&tasks[1], 2).await;
    assert!(prompt.contains("Independent document 1"), "{prompt}");
    assert!(prompt.contains("Read document 1"), "{prompt}");
    assert!(prompt.contains("/pull/1"), "{prompt}");
    rig.stop().await.unwrap();
    rig.start().await;
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: tasks[1].clone(),
                number: n(1),
                answers: answers(&[(1, "ok")]),
            }),
        )
        .await
        .unwrap();
    let state = rig
        .state_where("the last split part merged", |state| {
            matches!(
                state
                    .team(&team_id)
                    .unwrap()
                    .item("1.1".parse().unwrap())
                    .unwrap()
                    .phase(),
                ItemPhase::Done { .. }
            )
        })
        .await;
    let team = state.team(&team_id).unwrap();
    assert_eq!(team.rounds().len(), 2);
    let first = team.rounds()[0].part_pull_request().unwrap();
    let second = team.rounds()[1].part_pull_request().unwrap();
    assert_ne!(first.number, second.number);
    assert_ne!(first.branch, second.branch);
    rig.status_is(&tasks[0], TaskStatus::Done).await;
    rig.stop().await.unwrap();
}
