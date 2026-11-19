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
    assert!(stop.fields().contains_key("message_id"), "{:?}", stop.fields());
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
        assert!(row.fields().contains_key("message_id"), "{:?}", row.fields());
    }
}

/// A team whose coordinator ends its fix pass with nothing to say (`sleep`), and gives the response in the run a message
/// wakes it for.
async fn sleepy_fix_pass() -> Rig {
    Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate,sleep,fix-work:1"),
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
async fn a_wake_up_during_an_open_fix_pass_says_so_in_one_line_and_its_response_starts_the_verification() {
    let mut rig = sleepy_fix_pass().await;
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    let fix_pass = rig.prompt(&tasks[0], 3).await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    assert_eq!(
        rig.runs(&tasks[0]),
        3,
        "a lead that sleeps is not woken again by the round"
    );
    say(&rig, &tasks[0], "how far are you?").await;

    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    let wake_up = rig.kept_prompt(&tasks[0], 4);
    rig.stop().await.unwrap();

    assert!(fix_pass.contains("THE REVIEWERS ASKED FOR CHANGES"), "{fix_pass}");
    assert!(
        wake_up.contains("fix pass 1 of pull request #1 is still open"),
        "{wake_up}"
    );
    assert!(!wake_up.contains("THE REVIEWERS ASKED FOR CHANGES"), "{wake_up}");
}

#[tokio::test]
async fn a_wake_up_with_nothing_to_do_leaves_the_round_as_it_was() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate,sleep,sleep,fix-work:1"),
            normal("claude-1", "work"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.wait_for("the fix pass run", || rig.runs(&tasks[0]) == 3).await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    let before = rig.state();
    say(&rig, &tasks[0], "no hurry").await;
    rig.wait_for("the wake-up to run", || rig.runs(&tasks[0]) == 4).await;
    let state = rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    rig.stop().await.unwrap();

    let phase =
        |state: &agent_core::state::State| state.teams()[0].item("1.1".parse().unwrap()).unwrap().phase().clone();
    assert_eq!(phase(&state), phase(&before));
    assert!(matches!(phase(&state), ItemPhase::Fixing { .. }));
}

/// Gives the home a limit on wake-ups that the replay cannot reach, so that only the hold can save the round.
fn raise_wake_up_limit(rig: &Rig) {
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str("\n[limits]\nwake_ups_per_message = 1000\n");
    std::fs::write(rig.home.config_file(), config).unwrap();
}

#[tokio::test]
async fn a_lead_whose_fix_response_was_taken_is_published_before_it_is_woken_again() {
    let rig = held_fix_pass().await;
    raise_wake_up_limit(&rig);
    let mut rig = rig;
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

/// A line of the bus log that the daemon cannot read as a message and the bus itself ignores: its number is 0.
const UNREADABLE_LINE: &str = r#"{"id":0,"ts":"2026-11-19 14:00:00","from":"nobody","to":"nobody","text":"x"}"#;

#[tokio::test]
async fn an_unreadable_bus_log_is_told_once_and_does_not_stretch_the_wake_ups() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,deaf-fix-held:1,deaf-sleep"),
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
    start_logged(&mut rig).await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    say(&rig, &tasks[0], "are you there?").await;
    let worktree = rig.worktree(&tasks[0]);
    rig.wait_for("the first wake-up", || worktree.join(".fake-started-2").exists())
        .await;
    {
        use std::io::Write;
        let mut log = std::fs::OpenOptions::new()
            .append(true)
            .open(rig.home.root().join("bus/messages.jsonl"))
            .unwrap();
        writeln!(log, "{UNREADABLE_LINE}").unwrap();
    }
    std::fs::write(worktree.join(".fake-release-2"), "").unwrap();

    rig.wait_for("the wake-ups to stop", || {
        logged(&rig)
            .iter()
            .any(|row| row.event() == LogEvent::BusDown && row.fields().contains_key("wake_ups"))
    })
    .await;
    std::fs::write(rig.worktree(&tasks[1]).join(".fake-release-1"), "").unwrap();
    let runs = rig.runs(&tasks[0]);
    rig.stop().await.unwrap();

    assert_eq!(
        runs, 3,
        "one run of its own and two wake-ups, with the log unreadable for the second"
    );
    let unreadable: Vec<_> = logged(&rig)
        .into_iter()
        .filter(|row| {
            row.event() == LogEvent::BusDown && row.task() == Some(&tasks[0]) && row.fields().contains_key("error")
        })
        .collect();
    assert_eq!(unreadable.len(), 1, "{unreadable:?}");
    assert_eq!(unreadable[0].count().get(), 1);
}

#[tokio::test]
async fn wake_delivery_is_saved_before_running_and_marked_delivered_at_the_limit() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,deaf-fix-held:1,deaf-sleep"),
            normal("claude-1", "fix-held:1"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        true,
    )
    .await;
    start_logged(&mut rig).await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    say(&rig, &tasks[0], "keep this message until you can read it").await;
    let worktree = rig.worktree(&tasks[0]);
    rig.wait_for("the first wake-up to start", || {
        worktree.join(".fake-started-2").exists()
    })
    .await;
    let pending = rig.state().wake_delivery(&tasks[0]);
    std::fs::write(worktree.join(".fake-release-2"), "").unwrap();
    rig.wait_for("the wake-up budget to be exhausted", || {
        logged(&rig)
            .iter()
            .any(|row| row.event() == LogEvent::BusDown && row.fields().contains_key("wake_ups"))
    })
    .await;
    rig.stop().await.unwrap();

    assert!(
        matches!(
            pending,
            Some(agent_core::state::WakeDelivery::Pending {
                message: Some(_),
                count: 1
            })
        ),
        "the first wake-up was not saved: {pending:?}"
    );
    let Some(agent_core::state::WakeDelivery::Pending { message, count: 1 }) = pending else {
        panic!("the first wake-up was not saved: {pending:?}");
    };
    assert!(message.is_some(), "the saved delivery identifies its message");
    assert_eq!(
        rig.state().wake_delivery(&tasks[0]),
        Some(agent_core::state::WakeDelivery::Delivered { message, count: 2 })
    );
}

