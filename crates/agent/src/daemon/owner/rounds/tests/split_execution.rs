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

#[tokio::test]
async fn human_merges_release_three_parts_across_restart_without_closing_on_approval() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate"),
            normal("claude-1", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team_id, tasks) = rig.run(&["inv-lead", "claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    rig.status_is(&tasks[1], TaskStatus::Waiting).await;
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: tasks[0].clone(),
                number: n(1),
                answers: answers(&[(1, &approval(3))]),
            }),
        )
        .await
        .unwrap();
    for number in 1..=3 {
        rig.wait_for("part approved or wrongly closed", || {
            let wire: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(rig.home.state_file()).unwrap()).unwrap();
            let team = &wire["teams"][0];
            team["rounds"].as_array().unwrap().len() == usize::try_from(number).unwrap()
                && (team["rounds"].as_array().unwrap().last().unwrap()["state"]["status"] == "closed"
                    || team["items"][0]["phase"]["phase"] == "approved")
        })
        .await;
        let wire: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(rig.home.state_file()).unwrap()).unwrap();
        assert_eq!(wire["teams"][0]["items"][0]["phase"]["phase"], "approved");
        let state = rig.state();
        let team = state.team(&team_id).unwrap();
        assert!(team.rounds().last().unwrap().is_open());
        assert_eq!(
            team.rounds().last().unwrap().part().unwrap().to_string(),
            match number {
                1 => "a",
                2 => "b",
                3 => "c",
                _ => panic!("the test opens exactly three parts"),
            }
        );
        if number == 1 {
            rig.stop().await.unwrap();
            rig.github.merged_by_a_human(number);
            rig.start().await;
        } else {
            rig.github.merged_by_a_human(number);
        }
    }
    let state = rig
        .item_where("1.1", "all three parts merged", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    let team = state.team(&team_id).unwrap();
    assert_eq!(team.rounds().len(), 3);
    assert_eq!(
        team.rounds()
            .iter()
            .filter(|round| round.part_pull_request().is_some())
            .count(),
        3
    );
    rig.status_is(&tasks[0], TaskStatus::Done).await;
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn split_answer_socket_reports_complete_activation_and_refusal_without_partial_parts() {
    for (verdict, expected, recorded) in [
        (
            approval(2),
            "Recorded split parts: 1.1a — Document 0; 1.1b — Document 1.",
            1,
        ),
        ("please split into two".into(), "use the explicit split verdict form", 0),
        ("split\n```json\n{}\n```".into(), "invalid split verdict json", 0),
        (
            approval(2).replace("\"depends_on\":[]", "\"depends_on\":[\"b\"]"),
            "invalid split plan",
            0,
        ),
    ] {
        let mut rig = Rig::new(
            &[
                investigator("inv-lead", "ask,sleep"),
                normal("claude-1", "sleep"),
                investigator("inv-r1", "sleep"),
                investigator("inv-r2", "sleep"),
            ],
            5,
            true,
        )
        .await;
        rig.start().await;
        let (team, tasks) = rig.run(&["inv-lead", "claude-1"], ONE, MergeMode::NoMerge, None).await;
        rig.status_is(&tasks[0], TaskStatus::Questions).await;
        let mut connection = Connection::connect(&rig.home.socket_file()).await.unwrap();
        let reply = connection
            .request(
                Client::Cli,
                Command::Answer(AnswerRequest {
                    task: tasks[0].clone(),
                    number: n(1),
                    answers: answers(&[(1, &verdict)]),
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            reply,
            agent_core::protocol::Reply::Applied {
                tasks: vec![tasks[0].clone()]
            }
        );
        assert_eq!(rig.state().team(&team).unwrap().splits().len(), recorded);
        let acknowledgement = connection.split_acknowledgement().unwrap();
        assert!(acknowledgement.contains(expected), "{acknowledgement}");
        rig.stop().await.unwrap();
    }
}

#[tokio::test]
async fn split_fix_updates_its_pull_request() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate,fix-work:1"),
            normal("claude-1", "work"),
            investigator("inv-r1", "structured-commit:REQUEST_CHANGES,structured-commit:APPROVE"),
            investigator("inv-r2", "structured-commit:APPROVE"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team_id, tasks) = rig.run(&["inv-lead", "claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    Connection::connect(&rig.home.socket_file())
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
    let state = rig
        .state_where("the fixed part approved or handed over", |state| {
            let team = state.team(&team_id).unwrap();
            matches!(
                team.item("1.1".parse().unwrap()).unwrap().phase(),
                ItemPhase::Approved { .. }
            ) || state.find_task(&tasks[0]).unwrap().member.status() == TaskStatus::Questions
        })
        .await;
    assert!(
        matches!(
            state
                .team(&team_id)
                .unwrap()
                .item("1.1".parse().unwrap())
                .unwrap()
                .phase(),
            ItemPhase::Approved { .. }
        ),
        "{:?}",
        state
            .team(&team_id)
            .unwrap()
            .item("1.1".parse().unwrap())
            .unwrap()
            .phase()
    );
    let pulls = rig.github.pulls();
    assert_eq!(pulls.len(), 1);
    let gate: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(rig.home.gate_record_file(&tasks[0])).unwrap()).unwrap();
    assert_eq!(rig.remote_tip(&pulls[0].head), gate["result"]["head"].as_str().unwrap());
    rig.stop().await.unwrap();
}
