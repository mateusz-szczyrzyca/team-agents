//! Approved parts keep their parent open and get separate daemon rounds and pull requests.

use super::rig::{ONE, Rig, investigator, normal};
use crate::client::Connection;
use crate::daemon::owner::fixture::{answers, n};
use agent_core::ids::{TaskId, TeamId};
use agent_core::protocol::{AnswerRequest, Client, Command, ControlRequest, Target};
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

#[tokio::test]
async fn split_answer_and_status_negotiate_version_four() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate"),
            normal("claude-1", "ask-here"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["inv-lead", "claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    let mut connection = Connection::connect(&rig.home.socket_file()).await.unwrap();
    let answer = connection
        .request_split(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: tasks[0].clone(),
                number: n(1),
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();
    assert_eq!(
        answer,
        agent_core::protocol::Reply::Applied {
            tasks: vec![tasks[0].clone()]
        }
    );
    assert!(matches!(connection.split_acknowledgement(), Some(text) if text.contains("1.1a") && text.contains("1.1b")));
    let status = connection
        .request_split(
            Client::Cli,
            Command::Status(agent_core::protocol::StatusRequest { target: None }),
        )
        .await
        .unwrap();
    let agent_core::protocol::Reply::Teams { teams } = status else {
        panic!("expected team summaries");
    };
    assert_eq!(teams[0].splits.len(), 1);
    assert_eq!(teams[0].splits[0].parts().len(), 2);
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn split_pre_review_correction_reuses_its_draft_pull_request() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate,script:restore-test"),
            normal("claude-1", "script:edit-test"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        format!("{config}[repos.example.checks]\ntest_paths = [\"tests/\"]\ncode_paths = [\"src/\"]\nprotected_paths = [\"ci/\"]\n"),
    ).unwrap();
    std::fs::create_dir_all(rig.repo.join("tests")).unwrap();
    std::fs::write(rig.repo.join("tests/existing.txt"), "a\nb\nc\n").unwrap();
    super::rig::git(&rig.repo, &["add", "tests/existing.txt"]);
    super::rig::git(&rig.repo, &["commit", "-q", "-m", "Seed the existing test"]);
    super::rig::git(&rig.repo, &["push", "-q", "origin", "main"]);
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    for (name, command) in [
        ("edit-test", "sed -i 2d tests/existing.txt"),
        ("restore-test", "git checkout main -- tests/existing.txt"),
    ] {
        std::fs::write(scripts.join(format!("{name}.sh")), format!(
            "set -eu\n{command}\ngit add tests/existing.txt\ngit -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q -m '{name}'\n"
        )).unwrap();
    }
    rig.start().await;
    let (_team, tasks) = rig.run(&["inv-lead", "claude-1"], ONE, MergeMode::NoMerge, None).await;
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
    rig.item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    assert_eq!(rig.runs(&tasks[0]), 3);
    let pulls = rig.github.pulls();
    assert_eq!(pulls.len(), 1);
    assert_eq!(pulls[0].head, format!("agents/inv-lead/{}-1.1a", tasks[0]));
    let gate: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(rig.home.gate_record_file(&tasks[0])).unwrap()).unwrap();
    assert_eq!(rig.remote_tip(&pulls[0].head), gate["result"]["head"].as_str().unwrap());
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn split_part_delivery_with_unmerged_pr_is_refused_and_round_stays_open() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,done:DELIVERED: #1,sleep"),
            normal("claude-1", "sleep"),
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
    let part_a_branch = format!("agents/claude-1/{}-1.1a", tasks[1]);
    super::rig::git(&rig.origin, &["branch", &part_a_branch, "main"]);
    let pr1 = rig.github.foreign_pull(&part_a_branch);
    assert_eq!(pr1, 1);
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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();
    let state = rig
        .state_where("lead refusal reaches task 0", |state| {
            state.team(&team_id).unwrap().find(&tasks[0]).unwrap().1.runs().len() >= 3
        })
        .await;
    let prompt = rig.prompt(&tasks[0], 3).await;
    assert!(prompt.contains("the delivering pull request is not merged"), "{prompt}");
    let team = state.team(&team_id).unwrap();
    assert_eq!(team.rounds().len(), 1);
    let round1 = &team.rounds()[0];
    assert!(round1.is_open());
    assert_eq!(round1.delivered_by(), None);
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn split_part_delivered_by_merged_pr_outside_round_completes_item_and_team() {
    let mut rig = Rig::new(
        &[
            investigator(
                "inv-lead",
                "ask,clean-done:DELIVERED: #1,clean-done:DELIVERED: #2,sleep",
            ),
            normal("claude-1", "clean-done:already delivered,clean-done:already delivered"),
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
    let branch_part_a = format!("agents/claude-1/{}-1.1a", tasks[1]);
    let branch_part_b = format!("agents/claude-1/{}-1.1b", tasks[1]);
    super::rig::git(&rig.origin, &["branch", &branch_part_a, "main"]);
    super::rig::git(&rig.origin, &["branch", &branch_part_b, "main"]);
    let pr1 = rig.github.foreign_pull(&branch_part_a);
    let pr2 = rig.github.foreign_pull(&branch_part_b);
    assert_eq!(pr1, 1);
    assert_eq!(pr2, 2);
    rig.github.merged_by_a_human(1);
    rig.github.merged_by_a_human(2);
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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();
    rig.state_where("round 1 closed", |state| {
        let team = state.team(&team_id).unwrap();
        team.rounds().first().is_some_and(|r| !r.is_open())
    })
    .await;
    let state = rig
        .state_where("team done after last part", |state| {
            let team = state.team(&team_id).unwrap();
            matches!(
                team.item("1.1".parse().unwrap()).unwrap().phase(),
                ItemPhase::Done { .. }
            )
        })
        .await;
    let team = state.team(&team_id).unwrap();
    assert_eq!(team.rounds().len(), 2);
    assert_eq!(team.rounds()[0].delivered_by().unwrap().number.get().get(), 1);
    assert_eq!(team.rounds()[0].part().unwrap().to_string(), "a");
    assert_eq!(team.rounds()[1].delivered_by().unwrap().number.get().get(), 2);
    assert_eq!(team.rounds()[1].part().unwrap().to_string(), "b");
    assert!(team.rounds().iter().all(|round| !round.is_open()));
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn earlier_split_part_pr_is_refused_for_next_part_as_foreign_branch() {
    let mut rig = Rig::new(
        &[
            investigator(
                "inv-lead",
                "ask,clean-done:DELIVERED: #1,clean-done:DELIVERED: #1,sleep",
            ),
            normal(
                "claude-1",
                "clean-done:already delivered,clean-done:already delivered,sleep",
            ),
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
    let branch_part_a = format!("agents/claude-1/{}-1.1a", tasks[1]);
    super::rig::git(&rig.origin, &["branch", &branch_part_a, "main"]);
    let pr1 = rig.github.foreign_pull(&branch_part_a);
    assert_eq!(pr1, 1);
    rig.github.merged_by_a_human(1);

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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();
    // Round 1 delivers via PR 1
    rig.state_where("round 1 closed", |state| {
        let team = state.team(&team_id).unwrap();
        team.rounds().first().is_some_and(|r| !r.is_open())
    })
    .await;
    // Round 2 opens for Part 1.1b. Lead tries to deliver part b with PR 1 (which belongs to part a)
    // -> refused as foreign branch!
    rig.state_where("lead refusal reaches task 0", |state| {
        state.team(&team_id).unwrap().find(&tasks[0]).unwrap().1.runs().len() >= 4
    })
    .await;
    let prompt = rig.prompt(&tasks[0], 4).await;
    assert!(
        prompt.contains("the delivering pull request does not use the task's or part's branch"),
        "{prompt}"
    );
    let state = rig.state();
    let team = state.team(&team_id).unwrap();
    assert_eq!(team.rounds().len(), 2);
    let round2 = &team.rounds()[1];
    assert!(round2.is_open());
    assert_eq!(round2.delivered_by(), None);
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn split_part_delivered_by_merged_pr_on_task_branch_is_accepted_for_split_part() {
    let mut rig = Rig::new(
        &[
            investigator(
                "inv-lead",
                "ask,clean-done:DELIVERED: #1,clean-done:DELIVERED: #2,sleep",
            ),
            normal("claude-1", "clean-done:already delivered,clean-done:already delivered"),
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
    let branch_task = format!("agents/claude-1/{}-1.1", tasks[1]);
    let branch_part_b = format!("agents/claude-1/{}-1.1b", tasks[1]);
    super::rig::git(&rig.origin, &["branch", &branch_task, "main"]);
    super::rig::git(&rig.origin, &["branch", &branch_part_b, "main"]);
    let pr1 = rig.github.foreign_pull(&branch_task);
    let pr2 = rig.github.foreign_pull(&branch_part_b);
    assert_eq!(pr1, 1);
    assert_eq!(pr2, 2);
    rig.github.merged_by_a_human(1);
    rig.github.merged_by_a_human(2);
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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();
    rig.state_where("round 1 closed", |state| {
        let team = state.team(&team_id).unwrap();
        team.rounds().first().is_some_and(|r| !r.is_open())
    })
    .await;
    let state = rig
        .state_where("team done after last part", |state| {
            let team = state.team(&team_id).unwrap();
            matches!(
                team.item("1.1".parse().unwrap()).unwrap().phase(),
                ItemPhase::Done { .. }
            )
        })
        .await;
    let team = state.team(&team_id).unwrap();
    assert_eq!(team.rounds().len(), 2);
    assert_eq!(team.rounds()[0].delivered_by().unwrap().number.get().get(), 1);
    assert_eq!(team.rounds()[0].part().unwrap().to_string(), "a");
    assert_eq!(team.rounds()[1].delivered_by().unwrap().number.get().get(), 2);
    assert_eq!(team.rounds()[1].part().unwrap().to_string(), "b");
    assert!(team.rounds().iter().all(|round| !round.is_open()));
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn split_part_delivery_with_closed_pr_is_refused_and_tells_lead() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,clean-done:DELIVERED: #1,sleep"),
            normal("claude-1", "clean-done:already delivered,sleep"),
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
    let branch_part_a = format!("agents/claude-1/{}-1.1a", tasks[1]);
    super::rig::git(&rig.origin, &["branch", &branch_part_a, "main"]);
    let pr1 = rig.github.foreign_pull(&branch_part_a);
    assert_eq!(pr1, 1);
    rig.github.closed_by_a_human(1);

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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();

    rig.state_where("lead refusal reaches task 0", |state| {
        state.team(&team_id).unwrap().find(&tasks[0]).unwrap().1.runs().len() >= 3
    })
    .await;
    let prompt = rig.prompt(&tasks[0], 3).await;
    assert!(
        prompt.contains("the delivering pull request was closed without merging"),
        "{prompt}"
    );
    let state = rig.state();
    let team = state.team(&team_id).unwrap();
    assert_eq!(team.rounds().len(), 1);
    assert!(team.rounds()[0].is_open());
    assert_eq!(team.rounds()[0].delivered_by(), None);
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn three_part_split_stopped_member_before_b_merges_opens_c_for_lead() {
    let mut rig = Rig::new(
        &[
            investigator(
                "inv-lead",
                "ask,clean-done:DELIVERED: #1,clean-done:DELIVERED: #2,clean-done:DELIVERED: #3,sleep",
            ),
            normal("claude-1", "clean-done:already delivered,sleep"),
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

    let branch_part_a = format!("agents/claude-1/{}-1.1a", tasks[1]);
    let branch_part_b = format!("agents/claude-1/{}-1.1b", tasks[1]);
    let branch_part_c = format!("agents/inv-lead/{}-1.1c", tasks[0]);
    super::rig::git(&rig.origin, &["branch", &branch_part_a, "main"]);
    super::rig::git(&rig.origin, &["branch", &branch_part_b, "main"]);
    super::rig::git(&rig.origin, &["branch", &branch_part_c, "main"]);
    let pr1 = rig.github.foreign_pull(&branch_part_a);
    let pr2 = rig.github.foreign_pull(&branch_part_b);
    let pr3 = rig.github.foreign_pull(&branch_part_c);
    assert_eq!(pr1, 1);
    assert_eq!(pr2, 2);
    assert_eq!(pr3, 3);
    rig.github.merged_by_a_human(1);
    rig.github.merged_by_a_human(2);
    rig.github.merged_by_a_human(3);

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

    // Round 1 closed via PR 1:
    rig.state_where("round 1 closed", |state| {
        let team = state.team(&team_id).unwrap();
        team.rounds().first().is_some_and(|r| !r.is_open())
    })
    .await;

    // Round 2 opened for part b:
    rig.state_where("round 2 open for part b", |state| {
        let team = state.team(&team_id).unwrap();
        team.rounds().len() == 2 && team.rounds()[1].is_open()
    })
    .await;

    // Stop claude-1 before part b merges:
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Task(tasks[1].clone()),
            }),
        )
        .await
        .unwrap();
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;

    // Round 2 closes via PR 2, and round 3 opens for inv-lead on part c:
    rig.state_where("round 3 open for inv-lead on part c", |state| {
        let team = state.team(&team_id).unwrap();
        team.rounds().len() >= 3
            && team.rounds()[2].is_open()
            && team.rounds()[2].part().map(|p| p.to_string()) == Some("c".to_string())
            && match team.item("1.1".parse().unwrap()).unwrap().phase() {
                ItemPhase::InProgress { assignee, .. } => assignee.as_str() == "inv-lead",
                _ => false,
            }
    })
    .await;

    // Team finishes after part c delivered by lead:
    let state = rig
        .state_where("team done after part c delivered", |state| {
            let team = state.team(&team_id).unwrap();
            matches!(
                team.item("1.1".parse().unwrap()).unwrap().phase(),
                ItemPhase::Done { .. }
            )
        })
        .await;

    let team = state.team(&team_id).unwrap();
    assert_eq!(team.rounds().len(), 3);
    assert_eq!(team.rounds()[0].part().unwrap().to_string(), "a");
    assert_eq!(team.rounds()[1].part().unwrap().to_string(), "b");
    assert_eq!(team.rounds()[2].part().unwrap().to_string(), "c");
    assert_eq!(team.rounds()[2].delivered_by().unwrap().number.get().get(), 3);
    assert!(team.rounds().iter().all(|round| !round.is_open()));
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn split_part_with_fitting_replacement_agent_takes_over_when_member_stops() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate"),
            normal("claude-1", "work,sleep"),
            normal("claude-2", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    let mut cfg = std::fs::read_to_string(rig.home.config_file()).unwrap();
    cfg.push_str("\n[replacement]\nagents = [\"claude-2\"]\n");
    std::fs::write(rig.home.config_file(), cfg).unwrap();
    rig.start().await;
    let (team_id, tasks) = rig
        .run(&["inv-lead", "claude-1", "claude-2"], ONE, MergeMode::NoMerge, None)
        .await;
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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();

    // Part A approves:
    rig.wait_for("part a approved", || {
        let wire: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(rig.home.state_file()).unwrap()).unwrap();
        wire["teams"][0]["items"][0]["phase"]["phase"] == "approved"
    })
    .await;

    // Stop claude-1 before part A merges:
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Task(tasks[1].clone()),
            }),
        )
        .await
        .unwrap();
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;

    // Part A merges:
    rig.github.merged_by_a_human(1);

    // Part B opens for claude-2:
    rig.wait_for("part b opened for replacement member claude-2", || {
        let state = rig.state();
        let team = state.team(&team_id).unwrap();
        team.rounds().len() == 2
            && team.rounds()[1].is_open()
            && match team.item("1.1".parse().unwrap()).unwrap().phase() {
                ItemPhase::InProgress { assignee, .. } => assignee.as_str() == "claude-2",
                _ => false,
            }
    })
    .await;
    let state = rig.state();
    let team = state.team(&team_id).unwrap();
    assert_eq!(team.rounds()[1].part().unwrap().to_string(), "b");
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn split_part_nobody_qualified_writes_one_question_and_no_repeated_lead_wakeups() {
    let parts = vec![
        serde_json::json!({
            "title": "Document 0",
            "delivers": "Independent document 0",
            "checks": "Read document 0",
            "depends_on": []
        }),
        serde_json::json!({
            "title": "Document 1",
            "delivers": "Independent document 1",
            "checks": "Read document 1",
            "depends_on": ["a"],
            "tier": "investigator"
        }),
    ];
    let split_plan = format!(
        "split\n```json\n{}\n```",
        serde_json::json!({"parent": "1.1", "parts": parts})
    );

    let mut rig = Rig::new(
        &[
            normal("lead", "ask,integrate,sleep"),
            normal("claude-1", "work,sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (team_id, tasks) = rig
        .run(&["lead", "claude-1"], ONE, MergeMode::NoMerge, Some("lead"))
        .await;
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
                answers: answers(&[(1, &split_plan)]),
            }),
        )
        .await
        .unwrap();

    // Part A approves:
    rig.wait_for("part a approved", || {
        let wire: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(rig.home.state_file()).unwrap()).unwrap();
        wire["teams"][0]["items"][0]["phase"]["phase"] == "approved"
    })
    .await;

    // Part A merges:
    rig.github.merged_by_a_human(1);

    // Part B requires investigator tier, but nobody in crew is investigator. A tier question is created for the coordinator:
    rig.wait_for("tier question for part b", || {
        let state = rig.state();
        let team = state.team(&team_id).unwrap();
        team.find(&tasks[0]).unwrap().1.status() == TaskStatus::Questions
            && team.tier_waits().contains_key(&"1.1".parse().unwrap())
    })
    .await;

    // Verify question content names part 1.1b:
    let question_path = rig.home.questions_file(&tasks[0], n(2));
    assert!(question_path.is_file());
    let content = std::fs::read_to_string(&question_path).unwrap();
    assert!(content.contains("1.1b"), "{content}");

    // The coordinator asked the question once; tier_questions records it deterministically:
    let team = rig.state().team(&team_id).unwrap().clone();
    assert!(team.tier_waits().contains_key(&"1.1".parse().unwrap()));
    assert_eq!(team.find(&tasks[0]).unwrap().1.status(), TaskStatus::Questions);
    assert!(!rig.home.questions_file(&tasks[0], n(3)).exists());

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn authentic_pre_task_persisted_fixture_loads_and_daemon_resumes() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "clean-done:DELIVERED: #3,clean-done:DELIVERED: #3,sleep"),
            normal("claude-1", "sleep"),
        ],
        5,
        false,
    )
    .await;
    let fixture = include_str!("../../../../../../agent-core/tests/fixtures/state-before-split-delivery.json");
    std::fs::write(rig.home.state_file(), fixture).unwrap();

    let team_id: TeamId = "team-20261121-120000-spl1".parse().unwrap();
    let lead_task: TaskId = "task-20261121-120001-spl1".parse().unwrap();
    let member_task: TaskId = "task-20261121-120002-spl1".parse().unwrap();

    std::fs::create_dir_all(rig.home.task_dir(&lead_task)).unwrap();
    std::fs::create_dir_all(rig.home.task_dir(&member_task)).unwrap();
    let lead_branch = format!("agents/inv-lead/{lead_task}-1.1");
    let branch_c = format!("agents/inv-lead/{lead_task}-1.1c");
    let member_branch = format!("agents/claude-1/{member_task}-1.1");
    super::rig::git(&rig.repo, &["branch", &lead_branch, "main"]);
    super::rig::git(&rig.origin, &["branch", &branch_c, "main"]);
    super::rig::git(&rig.repo, &["branch", &member_branch, "main"]);
    super::rig::git(
        &rig.repo,
        &[
            "worktree",
            "add",
            rig.worktree(&lead_task).to_str().unwrap(),
            &lead_branch,
        ],
    );
    super::rig::git(
        &rig.repo,
        &[
            "worktree",
            "add",
            rig.worktree(&member_task).to_str().unwrap(),
            &member_branch,
        ],
    );

    rig.github
        .foreign_pull("agents/claude-1/task-20261121-120002-spl1-1.1a");
    rig.github
        .foreign_pull("agents/claude-1/task-20261121-120002-spl1-1.1b");
    let pr3 = rig.github.foreign_pull(&branch_c);
    assert_eq!(pr3, 3);
    rig.github.merged_by_a_human(3);

    rig.start().await;

    // Part c round opens for inv-lead:
    rig.state_where("round 3 opens for inv-lead on part c", |state| {
        let team = state.team(&team_id).unwrap();
        team.rounds().len() == 3
            && team.rounds()[2].is_open()
            && team.rounds()[2].part().map(|p| p.to_string()) == Some("c".to_string())
    })
    .await;

    // Restart daemon between opening and delivery:
    rig.stop().await.unwrap();
    rig.start().await;

    // Team reaches Done after part c delivered:
    let state = rig
        .state_where("team done after part c delivered", |state| {
            let team = state.team(&team_id).unwrap();
            matches!(
                team.item("1.1".parse().unwrap()).unwrap().phase(),
                ItemPhase::Done { .. }
            )
        })
        .await;

    let team = state.team(&team_id).unwrap();
    assert_eq!(team.rounds().len(), 3);
    assert_eq!(team.rounds()[2].delivered_by().unwrap().number.get().get(), 3);
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn three_part_split_staged_stopped_member_before_b_merges_opens_c_for_lead() {
    let mut rig = Rig::new(
        &[
            investigator(
                "inv-lead",
                "ask,clean-done:DELIVERED: #1,clean-done:DELIVERED: #2,clean-done:DELIVERED: #3,sleep",
            ),
            normal("claude-1", "clean-done:already delivered,sleep"),
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

    let branch_part_a = format!("agents/claude-1/{}-1.1a", tasks[1]);
    let branch_part_b = format!("agents/claude-1/{}-1.1b", tasks[1]);
    let branch_part_c = format!("agents/inv-lead/{}-1.1c", tasks[0]);
    super::rig::git(&rig.origin, &["branch", &branch_part_a, "main"]);
    super::rig::git(&rig.origin, &["branch", &branch_part_b, "main"]);
    super::rig::git(&rig.origin, &["branch", &branch_part_c, "main"]);

    let pr1 = rig.github.foreign_pull(&branch_part_a);
    let pr2 = rig.github.foreign_pull(&branch_part_b);
    let pr3 = rig.github.foreign_pull(&branch_part_c);
    assert_eq!(pr1, 1);
    assert_eq!(pr2, 2);
    assert_eq!(pr3, 3);
    rig.github.merged_by_a_human(1);

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

    // Round 1 closed via PR 1:
    rig.state_where("round 1 closed", |state| {
        let team = state.team(&team_id).unwrap();
        team.rounds().first().is_some_and(|r| !r.is_open())
    })
    .await;

    // Round 2 opened for part b:
    rig.state_where("round 2 open for part b", |state| {
        let team = state.team(&team_id).unwrap();
        team.rounds().len() == 2 && team.rounds()[1].is_open()
    })
    .await;

    // Stop claude-1 BEFORE part b merges:
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Task(tasks[1].clone()),
            }),
        )
        .await
        .unwrap();
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;

    // NOW merge PR 2 by a human while member is stopped:
    rig.github.merged_by_a_human(2);
    rig.github.merged_by_a_human(3);

    // Round 2 closes via PR 2, and round 3 opens for inv-lead on part c:
    rig.state_where("round 3 open for inv-lead on part c", |state| {
        let team = state.team(&team_id).unwrap();
        team.rounds().len() >= 3
            && team.rounds()[2].is_open()
            && team.rounds()[2].part().map(|p| p.to_string()) == Some("c".to_string())
            && match team.item("1.1".parse().unwrap()).unwrap().phase() {
                ItemPhase::InProgress { assignee, .. } => assignee.as_str() == "inv-lead",
                _ => false,
            }
    })
    .await;

    // Team finishes after part c delivered by lead:
    let state = rig
        .state_where("team done after part c delivered", |state| {
            let team = state.team(&team_id).unwrap();
            matches!(
                team.item("1.1".parse().unwrap()).unwrap().phase(),
                ItemPhase::Done { .. }
            )
        })
        .await;

    let team = state.team(&team_id).unwrap();
    assert_eq!(team.rounds().len(), 3);
    assert_eq!(team.rounds()[0].part().unwrap().to_string(), "a");
    assert_eq!(team.rounds()[1].part().unwrap().to_string(), "b");
    assert_eq!(team.rounds()[2].part().unwrap().to_string(), "c");
    assert_eq!(team.rounds()[2].delivered_by().unwrap().number.get().get(), 3);
    assert!(team.rounds().iter().all(|round| !round.is_open()));
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn split_part_delivery_with_foreign_repo_is_refused_and_tells_lead() {
    let mut rig = Rig::new(
        &[
            investigator(
                "inv-lead",
                "ask,clean-done:DELIVERED: #1,clean-done:DELIVERED: #99,sleep",
            ),
            normal("claude-1", "clean-done:already delivered,sleep"),
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

    let branch_part_a = format!("agents/claude-1/{}-1.1a", tasks[1]);
    super::rig::git(&rig.origin, &["branch", &branch_part_a, "main"]);
    let pr1 = rig.github.foreign_pull(&branch_part_a);
    assert_eq!(pr1, 1);
    rig.github.merged_by_a_human(1);

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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();

    // Round 1 closed via PR 1:
    rig.state_where("round 1 closed", |state| {
        let team = state.team(&team_id).unwrap();
        team.rounds().first().is_some_and(|r| !r.is_open())
    })
    .await;

    // Round 2 opens for part b:
    rig.state_where("round 2 open for part b", |state| {
        let team = state.team(&team_id).unwrap();
        team.rounds().len() == 2 && team.rounds()[1].is_open()
    })
    .await;

    // Stop claude-1 so part b opens for lead:
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Task(tasks[1].clone()),
            }),
        )
        .await
        .unwrap();
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;

    // Lead tries to deliver unrecorded foreign PR #99:
    rig.state_where("lead round remains open after foreign pr refusal", |state| {
        let team = state.team(&team_id).unwrap();
        team.rounds().len() == 2 && team.rounds()[1].is_open()
    })
    .await;

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn split_part_assignment_logs_who_gets_part_decision() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,clean-done:DELIVERED: #1,sleep"),
            normal("claude-1", "clean-done:already delivered,sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    let mut cfg = std::fs::read_to_string(rig.home.config_file()).unwrap();
    cfg.push_str("\n[log]\nlevel = \"debug\"\n");
    std::fs::write(rig.home.config_file(), cfg).unwrap();

    rig.start().await;
    let (team_id, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;

    let branch_part_a = format!("agents/claude-1/{}-1.1a", tasks[1]);
    super::rig::git(&rig.origin, &["branch", &branch_part_a, "main"]);
    let pr1 = rig.github.foreign_pull(&branch_part_a);
    assert_eq!(pr1, 1);
    rig.github.merged_by_a_human(1);

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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();

    rig.state_where("part b open", |state| {
        let team = state.team(&team_id).unwrap();
        team.rounds().len() == 2 && team.rounds()[1].is_open()
    })
    .await;

    // History log contains who_gets_part decision:
    rig.stop().await.unwrap();
    let reader = agent_runtime::history::Reader::open(&rig.home.history_file()).unwrap();
    let logs = reader.daemon_logs().unwrap();
    let row = logs.iter().find(|row| {
        row.event() == agent_core::log::LogEvent::DecisionTaken
            && row.fields().get("decision").and_then(|v| v.as_str()) == Some("who_gets_part")
    });
    assert!(row.is_some(), "who_gets_part decision must be logged");
    let result = &row.unwrap().fields()["result"];
    assert!(result.get("choice").is_some(), "result must contain choice: {result:?}");
    assert!(result.get("reason").is_some(), "result must contain reason: {result:?}");
}

#[tokio::test]
async fn split_part_with_failed_prior_member_opens_for_lead() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate"),
            normal("claude-1", "work,sleep"),
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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();

    // Part A approves:
    rig.wait_for("part a approved", || {
        let wire: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(rig.home.state_file()).unwrap()).unwrap();
        wire["teams"][0]["items"][0]["phase"]["phase"] == "approved"
    })
    .await;

    // Fail claude-1:
    rig.stop().await.unwrap();
    {
        let mut state = rig.state();
        let mut team = state.team_mut(&team_id).unwrap();
        let mut member = team.find_mut(&tasks[1]).unwrap().1;
        member.transition(TaskStatus::Queued).unwrap();
        member.transition(TaskStatus::Running).unwrap();
        member.transition(TaskStatus::Failed).unwrap();
        std::fs::write(rig.home.state_file(), serde_json::to_string_pretty(&state).unwrap()).unwrap();
    }
    rig.start().await;

    // Part A merges:
    rig.github.merged_by_a_human(1);

    // Part B opens for inv-lead:
    rig.wait_for("part b opened for lead", || {
        let state = rig.state();
        let team = state.team(&team_id).unwrap();
        team.rounds().len() == 2
            && team.rounds()[1].is_open()
            && match team.item("1.1".parse().unwrap()).unwrap().phase() {
                ItemPhase::InProgress { assignee, .. } => assignee.as_str() == "inv-lead",
                _ => false,
            }
    })
    .await;
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn split_part_with_handed_over_prior_member_opens_for_lead() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate"),
            normal("claude-1", "work,sleep"),
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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();

    // Part A approves:
    rig.wait_for("part a approved", || {
        let wire: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(rig.home.state_file()).unwrap()).unwrap();
        wire["teams"][0]["items"][0]["phase"]["phase"] == "approved"
    })
    .await;

    // Persist the stopped member and its outstanding handover to the lead:
    rig.stop().await.unwrap();
    {
        let mut state = rig.state();
        state
            .find_task_mut(&tasks[1])
            .unwrap()
            .1
            .transition(TaskStatus::Stopped)
            .unwrap();
        state
            .owe_lead(
                &tasks[1],
                agent_core::decide::ladder::Trouble::NoStatus,
                "2026-12-01T00:00:00Z".parse().unwrap(),
            )
            .unwrap();
        std::fs::write(rig.home.state_file(), serde_json::to_string_pretty(&state).unwrap()).unwrap();
    }
    rig.start().await;

    // Part A merges:
    rig.github.merged_by_a_human(1);

    // Part B opens for inv-lead:
    rig.wait_for("part b opened for lead", || {
        let state = rig.state();
        let team = state.team(&team_id).unwrap();
        team.rounds().len() == 2
            && team.rounds()[1].is_open()
            && match team.item("1.1".parse().unwrap()).unwrap().phase() {
                ItemPhase::InProgress { assignee, .. } => assignee.as_str() == "inv-lead",
                _ => false,
            }
    })
    .await;
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn split_part_configured_off_mode_writes_question_for_human() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate"),
            normal("claude-1", "work,sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    let mut cfg = std::fs::read_to_string(rig.home.config_file()).unwrap();
    cfg.push_str("\n[replacement]\nmode = \"off\"\n");
    std::fs::write(rig.home.config_file(), cfg).unwrap();

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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();

    // Part A approves:
    rig.wait_for("part a approved", || {
        let wire: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(rig.home.state_file()).unwrap()).unwrap();
        wire["teams"][0]["items"][0]["phase"]["phase"] == "approved"
    })
    .await;

    // Stop claude-1 before part A merges:
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Task(tasks[1].clone()),
            }),
        )
        .await
        .unwrap();
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;

    // Part A merges:
    rig.github.merged_by_a_human(1);

    // Under mode = "off", stopped member creates a question for the human:
    rig.wait_for("question written for human under off mode", || {
        let state = rig.state();
        let team = state.team(&team_id).unwrap();
        team.find(&tasks[0]).unwrap().1.status() == TaskStatus::Questions
            && team.tier_waits().contains_key(&"1.1".parse().unwrap())
    })
    .await;
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn split_part_configured_lead_mode_opens_for_lead() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate"),
            normal("claude-1", "work,sleep"),
            normal("claude-2", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    let mut cfg = std::fs::read_to_string(rig.home.config_file()).unwrap();
    cfg.push_str("\n[replacement]\nmode = \"lead\"\nagents = [\"claude-2\"]\n");
    std::fs::write(rig.home.config_file(), cfg).unwrap();

    rig.start().await;
    let (team_id, tasks) = rig
        .run(&["inv-lead", "claude-1", "claude-2"], ONE, MergeMode::NoMerge, None)
        .await;
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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();

    // Part A approves:
    rig.wait_for("part a approved", || {
        let wire: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(rig.home.state_file()).unwrap()).unwrap();
        wire["teams"][0]["items"][0]["phase"]["phase"] == "approved"
    })
    .await;

    // Stop claude-1 before part A merges:
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Task(tasks[1].clone()),
            }),
        )
        .await
        .unwrap();
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;

    // Part A merges:
    rig.github.merged_by_a_human(1);

    // In mode = "lead", part B opens for lead even though replacement claude-2 is available and configured:
    rig.wait_for("part b opened for lead under lead mode", || {
        let state = rig.state();
        let team = state.team(&team_id).unwrap();
        team.rounds().len() == 2
            && team.rounds()[1].is_open()
            && match team.item("1.1".parse().unwrap()).unwrap().phase() {
                ItemPhase::InProgress { assignee, .. } => assignee.as_str() == "inv-lead",
                _ => false,
            }
    })
    .await;
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn split_part_restart_between_prior_merge_and_next_opening() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate"),
            normal("claude-1", "work,sleep"),
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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();

    // Part A approves:
    rig.wait_for("part a approved", || {
        let wire: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(rig.home.state_file()).unwrap()).unwrap();
        wire["teams"][0]["items"][0]["phase"]["phase"] == "approved"
    })
    .await;

    // Keep the next round unopened until recovery observes the merged pull request.
    rig.stop().await.unwrap();
    assert_eq!(rig.state().team(&team_id).unwrap().rounds().len(), 1);
    rig.github.merged_by_a_human(1);

    // Restart daemon:
    rig.start().await;

    // Part B opens cleanly:
    rig.wait_for("part b opened after restart following merge", || {
        let state = rig.state();
        let team = state.team(&team_id).unwrap();
        team.rounds().len() == 2 && team.rounds()[1].is_open()
    })
    .await;
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn split_part_with_outside_replacement_reuses_stopped_members_task() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate"),
            normal("claude-1", "work,sleep"),
            normal("claude-2", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    let mut cfg = std::fs::read_to_string(rig.home.config_file()).unwrap();
    cfg.push_str("\n[replacement]\nagents = [\"claude-2\"]\n");
    std::fs::write(rig.home.config_file(), cfg).unwrap();
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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();

    // Part A approves:
    rig.wait_for("part a approved", || {
        let wire: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(rig.home.state_file()).unwrap()).unwrap();
        wire["teams"][0]["items"][0]["phase"]["phase"] == "approved"
    })
    .await;

    // Stop claude-1 before part A merges:
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Task(tasks[1].clone()),
            }),
        )
        .await
        .unwrap();
    rig.status_is(&tasks[1], TaskStatus::Stopped).await;

    // Part A merges:
    rig.github.merged_by_a_human(1);

    // Part B opens for claude-2:
    rig.wait_for("part b opened for replacement member claude-2", || {
        let state = rig.state();
        let team = state.team(&team_id).unwrap();
        team.rounds().len() == 2
            && team.rounds()[1].is_open()
            && match team.item("1.1".parse().unwrap()).unwrap().phase() {
                ItemPhase::InProgress { assignee, .. } => assignee.as_str() == "claude-2",
                _ => false,
            }
    })
    .await;
    let state = rig.state();
    let team = state.team(&team_id).unwrap();
    assert_eq!(team.rounds()[1].part().unwrap().to_string(), "b");
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn later_split_part_accepts_the_parent_task_branch_for_external_delivery() {
    let mut rig = Rig::new(
        &[
            investigator(
                "inv-lead",
                "ask,clean-done:DELIVERED: #1,clean-done:DELIVERED: #2,sleep",
            ),
            normal("claude-1", "clean-done:already delivered,clean-done:already delivered"),
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
    let branch_task = format!("agents/claude-1/{}-1.1", tasks[1]);
    let branch_part_b = format!("agents/claude-1/{}-1.1b", tasks[1]);
    super::rig::git(&rig.origin, &["branch", &branch_task, "main"]);
    super::rig::git(&rig.origin, &["branch", &branch_part_b, "main"]);
    let pr1 = rig.github.foreign_pull(&branch_task);
    let pr2 = rig.github.foreign_pull(&branch_task);
    assert_eq!(pr1, 1);
    assert_eq!(pr2, 2);
    rig.github.merged_by_a_human(1);
    rig.github.merged_by_a_human(2);
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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();
    rig.state_where("round 1 closed", |state| {
        let team = state.team(&team_id).unwrap();
        team.rounds().first().is_some_and(|r| !r.is_open())
    })
    .await;
    let state = rig
        .state_where("team done after last part", |state| {
            let team = state.team(&team_id).unwrap();
            matches!(
                team.item("1.1".parse().unwrap()).unwrap().phase(),
                ItemPhase::Done { .. }
            )
        })
        .await;
    let team = state.team(&team_id).unwrap();
    assert_eq!(team.rounds().len(), 2);
    assert_eq!(team.rounds()[0].delivered_by().unwrap().number.get().get(), 1);
    assert_eq!(team.rounds()[0].part().unwrap().to_string(), "a");
    assert_eq!(team.rounds()[1].delivered_by().unwrap().number.get().get(), 2);
    assert_eq!(team.rounds()[1].part().unwrap().to_string(), "b");
    assert!(team.rounds().iter().all(|round| !round.is_open()));
    rig.stop().await.unwrap();
}

fn add_collision_test_worktree(rig: &Rig, task: &TaskId, branch: &str) {
    super::rig::git(
        &rig.repo,
        &["worktree", "add", rig.worktree(task).to_str().unwrap(), branch],
    );
}

#[tokio::test]
async fn split_branch_collision_keeps_the_old_holder_stopped_until_a_round_can_open() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
            normal("claude-2", "sleep"),
            normal("claude-1", "sleep"),
        ],
        5,
        false,
    )
    .await;
    let fixture = include_str!("../../../../../../agent-core/tests/fixtures/state-before-split-delivery.json");
    std::fs::write(rig.home.state_file(), fixture).unwrap();
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str("\n[replacement]\nagents = [\"claude-2\"]\n");
    std::fs::write(rig.home.config_file(), config).unwrap();

    let team_id: TeamId = "team-20261121-120000-spl1".parse().unwrap();
    let lead_task: TaskId = "task-20261121-120001-spl1".parse().unwrap();
    let member_task: TaskId = "task-20261121-120002-spl1".parse().unwrap();

    std::fs::create_dir_all(rig.home.task_dir(&lead_task)).unwrap();
    std::fs::create_dir_all(rig.home.task_dir(&member_task)).unwrap();
    let lead_branch = format!("agents/inv-lead/{lead_task}-1.1");
    let branch_c = format!("agents/inv-lead/{lead_task}-1.1c");
    let member_branch = format!("agents/claude-1/{member_task}-1.1");
    super::rig::git(&rig.repo, &["branch", &lead_branch, "main"]);
    super::rig::git(&rig.origin, &["branch", &branch_c, "main"]);
    super::rig::git(&rig.repo, &["branch", &member_branch, "main"]);
    add_collision_test_worktree(&rig, &lead_task, &lead_branch);
    add_collision_test_worktree(&rig, &member_task, &member_branch);

    rig.github
        .foreign_pull("agents/claude-1/task-20261121-120002-spl1-1.1a");
    rig.github
        .foreign_pull("agents/claude-1/task-20261121-120002-spl1-1.1b");
    let pr3 = rig.github.foreign_pull(&branch_c);
    assert_eq!(pr3, 3);
    rig.github.merged_by_a_human(3);

    let collision = format!("agents/claude-2/{member_task}-1.1c");
    let checkout = rig.worktree(&member_task);
    super::rig::git(&checkout, &["branch", &collision, "HEAD"]);
    let layer = agent_runtime::history::HistoryLayer::deferred(
        agent_core::log::LogLevel::Info,
        rig.history_test_clock(),
        agent_core::config::Secrets::default(),
    );
    rig.start_with_log_layer(layer).await;
    rig.wait_for("branch collision observed", || {
        let reader = agent_runtime::history::Reader::open(&rig.home.history_file()).unwrap();
        reader
            .daemon_logs()
            .unwrap()
            .iter()
            .any(|row| row.event() == agent_core::log::LogEvent::GitCallFailed && row.task() == Some(&member_task))
    })
    .await;
    let blocked = rig.state();
    let holder = blocked.find_task(&member_task).unwrap().member;
    assert_eq!(holder.agent().name(), "claude-1");
    assert_eq!(holder.status(), TaskStatus::Stopped);
    assert_eq!(blocked.team(&team_id).unwrap().rounds().len(), 2);
    super::rig::git(&checkout, &["branch", "-d", &collision]);
    let state = rig
        .state_where("replacement has its new round before running", |state| {
            let team = state.team(&team_id).unwrap();
            team.rounds().len() == 3
                && team.rounds()[2].is_open()
                && state.find_task(&member_task).unwrap().member.agent().name() == "claude-2"
                && state.find_task(&member_task).unwrap().member.status() == TaskStatus::Running
        })
        .await;
    assert_eq!(
        state.team(&team_id).unwrap().rounds()[2].part().unwrap().to_string(),
        "c"
    );
    assert_eq!(state.find_task(&member_task).unwrap().member.agent().name(), "claude-2");
    assert_eq!(
        state.find_task(&member_task).unwrap().member.status(),
        TaskStatus::Running
    );
    rig.stop().await.unwrap();
    let reader = agent_runtime::history::Reader::open(&rig.home.history_file()).unwrap();
    assert_eq!(
        reader
            .daemon_logs()
            .unwrap()
            .iter()
            .filter(|row| {
                row.event() == agent_core::log::LogEvent::TaskHandedOver
                    && row.task() == Some(&member_task)
                    && row.fields().get("to").and_then(|v| v.as_str()) == Some("claude-2")
            })
            .count(),
        1
    );
}
#[tokio::test]
async fn outside_replacement_reuses_a_stopped_coordinators_task_for_the_next_part() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
            investigator("inv-extra", "sleep"),
            normal("claude-1", "sleep"),
        ],
        5,
        false,
    )
    .await;
    let fixture = include_str!("../../../../../../agent-core/tests/fixtures/state-before-split-delivery.json");
    let mut wire: serde_json::Value = serde_json::from_str(fixture).unwrap();
    let last_run = wire["teams"][0]["crew"]["members"][0]["runs"]
        .as_array_mut()
        .unwrap()
        .pop()
        .unwrap();
    wire["teams"][0]["crew"]["coordinator"]["runs"]
        .as_array_mut()
        .unwrap()
        .push(last_run);
    wire["teams"][0]["crew"]["coordinator"]["status"] = serde_json::json!("stopped");
    std::fs::write(rig.home.state_file(), serde_json::to_string(&wire).unwrap()).unwrap();
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str("\n[replacement]\nagents = [\"inv-extra\"]\n");
    std::fs::write(rig.home.config_file(), config).unwrap();

    let team_id: TeamId = "team-20261121-120000-spl1".parse().unwrap();
    let lead_task: TaskId = "task-20261121-120001-spl1".parse().unwrap();
    let member_task: TaskId = "task-20261121-120002-spl1".parse().unwrap();

    std::fs::create_dir_all(rig.home.task_dir(&lead_task)).unwrap();
    std::fs::create_dir_all(rig.home.task_dir(&member_task)).unwrap();
    let lead_branch = format!("agents/inv-lead/{lead_task}-1.1");
    let branch_c = format!("agents/inv-lead/{lead_task}-1.1c");
    let member_branch = format!("agents/claude-1/{member_task}-1.1");
    super::rig::git(&rig.repo, &["branch", &lead_branch, "main"]);
    super::rig::git(&rig.origin, &["branch", &branch_c, "main"]);
    super::rig::git(&rig.repo, &["branch", &member_branch, "main"]);
    super::rig::git(
        &rig.repo,
        &[
            "worktree",
            "add",
            rig.worktree(&lead_task).to_str().unwrap(),
            &lead_branch,
        ],
    );
    super::rig::git(
        &rig.repo,
        &[
            "worktree",
            "add",
            rig.worktree(&member_task).to_str().unwrap(),
            &member_branch,
        ],
    );

    rig.github
        .foreign_pull("agents/claude-1/task-20261121-120002-spl1-1.1a");
    rig.github
        .foreign_pull("agents/claude-1/task-20261121-120002-spl1-1.1b");
    let pr3 = rig.github.foreign_pull(&branch_c);
    assert_eq!(pr3, 3);
    rig.github.merged_by_a_human(3);

    rig.start().await;

    let state = rig
        .state_where(
            "outside replacement receives a round after the stopped coordinator",
            |state| {
                let team = state.team(&team_id).unwrap();
                team.rounds().len() == 3
                    && team.rounds()[2].is_open()
                    && state.find_task(&lead_task).unwrap().member.agent().name() == "inv-extra"
                    && state.find_task(&lead_task).unwrap().member.status() == TaskStatus::Running
            },
        )
        .await;
    assert_eq!(
        state.team(&team_id).unwrap().rounds()[2].part().unwrap().to_string(),
        "c"
    );
    assert_eq!(
        state.find_task(&member_task).unwrap().member.status(),
        TaskStatus::Stopped
    );
    rig.stop().await.unwrap();
}

