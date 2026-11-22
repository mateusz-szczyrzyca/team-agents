//! A `SPLIT:` line the harness cannot take is recorded with its reason, and its author is told and woken for it.

use super::rig::{ONE, Rig, investigator, normal};
use agent_core::history::Record;
use agent_core::state::MergeMode;
use agent_runtime::history::Reader;

fn proposal() -> serde_json::Value {
    serde_json::json!({"parent":"1.1", "scope":{"kind":"unchanged"}, "parts":[
        {"title":"First", "delivers":"Independent first deliverable", "checks":"Read first",
         "depends_on":[], "estimate":{"amount":100,"unit":"lines"}},
        {"title":"Second", "delivers":"Independent second deliverable", "checks":"Read second",
         "depends_on":["a"], "estimate":{"amount":100,"unit":"lines"}}
    ]})
}

// Rule: a coordinator that ends a run with `SPLIT: 1.1` for an item that is split already is not left to wait: the refusal
// is recorded in the log and the history with its reason, the coordinator is told in a message of the catalog that says
// what to do instead, and it is woken for it, with no question to the human.
#[tokio::test]
async fn a_second_split_of_an_item_is_refused_recorded_and_told_to_its_author() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "proposal:good,proposal:good,sleep"),
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
    std::fs::write(dir.join("good.json"), proposal().to_string()).unwrap();
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str("\n[log]\nlevel = \"debug\"\n");
    std::fs::write(rig.home.config_file(), config).unwrap();
    rig.start().await;
    let (id, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;

    // The third run of the coordinator exists only because the second was woken, with the reason in the bus.
    rig.prompt(&tasks[0], 3).await;
    rig.stop().await.unwrap();

    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let events: Vec<_> = reader
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
    assert_eq!(outcomes, vec!["approved", "corrected"], "{events:?}");
    assert!(
        events[1]["reason"].as_str().unwrap().contains("already split"),
        "{events:?}"
    );
    let logs = reader.daemon_logs().unwrap();
    let decisions = logs
        .iter()
        .filter(|row| {
            row.event().as_str() == "decision_taken"
                && row.fields().get("decision") == Some(&serde_json::json!("split_proposal"))
        })
        .count();
    assert_eq!(decisions, 2);
    let messages = std::fs::read_to_string(rig.home.root().join("bus/messages.jsonl")).unwrap();
    assert!(messages.contains("Item 1.1 is already split"), "{messages}");
    assert!(!rig.home.questions_file(&tasks[0], std::num::NonZeroU32::MIN).exists());
}

/// A team whose coordinator runs `script`, with the proposal `good` in the home and the debug log on.
async fn team_with(script: &'static str) -> Rig {
    let rig = Rig::new(
        &[
            investigator("inv-lead", script),
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
    std::fs::write(dir.join("good.json"), proposal().to_string()).unwrap();
    rig
}

fn messages(rig: &Rig) -> String {
    std::fs::read_to_string(rig.home.root().join("bus/messages.jsonl")).unwrap_or_default()
}

// Rule: a name that is no item of the team is refused by name, and its author is told and woken.
#[tokio::test]
async fn a_split_of_an_item_the_team_does_not_have_is_refused_by_name_and_told() {
    let mut rig = team_with("split-line:9.9@SLEEPING,sleep").await;
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;

    rig.prompt(&tasks[0], 2).await;
    rig.stop().await.unwrap();

    assert!(
        messages(&rig).contains("The team has no item named `9.9`"),
        "{}",
        messages(&rig)
    );
    assert!(!rig.home.questions_file(&tasks[0], std::num::NonZeroU32::MIN).exists());
}

// Rule: the refusal is told and the run's own STATUS: is acted on: QUESTIONS puts the question to the human, as it does
// when no split is refused.
#[tokio::test]
async fn a_refusal_does_not_throw_away_the_status_questions() {
    use agent_core::status::TaskStatus;
    let mut rig = team_with("proposal:good,split-line:1.1@QUESTIONS").await;
    rig.start().await;
    let (id, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;

    let state = rig.status_is(&tasks[0], TaskStatus::Questions).await;
    rig.stop().await.unwrap();

    assert_eq!(
        state.team(&id).unwrap().crew().coordinator().unwrap().questions().len(),
        1
    );
    assert!(
        messages(&rig).contains("Item 1.1 is already split"),
        "{}",
        messages(&rig)
    );
}

// Rule: a refused line followed by STATUS: DONE ends the run as DONE does: the coordinator is told, and its task is not
// failed or lost.
#[tokio::test]
async fn a_refusal_does_not_throw_away_the_status_done() {
    use agent_core::state::{RunEnd, RunState};
    use agent_core::status::RunOutcome;
    let mut rig = team_with("proposal:good,split-line:1.1@DONE,sleep").await;
    rig.start().await;
    let (id, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;

    rig.prompt(&tasks[0], 2).await;
    let state = rig
        .state_where("the run that ended with DONE is recorded", |state| {
            state
                .team(&id)
                .and_then(|team| team.find(&tasks[0]).map(|(_, member)| member.runs().to_vec()))
                .is_some_and(|runs| {
                    runs.get(1).is_some_and(|run| {
                        matches!(
                            run.state(),
                            RunState::Ended {
                                end: RunEnd::Reported {
                                    outcome: RunOutcome::Done
                                },
                                ..
                            }
                        )
                    })
                })
        })
        .await;
    rig.stop().await.unwrap();

    assert!(state.team(&id).is_some());
    assert!(
        messages(&rig).contains("Item 1.1 is already split"),
        "{}",
        messages(&rig)
    );
}

// Rule: a coordinator that repeats a refused line is told as many times as a proposal is corrected (`[split] corrections`,
// two) and then its own STATUS: is acted on: it is not run again forever.
#[tokio::test]
async fn a_coordinator_that_repeats_a_refused_line_is_told_twice_and_then_left_to_its_status() {
    use agent_core::status::TaskStatus;
    let mut rig =
        team_with("proposal:good,split-line:1.1@SLEEPING,split-line:1.1@SLEEPING,split-line:1.1@SLEEPING,sleep").await;
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;

    rig.prompt(&tasks[0], 4).await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    let told = messages(&rig).matches("Item 1.1 is already split").count();
    let runs = rig.runs(&tasks[0]);
    rig.stop().await.unwrap();

    assert_eq!(told, 2, "{}", messages(&rig));
    assert_eq!(runs, 4);
}
