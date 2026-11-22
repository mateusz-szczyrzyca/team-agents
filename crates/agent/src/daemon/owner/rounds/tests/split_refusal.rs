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
