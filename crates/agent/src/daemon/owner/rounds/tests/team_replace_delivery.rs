//! A replacement starts with its bus membership and receives messages in that run.

use std::sync::Arc;

use agent_core::config::Secrets;
use agent_core::history::MaxTextBytes;
use agent_core::log::{LogEvent, LogLevel};
use agent_runtime::clock::{SharedClock, SystemClock};
use agent_runtime::history::{DEFAULT_QUEUE, History, HistoryDb, HistoryLayer, Problem, Reader, Reporter, Setup};

use agent_core::decide::ladder::Trouble;
use agent_core::state::{ItemPhase, MergeMode};
use agent_core::status::TaskStatus;
use agent_runtime::agentbus::{AgentBus, Outgoing, Recipient};

use super::rig::{ONE, Rig, investigator, normal};

/// Keeps history failures visible in a test that inspects the daemon log.
#[derive(Debug)]
struct TestReporter;

impl Reporter for TestReporter {
    fn report(&self, problem: &Problem) {
        panic!("history failure: {problem:?}");
    }
}

/// Verifies the roster at launch and message delivery in the replacement process.
async fn verify_membership_and_deliver_messages(
    rig: &Rig,
    task: &agent_core::ids::TaskId,
    team: agent_core::ids::TeamId,
) {
    let current_bus = AgentBus::new(
        super::super::fixture::agentbus_binary()
            .to_str()
            .unwrap()
            .parse()
            .unwrap(),
        rig.home.root().to_str().unwrap().parse().unwrap(),
    );
    let roster = current_bus.team_get(&team).await.unwrap();
    assert_eq!(
        roster.members.keys().map(ToString::to_string).collect::<Vec<_>>(),
        ["claude-2", "inv-lead"]
    );
    let membership = rig.worktree(task).join(".fake-replacement-members");
    rig.wait_for("replacement CLI membership snapshot", || {
        std::fs::read_to_string(&membership).is_ok_and(|text| serde_json::from_str::<serde_json::Value>(&text).is_ok())
    })
    .await;
    let members: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&membership).unwrap()).unwrap();
    assert!(members["members"].get("claude-2").is_some(), "{members}");
    assert!(members["members"].get("claude-1").is_none(), "{members}");
    let bus = AgentBus::new(
        super::super::fixture::agentbus_binary()
            .to_str()
            .unwrap()
            .parse()
            .unwrap(),
        rig.home.root().to_str().unwrap().parse().unwrap(),
    );
    bus.send(
        &"inv-lead".parse().unwrap(),
        &Recipient::Member {
            team,
            agent: "claude-2".parse().unwrap(),
        },
        &Outgoing::Chat {
            text: "Continue the café item".to_owned(),
        },
    )
    .await
    .unwrap();
    let inbox = rig.worktree(task).join(".fake-replacement-inbox");
    rig.wait_for("the lead message in the replacement run", || {
        std::fs::read_to_string(&inbox).is_ok_and(|text| text.contains("Continue the café item"))
    })
    .await;
    rig.wait_for("the replacement's Serena reminder", || {
        super::serena_delivery_tests::chats_from_the_harness(rig)
            .iter()
            .any(|text| text.contains("Serena"))
    })
    .await;
    rig.wait_for("Serena reminder read by the replacement", || {
        std::fs::read_to_string(&inbox).is_ok_and(|text| text.contains("Serena"))
    })
    .await;
}

