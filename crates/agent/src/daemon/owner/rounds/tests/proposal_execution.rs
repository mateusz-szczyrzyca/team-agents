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

#[tokio::test]
async fn missing_checks_are_corrected_without_a_human_question() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "proposal:bad,proposal:good,sleep"),
            normal("claude-1", "sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        5,
        true,
    )
    .await;
    put(&rig, "bad", &proposal(""));
    put(&rig, "good", &proposal("Read first"));
    rig.start().await;
    let (id, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;
    let state = rig
        .state_where("corrected proposal approved", |state| {
            !state.team(&id).unwrap().splits().is_empty()
        })
        .await;
    assert_eq!(state.team(&id).unwrap().splits().len(), 1);
    let prompt = rig.prompt(&tasks[0], 2).await;
    assert!(prompt.contains("Part a has no checks"), "{prompt}");
    assert!(!rig.home.questions_file(&tasks[0], std::num::NonZeroU32::MIN).exists());
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn scope_change_and_exhaustion_ask_once_and_accept_legacy_split_or_free_text() {
    use crate::client::Connection;
    use crate::daemon::owner::fixture::{answers, n};
    use agent_core::protocol::{AnswerRequest, Client, Command};
    for (scope_changed, answer_split) in [(true, true), (true, false), (false, false)] {
        let mut rig = Rig::new(
            &[
                investigator("inv-lead", "proposal:bad,proposal:bad,proposal:bad,sleep"),
                normal("claude-1", "sleep"),
                investigator("inv-r1", "done:VERDICT: APPROVE"),
                investigator("inv-r2", "done:VERDICT: APPROVE"),
            ],
            5,
            true,
        )
        .await;
        let mut value = proposal(if scope_changed { "Read first" } else { "" });
        if scope_changed {
            value["scope"] = serde_json::json!({"kind":"less", "reason":"Omit translations"});
        }
        put(&rig, "bad", &value);
        rig.start().await;
        let (id, tasks) = rig
            .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
            .await;
        let state = rig.status_is(&tasks[0], TaskStatus::Questions).await;
        assert_eq!(state.team(&id).unwrap().splits().len(), 0);
        assert_eq!(
            state.team(&id).unwrap().crew().coordinator().unwrap().questions().len(),
            1
        );
        let path = rig.home.questions_file(&tasks[0], n(1));
        let question = std::fs::read_to_string(&path).unwrap();
        assert!(
            question.contains(if scope_changed {
                "Omit translations"
            } else {
                "Part a has no checks"
            }),
            "{question}"
        );
        assert!(question.contains("```json"), "{question}");
        rig.stop().await.unwrap();
        rig.start().await;
        assert_eq!(
            rig.state()
                .team(&id)
                .unwrap()
                .crew()
                .coordinator()
                .unwrap()
                .questions()
                .len(),
            1
        );
        let valid = proposal("Read first");
        let legacy = serde_json::json!({"parent":"1.1", "parts":valid["parts"]});
        let verdict = if answer_split {
            format!("split\n```json\n{legacy}\n```")
        } else {
            "Keep the original scope and reconsider".to_owned()
        };
        Connection::connect(&rig.home.socket_file())
            .await
            .unwrap()
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
        let state = rig.state();
        assert_eq!(state.team(&id).unwrap().splits().len(), usize::from(answer_split));
        if !answer_split {
            let run = if scope_changed { 2 } else { 4 };
            let prompt = rig.prompt(&tasks[0], run).await;
            assert!(prompt.contains(&verdict), "{prompt}");
        }
        rig.stop().await.unwrap();
    }
}

#[tokio::test]
async fn saved_pending_proposal_is_approved_after_restart() {
    use agent_core::state::split::keeping::ProposalKeeping;
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
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
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    rig.status_is(&tasks[1], TaskStatus::Waiting).await;
    rig.stop().await.unwrap();
    let mut state = rig.state();
    state
        .team_mut(&id)
        .unwrap()
        .keep_split_proposal(
            "1.1".parse().unwrap(),
            ProposalKeeping::new(proposal("Read first").to_string()).unwrap(),
        )
        .unwrap();
    std::fs::write(rig.home.state_file(), serde_json::to_string(&state).unwrap()).unwrap();
    rig.start().await;
    let state = rig
        .state_where("saved pending proposal approved", |state| {
            state.team(&id).unwrap().splits().len() == 1
        })
        .await;
    assert_eq!(state.team(&id).unwrap().split_proposal("1.1".parse().unwrap()), None);
    assert_eq!(
        state.team(&id).unwrap().crew().coordinator().unwrap().questions().len(),
        0
    );
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn member_proposal_reaches_coordinator_without_human_questions() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep"),
            normal("claude-1", "proposal:good,sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        3,
        true,
    )
    .await;
    put(&rig, "good", &proposal("Read first"));
    rig.start().await;
    let (id, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;
    rig.prompt(&tasks[0], 2).await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    rig.stop().await.unwrap();
    let messages = std::fs::read_to_string(rig.home.root().join("bus/messages.jsonl")).unwrap();
    assert!(messages.contains("claude-1 requests a split of item 1.1"), "{messages}");
    let state = rig.state();
    assert_eq!(state.team(&id).unwrap().splits().len(), 0);
    for task in tasks {
        assert_eq!(state.find_task(&task).unwrap().member.questions().len(), 0);
        assert!(!rig.home.questions_file(&task, std::num::NonZeroU32::MIN).exists());
    }
}

#[tokio::test]
async fn decisions_and_rejections_are_recorded_in_history() {
    use agent_core::history::Record;
    use agent_runtime::history::Reader;
    let mut cases = Vec::new();
    cases.push(("checks", proposal(""), "Part a has no checks"));
    let mut cycle = proposal("Read first");
    cycle["parts"][1]["depends_on"] = serde_json::json!(["b"]);
    cases.push(("cycle", cycle, "Part b belongs to a dependency cycle"));
    let mut limit = proposal("Read first");
    limit["parts"][1]["estimate"]["amount"] = 1501.into();
    cases.push(("limit", limit, "Part b estimates 1501 lines"));
    cases.push((
        "unreadable",
        serde_json::json!({"parent":"1.1","unexpected":true}),
        "cannot be read in the proposal form",
    ));
    let mut scope = proposal("Read first");
    scope["scope"] = serde_json::json!({"kind":"more","reason":"Add translations"});
    cases.push(("scope", scope, "Add translations"));
    for (name, value, reason) in cases {
        let human = name == "scope";
        let mut rig = Rig::new(
            &[
                investigator("inv-lead", "proposal:bad,proposal:good,sleep"),
                normal("claude-1", "sleep"),
                investigator("inv-r1", "done:VERDICT: APPROVE"),
                investigator("inv-r2", "done:VERDICT: APPROVE"),
            ],
            3,
            true,
        )
        .await;
        put(&rig, "bad", &value);
        put(&rig, "good", &proposal("Read first"));
        let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
        config.push_str("\n[log]\nlevel = \"debug\"\n");
        std::fs::write(rig.home.config_file(), config).unwrap();
        rig.start().await;
        let (id, tasks) = rig
            .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
            .await;
        if human {
            rig.status_is(&tasks[0], TaskStatus::Questions).await;
        } else {
            rig.state_where("corrected proposal approved", |state| {
                state.team(&id).unwrap().splits().len() == 1
            })
            .await;
            assert!(rig.prompt(&tasks[0], 2).await.contains(reason));
        }
        rig.stop().await.unwrap();
        let logs = Reader::open(&rig.home.history_file()).unwrap().daemon_logs().unwrap();
        assert!(
            logs.iter().any(|row| row.event().as_str() == "decision_taken"
                && row.fields().get("decision") == Some(&serde_json::json!("split_proposal"))),
            "missing split decision log: {name}"
        );
        let events: Vec<_> = Reader::open(&rig.home.history_file())
            .unwrap()
            .team_records(&id)
            .unwrap()
            .into_iter()
            .filter_map(|row| match row.record {
                Record::Event(event) if event.event.kind().to_string() == "split_decision" => {
                    Some(serde_json::to_value(event.event).unwrap())
                }
                _ => None,
            })
            .collect();
        let outcomes: Vec<_> = events.iter().map(|event| event["decision"].as_str().unwrap()).collect();
        assert_eq!(
            outcomes,
            if human {
                vec!["asked"]
            } else {
                vec!["corrected", "approved"]
            },
            "{name}: {events:?}"
        );
        assert!(events[0]["reason"].as_str().unwrap().contains(reason), "{events:?}");
    }
}

#[tokio::test]
async fn missing_estimate_names_its_part_in_the_correction() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "proposal:bad,proposal:good,sleep"),
            normal("claude-1", "sleep"),
            investigator("inv-r1", "done:VERDICT: APPROVE"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        3,
        true,
    )
    .await;
    let mut value = proposal("Read first");
    value["parts"][1].as_object_mut().unwrap().remove("estimate");
    put(&rig, "bad", &value);
    put(&rig, "good", &proposal("Read first"));
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;

    let prompt = rig.prompt(&tasks[0], 2).await;

    rig.stop().await.unwrap();
    assert!(
        prompt.contains("Part b cannot be read in the proposal form"),
        "{prompt}"
    );
}
