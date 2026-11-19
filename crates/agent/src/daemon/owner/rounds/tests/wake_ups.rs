//! A message that reaches a lead while it works on a fix pass neither loses the fix response nor wakes the lead without end.

use agent_core::config::Secrets;
use agent_core::ids::TaskId;
use agent_core::log::{LogEntry, LogEvent, LogLevel};
use agent_core::protocol::{Client, Command, SayRequest, SayText, Target};
use agent_core::state::{ItemPhase, MergeMode};
use agent_core::status::TaskStatus;
use agent_runtime::history::{HistoryLayer, Reader};

use super::rig::{ONE, Rig, investigator, normal};
use crate::client::Connection;

/// A first review that asks for one change, then approves the fix.
const CHANGES_ONCE: &str = "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: fixed";
/// A review with nothing to ask for.
const APPROVING: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// Says `text` to the agent of `task`, as `agent say` does.
async fn say(rig: &Rig, task: &TaskId, text: &str) {
    let request = SayRequest {
        target: Target::Task(task.clone()),
        text: SayText::try_from(text.to_owned()).unwrap(),
    };
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, Command::Say(request))
        .await
        .unwrap();
}

/// A team whose coordinator integrates, then does its fix pass in its third run, which waits until the test releases it,
/// and then ignores its messages in every run.
async fn held_fix_pass() -> Rig {
    Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate,deaf-fix-held:1,deaf-answer:1"),
            normal("claude-1", "work"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        true,
    )
    .await
}

#[tokio::test]
async fn a_message_during_a_fix_pass_does_not_lose_the_fix_response() {
    let mut rig = held_fix_pass().await;
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    let worktree = rig.worktree(&tasks[0]);
    rig.wait_for("the fix pass to start", || worktree.join(".fake-started-3").exists())
        .await;
    say(&rig, &tasks[0], "please also look at the docs").await;
    std::fs::write(worktree.join(".fake-release-3"), "").unwrap();

    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();
}

/// A team whose coordinator sleeps after its first run and then ignores its messages, and whose only member works until
/// the test releases it: the team stays open while the coordinator is woken.
async fn deaf_lead() -> Rig {
    let rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,deaf-sleep"),
            normal("claude-1", "fix-held:1"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        true,
    )
    .await;
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str("\n[limits]\nwake_ups_per_message = 2\n");
    std::fs::write(rig.home.config_file(), config).unwrap();
    rig
}

/// Starts the daemon so that its log is recorded.
async fn start_logged(rig: &mut Rig) {
    rig.start_with_log_layer(HistoryLayer::deferred(
        LogLevel::Info,
        rig.history_test_clock(),
        Secrets::default(),
    ))
    .await;
}

/// The rows of the daemon's log so far, as the history has them; none while it cannot be read.
fn logged(rig: &Rig) -> Vec<LogEntry> {
    Reader::open(&rig.home.history_file())
        .and_then(|reader| reader.daemon_logs())
        .unwrap_or_default()
}

#[tokio::test]
async fn a_lead_that_ignores_its_message_is_woken_the_configured_number_of_times_and_then_left_alone() {
    let mut rig = deaf_lead().await;
    start_logged(&mut rig).await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    say(&rig, &tasks[0], "are you there?").await;

    rig.wait_for("the wake-ups to stop", || {
        logged(&rig)
            .iter()
            .any(|row| row.event() == LogEvent::BusDown && row.task() == Some(&tasks[0]))
    })
    .await;
    std::fs::write(rig.worktree(&tasks[1]).join(".fake-release-1"), "").unwrap();
    let runs = rig.runs(&tasks[0]);
    rig.stop().await.unwrap();

    assert_eq!(runs, 3, "one run of its own and two wake-ups");
    let stop = logged(&rig)
        .into_iter()
        .find(|row| row.event() == LogEvent::BusDown && row.task() == Some(&tasks[0]))
        .unwrap();
    assert_eq!(stop.fields().get("wake_ups"), Some(&serde_json::json!(2)));
    assert_eq!(stop.fields().get("from"), Some(&serde_json::json!("human")));
    assert!(stop.fields().contains_key("message"), "{:?}", stop.fields());
}

#[tokio::test]
async fn every_wake_up_is_logged_with_its_message_and_its_sender() {
    let mut rig = deaf_lead().await;
    start_logged(&mut rig).await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    say(&rig, &tasks[0], "are you there?").await;
    rig.wait_for("two wake-ups", || {
        logged(&rig)
            .iter()
            .filter(|row| row.event() == LogEvent::RunStarted && row.fields().contains_key("wake_ups"))
            .count()
            >= 2
    })
    .await;
    std::fs::write(rig.worktree(&tasks[1]).join(".fake-release-1"), "").unwrap();
    rig.stop().await.unwrap();

    let wake_ups: Vec<_> = logged(&rig)
        .into_iter()
        .filter(|row| row.event() == LogEvent::RunStarted && row.task() == Some(&tasks[0]))
        .filter(|row| row.fields().contains_key("wake_ups"))
        .collect();
    let counts: Vec<_> = wake_ups
        .iter()
        .map(|row| row.fields().get("wake_ups").cloned())
        .collect();
    assert_eq!(counts, [Some(serde_json::json!(1)), Some(serde_json::json!(2))]);
    for row in &wake_ups {
        assert_eq!(row.fields().get("why"), Some(&serde_json::json!("wake_up")));
        assert_eq!(row.fields().get("from"), Some(&serde_json::json!("human")));
        assert!(row.fields().contains_key("message"), "{:?}", row.fields());
    }
}
