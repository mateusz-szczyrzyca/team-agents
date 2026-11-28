//! Tests for lead task shifts on minute ticks: coordinator and solo tasks rotating to the next free agent.

use std::time::Duration;

use agent_core::decide::ladder::Trouble;
use agent_core::ids::TaskId;
use agent_core::log::{LogEntry, LogEvent, LogLevel};
use agent_core::protocol::{Client, Command, GateRequest, Reply};
use agent_core::state::{ItemPhase, MergeMode, State};
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};
use crate::client::Connection;

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// Adds `table` to the configuration of `rig`, before the daemon starts.
fn configure(rig: &Rig, table: &str) {
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config = config.replace("hang_minutes = 1\n", "hang_minutes = 100000\n");
    config.push_str("[commit_reminder]\nlines = 0\ncut_after = 0\n");
    config.push_str(table);
    std::fs::write(rig.home.config_file(), config).unwrap();
}

/// The texts the bus of `rig` keeps, every file of its directory read as text.
fn bus_texts(rig: &Rig) -> String {
    let mut texts = String::new();
    let mut dirs = vec![rig.home.root().join("bus")];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if let Ok(text) = std::fs::read_to_string(&path) {
                texts.push_str(&text);
            }
        }
    }
    texts
}

/// The agent that holds `task` in `state`.
fn holder(state: &State, task: &TaskId) -> String {
    state
        .find_task(task)
        .map_or_else(String::new, |f| f.member.agent().name().to_owned())
}

// Rule: A (coordinator), B, C, lead_shift_minutes = 120, the round's pull request open: at 120 worked minutes B holds
// the coordinator's task and A B's task, both briefed shift, the bus says so; 120 minutes later C, then A again.
#[tokio::test]
async fn team_lead_shifts_in_a_circle() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "sleep,integrate,hold,sleep,hold"),
            normal("claude-2", "commit-clean,hold,sleep,hold"),
            normal("claude-3", "sleep,hold,sleep,hold"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", "verify:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    configure(&rig, "[limits]\nidle_team_minutes = 0\nlead_shift_minutes = 120\n");
    rig.wake_sleeping_teams();
    rig.start().await;

    let (_team, tasks) = rig
        .run(
            &["claude-1", "claude-2", "claude-3"],
            ONE,
            MergeMode::NoMerge,
            Some("claude-1"),
        )
        .await;

    rig.item_where("1.1", "pull request open in fix pass", |phase| {
        matches!(phase, ItemPhase::Fixing { .. })
    })
    .await;

    rig.prompt(&tasks[0], 3).await;

    let state = rig.state();
    assert_eq!(holder(&state, &tasks[0]), "claude-1");
    assert_eq!(holder(&state, &tasks[1]), "claude-2");
    assert_eq!(holder(&state, &tasks[2]), "claude-3");

    // At 120 worked minutes, B (claude-2) holds coordinator task, A (claude-1) holds member task.
    rig.advance_history_days(1);

    let state = rig
        .state_where("claude-2 holds coordinator task", |state| {
            holder(state, &tasks[0]) == "claude-2"
        })
        .await;
    assert_eq!(holder(&state, &tasks[1]), "claude-1");

    let p0 = rig.prompt(&tasks[0], 4).await;
    assert!(p0.contains("`shift`:"), "{p0}");
    assert!(p0.contains("You are agent **claude-2**"), "{p0}");

    let bus = bus_texts(&rig);
    assert!(
        bus.contains("Coordination shifted: claude-2 holds coordinator task"),
        "{bus}"
    );

    // 120 minutes later C, then A again.
    rig.advance_history_days(1);

    let state = rig
        .state_where("claude-3 holds coordinator task", |state| {
            holder(state, &tasks[0]) == "claude-3"
        })
        .await;
    assert_eq!(holder(&state, &tasks[2]), "claude-2");
    assert_eq!(holder(&state, &tasks[1]), "claude-1");

    rig.prompt(&tasks[0], 5).await;

    rig.advance_history_days(1);

    let state = rig
        .state_where("claude-1 holds coordinator task again", |state| {
            holder(state, &tasks[0]) == "claude-1"
        })
        .await;
    assert_eq!(holder(&state, &tasks[1]), "claude-3");
    assert_eq!(holder(&state, &tasks[2]), "claude-2");

    rig.stop().await.unwrap();
}