#[tokio::test]
async fn overdue_member_has_bus_membership_before_launch_and_receives_lead_and_serena_messages() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "hold"),
            normal("claude-2", "replacement-inbox"),
            investigator("inv-lead", "sleep"),
            investigator("inv-review-1", "sleep"),
            investigator("inv-review-2", "sleep"),
        ],
        5,
        true,
    )
    .await;
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        format!(
            "{}\n[replacement]\nagents = [\"claude-2\"]\n[limits]\nround_minutes = 120\n",
            config.replace("[binaries]\n", "[binaries]\nserena = \"serena\"\n")
        ),
    )
    .unwrap();
    let log_dir = tempfile::tempdir().unwrap();
    let log_path = log_dir.path().join("history.db");
    let started = History::start(
        HistoryDb::open(&log_path, MaxTextBytes::default()).unwrap(),
        Setup {
            reporter: Arc::new(TestReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    rig.start_with_log_layer(HistoryLayer::new(
        LogLevel::Info,
        SharedClock::new(SystemClock),
        Secrets::default(),
        Some(started.history.clone()),
    ))
    .await;
    let (team, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::NoMerge, Some("inv-lead"))
        .await;
    let state = rig
        .item_where("1.1", "the original member works on the item", |phase| {
            matches!(phase, ItemPhase::InProgress { .. })
        })
        .await;
    let task = tasks
        .iter()
        .find(|task| {
            state
                .find_task(task)
                .is_some_and(|found| found.member.agent().name() == "claude-1")
        })
        .unwrap()
        .clone();
    rig.status_is(&task, TaskStatus::Running).await;
    rig.advance_history_days(1);
    rig.state_where("the task is handed over", |state| {
        state
            .find_task(&task)
            .is_some_and(|found| found.member.agent().name() == "claude-2" && found.member.runs().len() == 2)
    })
    .await;
    verify_membership_and_deliver_messages(&rig, &task, team).await;
    let state = rig
        .state_where("the replacement's running history", |state| {
            state
                .find_task(&task)
                .is_some_and(|found| found.member.agent().name() == "claude-2" && found.member.runs().len() == 2)
        })
        .await;
    rig.stop().await.unwrap();
    started.writer.close().unwrap();
    let errors: Vec<_> = Reader::open(&log_path)
        .unwrap()
        .daemon_logs()
        .unwrap()
        .into_iter()
        .filter(|row| matches!(row.event(), LogEvent::BusDown | LogEvent::DaemonTaskFailed))
        .collect();
    assert_eq!(errors.len(), 0, "{errors:?}");
    assert_eq!(
        state.find_task(&task).unwrap().member.renewals().unwrap().reason(),
        Trouble::RoundOverdue
    );
}

#[tokio::test]
async fn storage_failure_blocks_the_replacement_run_until_a_later_pass_succeeds() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "hold"),
            normal("claude-2", "hold"),
            investigator("inv-lead", "sleep"),
            investigator("inv-review-1", "sleep"),
            investigator("inv-review-2", "sleep"),
        ],
        5,
        true,
    )
    .await;
    let real_bus = super::super::fixture::agentbus_binary();
    let flaky_bus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/flaky-agentbus.sh");
    std::fs::write(rig.home.root().join("real-agentbus"), real_bus.to_str().unwrap()).unwrap();
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        format!(
            "{}\n[replacement]\nagents = [\"claude-2\"]\n[limits]\nround_minutes = 120\n",
            config.replace(
                &format!("agentbus = \"{}\"", real_bus.display()),
                &format!("agentbus = \"{}\"", flaky_bus.display()),
            )
        ),
    )
    .unwrap();
    let log_dir = tempfile::tempdir().unwrap();
    let log_path = log_dir.path().join("history.db");
    let started = record_team_replace_logs(&mut rig, &log_path).await;
    let (_team, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::NoMerge, Some("inv-lead"))
        .await;
    let state = rig
        .item_where("1.1", "the member starts the item", |phase| {
            matches!(phase, ItemPhase::InProgress { .. })
        })
        .await;
    let task = tasks
        .iter()
        .find(|task| {
            state
                .find_task(task)
                .is_some_and(|found| found.member.agent().name() == "claude-1")
        })
        .unwrap()
        .clone();
    rig.status_is(&task, TaskStatus::Running).await;
    let failure = rig.home.root().join("flaky-bus-down");
    std::fs::write(&failure, "").unwrap();
    rig.advance_history_days(1);
    let state = rig
        .state_where("the replacement is saved", |state| {
            state.find_task(&task).is_some_and(|found| {
                found.member.agent().name() == "claude-2"
                    && (found.member.runs().len() == 2
                        || std::fs::read_to_string(rig.home.root().join("flaky-bus-refused"))
                            .is_ok_and(|text| text.lines().any(|line| line == "team-replace")))
            })
        })
        .await;
    assert_eq!(state.find_task(&task).unwrap().member.runs().len(), 1);
    rig.wait_for("team-replace refusal", || {
        std::fs::read_to_string(rig.home.root().join("flaky-bus-refused"))
            .is_ok_and(|text| text.lines().any(|line| line == "team-replace"))
    })
    .await;
    let state = rig
        .state_where("the failed replacement remains pending", |state| {
            !state.pending_team_replaces().is_empty()
        })
        .await;
    assert_eq!(state.find_task(&task).unwrap().member.runs().len(), 1);
    std::fs::remove_file(failure).unwrap();
    let state = rig
        .state_where("the retry permits the new run", |state| {
            state
                .find_task(&task)
                .is_some_and(|found| found.member.agent().name() == "claude-2" && found.member.runs().len() == 2)
        })
        .await;
    assert!(state.pending_team_replaces().is_empty());
    rig.stop().await.unwrap();
    started.writer.close().unwrap();
    let rows = Reader::open(&log_path).unwrap().daemon_logs().unwrap();
    assert!(
        rows.iter().any(|row| row.event() == LogEvent::BusDown
            && row.fields().get("operation") == Some(&serde_json::json!("team_replace"))),
        "{rows:?}"
    );
}

/// Records structured daemon logs without changing the rig's process behavior.
async fn record_team_replace_logs(rig: &mut Rig, path: &std::path::Path) -> agent_runtime::history::Started {
    let started = History::start(
        HistoryDb::open(path, MaxTextBytes::default()).unwrap(),
        Setup {
            reporter: Arc::new(TestReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    rig.start_with_log_layer(HistoryLayer::new(
        LogLevel::Info,
        SharedClock::new(SystemClock),
        Secrets::default(),
        Some(started.history.clone()),
    ))
    .await;
    started
}