/// One line of Claude Code's stream JSON: a model call of `input` tokens of context.
fn usage_line(input: u64) -> String {
    format!(
        "{{\"type\":\"assistant\",\"message\":{{\"id\":\"msg-{input}\",\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"working\"}}],\"usage\":{{\"input_tokens\":{input},\"cache_read_input_tokens\":0,\"cache_creation_input_tokens\":0,\"output_tokens\":100}}}}}}\n"
    )
}

/// Runs the split of one item into two parts, in which the member that did part a works on part b in the same task: its
/// first run streams `input` tokens. Returns the arguments and the prompt of its second run, which is part b's first run.
async fn next_part_of_a_split(input: u64) -> (String, String) {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate"),
            normal("claude-1", "work-stream:first,ask-here,work"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(scripts.join("first.jsonl"), usage_line(input)).unwrap();
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;
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
                answers: answers(&[(1, &approval(2))]),
            }),
        )
        .await
        .unwrap();
    rig.status_is(&tasks[1], TaskStatus::Questions).await;
    let worktree = rig.worktree(&tasks[1]);
    let args = std::fs::read_to_string(worktree.join(".fake-args-2")).unwrap();
    let prompt = rig.prompt(&tasks[1], 2).await;
    rig.stop().await.unwrap();
    (args, prompt)
}

// Rule: a member whose session stays below its agent's bound resumes it for the next part of its split, as before: part b's
// first run is the second run of the task, and it resumes the session of part a.
#[tokio::test]
async fn a_session_below_the_bound_resumes_for_the_next_part_of_a_split() {
    let (args, _) = next_part_of_a_split(42_000).await;

    assert!(
        args.contains("--resume"),
        "the next part does not resume its session: {args}"
    );
}

// Rule: a session above the bound is not resumed for the next part of a split either: the part's first run is briefed.
#[tokio::test]
async fn a_session_above_the_bound_is_replaced_for_the_next_part_of_a_split() {
    let (args, prompt) = next_part_of_a_split(200_000).await;

    assert!(
        !args.contains("--resume"),
        "the session above the bound was resumed: {args}"
    );
    assert!(prompt.contains("grown to 200000 tokens"), "{prompt}");
}