// Rule: no pull request at 120 minutes and round_minutes = 120: round_overdue acts, no shift.
#[tokio::test]
async fn round_without_pr_is_left_to_round_minutes() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "hold"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    configure(
        &rig,
        "[replacement]\nagents = [\"claude-2\"]\n[limits]\nround_minutes = 120\nlead_shift_minutes = 120\n",
    );
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "claude-1 to work on the item", |phase| {
        matches!(phase, ItemPhase::InProgress { .. })
    })
    .await;

    rig.advance_history_days(1);

    let state = rig
        .item_where("1.1", "merged after the round_overdue hand-over", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;

    let lead = state.find_task(&tasks[0]).unwrap().member;
    assert_eq!(lead.agent().name(), "claude-2");
    assert_eq!(lead.renewals().unwrap().reason(), Trouble::RoundOverdue);

    rig.stop().await.unwrap();
}

// Rule: A's agent gate is running: the shift waits for its end, then happens.
#[tokio::test]
async fn shift_waits_for_the_gate() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,hold"),
            normal("gemini-1", "hold"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", "verify:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    let gate_marker = rig.home.root().join("gate-release");
    let script = format!("while [ ! -f {} ]; do sleep 0.05; done", gate_marker.display());
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    let old_gate = "gate = [\"sh\", \"-c\", \"test ! -e gate-fails\"]";
    let new_gate = format!("gate = [\"sh\", \"-c\", {:?}]", script);
    std::fs::write(rig.home.config_file(), config.replace(old_gate, &new_gate)).unwrap();
    configure(
        &rig,
        "[replacement]\nagents = [\"claude-1\", \"gemini-1\"]\n[limits]\nidle_team_minutes = 0\nlead_shift_minutes = 120\n",
    );
    rig.wake_sleeping_teams();
    rig.start().await;

    let (_team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;

    rig.item_where("1.1", "pull request open in fix pass", |phase| {
        matches!(phase, ItemPhase::Fixing { .. })
    })
    .await;

    let socket = rig.home.socket_file();
    let gate_task = tasks[0].clone();
    let gate_handle = tokio::spawn(async move {
        Connection::connect(&socket)
            .await
            .unwrap()
            .request(Client::Cli, Command::Gate(GateRequest { task: gate_task }))
            .await
    });

    tokio::time::sleep(Duration::from_millis(200)).await;

    rig.advance_history_days(1);

    tokio::time::sleep(Duration::from_millis(200)).await;

    let state = rig.state();
    assert_eq!(
        holder(&state, &tasks[0]),
        "claude-1",
        "shift should wait while gate is running"
    );

    std::fs::write(&gate_marker, "ok").unwrap();
    let gate_reply = gate_handle.await.unwrap().unwrap();
    assert!(matches!(gate_reply, Reply::GateRecorded { .. }), "{gate_reply:?}");

    let state = rig
        .state_where("gemini-1 holds task after gate finished", |state| {
            holder(state, &tasks[0]) == "gemini-1"
        })
        .await;
    assert_eq!(holder(&state, &tasks[0]), "gemini-1");

    rig.stop().await.unwrap();
}

// Rule: B and C busy: no change, shift_no_free_agent logged once; when B is free, the next tick trades.
#[tokio::test]
async fn none_free_logs_once_then_trades() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "sleep,integrate,hold"),
            normal("claude-2", "commit-clean,hold"),
            normal("claude-3", "hold"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", "verify:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    configure(
        &rig,
        "[replacement]\nagents = [\"claude-1\", \"claude-2\", \"claude-3\"]\n[limits]\nidle_team_minutes = 0\nlead_shift_minutes = 120\n[log]\nlevel = \"debug\"\n",
    );
    rig.wake_sleeping_teams();
    rig.start_with_log_layer(agent_runtime::history::HistoryLayer::deferred(
        LogLevel::Debug,
        rig.history_test_clock(),
        agent_core::config::Secrets::default(),
    ))
    .await;

    let (_, tasks) = rig
        .run(
            &["claude-1", "claude-2", "claude-3"],
            ONE,
            MergeMode::NoMerge,
            Some("claude-1"),
        )
        .await;

    rig.item_where("1.1", "pull request open in fix pass", |phase| {
        matches!(phase, ItemPhase::Fixing { .. })
    })
    .await;

    rig.prompt(&tasks[0], 3).await;

    rig.status_is(&tasks[1], TaskStatus::Waiting).await;
    rig.resume(&tasks[1]).await;
    rig.prompt(&tasks[1], 2).await;
    rig.status_is(&tasks[1], TaskStatus::Running).await;
    rig.status_is(&tasks[2], TaskStatus::Running).await;

    rig.advance_history_days(1);

    rig.wait_for("shift_no_free_agent logged", || {
        let logs: Vec<LogEntry> = agent_runtime::history::Reader::open(&rig.home.history_file())
            .unwrap()
            .daemon_logs()
            .unwrap();
        logs.iter().any(|entry| entry.event() == LogEvent::ShiftNoFreeAgent)
    })
    .await;

    rig.state_where("both candidates remain busy after the no-free tick", |state| {
        [1, 2].into_iter().all(|index| {
            state.find_task(&tasks[index]).is_some_and(|found| {
                found.member.status() == TaskStatus::Running && found.member.current_run().is_some()
            })
        })
    })
    .await;
    let logs: Vec<LogEntry> = agent_runtime::history::Reader::open(&rig.home.history_file())
        .unwrap()
        .daemon_logs()
        .unwrap();
    let no_free_count = logs
        .iter()
        .filter(|entry| entry.event() == LogEvent::ShiftNoFreeAgent)
        .count();
    assert_eq!(no_free_count, 1, "shift_no_free_agent should be logged once per shift");

    let state = rig.state();
    assert_eq!(holder(&state, &tasks[0]), "claude-1");

    std::fs::write(rig.worktree(&tasks[1]).join(".fake-continue"), "ok").unwrap();

    let state = rig
        .state_where("claude-2 takes coordinator task once free", |state| {
            holder(state, &tasks[0]) == "claude-2"
        })
        .await;
    assert_eq!(holder(&state, &tasks[0]), "claude-2");

    rig.stop().await.unwrap();
}