#[tokio::test]
async fn reading_the_message_clears_its_saved_wake_budget() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,sleep"),
            normal("claude-1", "fix-held:1"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    say(&rig, &tasks[0], "read this message").await;
    rig.wait_for("the message wake-up", || rig.runs(&tasks[0]) == 2).await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    rig.stop().await.unwrap();

    assert_eq!(rig.state().wake_delivery(&tasks[0]), None);
}

#[tokio::test]
async fn restarting_keeps_the_exhausted_budget_and_a_new_message_gets_its_own_budget() {
    let mut rig = deaf_lead().await;
    start_logged(&mut rig).await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    say(&rig, &tasks[0], "first unread message").await;
    rig.wait_for("the first exhausted budget", || {
        logged(&rig)
            .iter()
            .any(|row| row.fields().contains_key("wake_ups") && row.event() == LogEvent::BusDown)
    })
    .await;
    rig.stop().await.unwrap();

    start_logged(&mut rig).await;
    let worker = rig.worktree(&tasks[1]);
    rig.wait_for("the worker to resume after restart", || {
        worker.join(".fake-started-2").exists()
    })
    .await;
    say(&rig, &tasks[0], "second unread message").await;
    rig.wait_for("the second exhausted budget", || {
        logged(&rig)
            .iter()
            .filter(|row| row.fields().contains_key("wake_ups") && row.event() == LogEvent::BusDown)
            .count()
            >= 2
    })
    .await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    rig.stop().await.unwrap();

    assert_eq!(
        rig.runs(&tasks[0]),
        5,
        "one initial run and two wake-ups per message across restart"
    );
    let stopped: Vec<_> = logged(&rig)
        .into_iter()
        .filter(|row| row.fields().contains_key("wake_ups") && row.event() == LogEvent::BusDown)
        .collect();
    assert_eq!(
        stopped.len(),
        2,
        "each delivery stop is logged once, including across restart"
    );
    assert_ne!(
        stopped[0].fields().get("message_id"),
        stopped[1].fields().get("message_id")
    );
}

#[tokio::test]
async fn restarting_after_the_first_wake_up_allows_only_the_second_one() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,deaf-fix-held:1,deaf-sleep"),
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
    start_logged(&mut rig).await;
    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    say(&rig, &tasks[0], "are you there?").await;
    let worktree = rig.worktree(&tasks[0]);
    rig.wait_for("the first wake-up to start", || {
        worktree.join(".fake-started-2").exists()
    })
    .await;
    let pending = rig.state().wake_delivery(&tasks[0]);
    rig.stop().await.unwrap();
    assert!(
        matches!(pending, Some(agent_core::state::WakeDelivery::Pending { count: 1, .. })),
        "{pending:?}"
    );

    start_logged(&mut rig).await;
    rig.wait_for("the wake-ups to stop", || {
        logged(&rig)
            .iter()
            .any(|row| row.event() == LogEvent::BusDown && row.fields().contains_key("wake_ups"))
    })
    .await;
    let runs = rig.runs(&tasks[0]);
    rig.stop().await.unwrap();

    assert_eq!(
        runs, 4,
        "the run of its own, the first wake-up cut by the restart, its resumption and the second wake-up"
    );
}