const CHANGES_ONCE: &str = "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: fixed";

// Rule: a solo task of claude-1 with [replacement] agents = ["claude-1", "gemini-1"]: handed to gemini-1,
// and back after another 120 minutes.
#[tokio::test]
async fn solo_task_shifts_and_back() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,hold"),
            normal("gemini-1", "hold"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", "verify:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    configure(
        &rig,
        "[replacement]\nagents = [\"claude-1\", \"gemini-1\"]\n[limits]\nidle_team_minutes = 0\nlead_shift_minutes = 120\n",
    );
    rig.wake_sleeping_teams();
    rig.start().await;

    let (_team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;

    rig.item_where("1.1", "pull request open in fix pass", |phase| {
        matches!(phase, ItemPhase::Fixing { .. })
    })
    .await;

    assert_eq!(holder(&rig.state(), &tasks[0]), "claude-1");

    rig.advance_history_days(1);

    let state = rig
        .state_where("handed to gemini-1", |state| holder(state, &tasks[0]) == "gemini-1")
        .await;
    assert_eq!(holder(&state, &tasks[0]), "gemini-1");

    let p0 = rig.prompt(&tasks[0], 3).await;
    assert!(p0.contains("`shift`:"), "{p0}");
    assert!(p0.contains("gemini-1"), "{p0}");

    rig.advance_history_days(1);

    let state = rig
        .state_where("handed back to claude-1", |state| {
            holder(state, &tasks[0]) == "claude-1"
        })
        .await;
    assert_eq!(holder(&state, &tasks[0]), "claude-1");

    rig.stop().await.unwrap();
}

// Rule: shift ignores [replacement] replacements = 0: history and log hold shift.
#[tokio::test]
async fn shift_ignores_replacements_zero() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,hold"),
            normal("gemini-1", "hold"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", "verify:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        true,
    )
    .await;
    configure(
        &rig,
        "[replacement]\nagents = [\"claude-1\", \"gemini-1\"]\nreplacements = 0\n[limits]\nidle_team_minutes = 0\nlead_shift_minutes = 120\n[log]\nlevel = \"debug\"\n",
    );
    rig.wake_sleeping_teams();
    rig.start_with_log_layer(agent_runtime::history::HistoryLayer::deferred(
        LogLevel::Debug,
        rig.history_test_clock(),
        agent_core::config::Secrets::default(),
    ))
    .await;

    let (_team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;

    rig.item_where("1.1", "pull request open in fix pass", |phase| {
        matches!(phase, ItemPhase::Fixing { .. })
    })
    .await;

    rig.advance_history_days(1);

    let state = rig
        .state_where("handed to gemini-1 despite replacements = 0", |state| {
            holder(state, &tasks[0]) == "gemini-1"
        })
        .await;
    assert_eq!(holder(&state, &tasks[0]), "gemini-1");

    rig.wait_for("history and log hold shift", || {
        let logs: Vec<LogEntry> = agent_runtime::history::Reader::open(&rig.home.history_file())
            .unwrap()
            .daemon_logs()
            .unwrap();
        let has_handover = logs.iter().any(|entry| {
            entry.event() == LogEvent::TaskHandedOver && entry.fields().get("reason").is_some_and(|r| r == "shift")
        });
        let has_decision = logs.iter().any(|entry| {
            entry.event() == LogEvent::DecisionTaken && entry.fields().get("decision").is_some_and(|d| d == "shift")
        });
        has_handover && has_decision
    })
    .await;

    rig.stop().await.unwrap();
}

// A shift keeps idle member tasks idle and moves the bus's coordinator with the task.
#[tokio::test]
async fn shift_keeps_idle_members_waiting_and_moves_bus_lead() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "sleep,integrate,hold"),
            normal("claude-2", "commit-clean,hold"),
            normal("claude-3", "sleep,hold"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    configure(&rig, "[limits]\nidle_team_minutes = 0\nlead_shift_minutes = 120\n");
    rig.wake_sleeping_teams();
    rig.start().await;
    let (team, tasks) = rig
        .run(
            &["claude-1", "claude-2", "claude-3"],
            ONE,
            MergeMode::NoMerge,
            Some("claude-1"),
        )
        .await;
    rig.item_where("1.1", "pull request open", |phase| {
        matches!(phase, ItemPhase::Fixing { .. })
    })
    .await;
    rig.prompt(&tasks[0], 3).await;
    rig.advance_history_days(1);
    rig.state_where("coordinator task traded", |state| {
        holder(state, &tasks[0]) == "claude-2"
    })
    .await;
    rig.prompt(&tasks[0], 4).await;
    let state = rig.state();
    assert_eq!(state.find_task(&tasks[1]).unwrap().member.status(), TaskStatus::Waiting);
    assert_eq!(state.find_task(&tasks[2]).unwrap().member.status(), TaskStatus::Waiting);
    let bus: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(rig.home.root().join("bus/state.json")).unwrap()).unwrap();
    assert_eq!(bus["teams"][team.as_str()]["lead"], "claude-2");
    rig.stop().await.unwrap();
}
