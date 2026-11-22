//! A queued task whose agent runs another task at its limit waits for that run, and the daemon logs the wait once as
//! `task_waits_for_agent`, naming the run it waits for.

use agent_core::ids::TaskId;
use agent_core::log::LogEvent;
use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;

use super::rig::{ONE, Rig, investigator, normal};

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// The rows of `task_waits_for_agent` in the history of `rig`.
fn wait_rows(rig: &Rig) -> Vec<agent_core::log::LogEntry> {
    agent_runtime::history::Reader::open(&rig.home.history_file())
        .unwrap()
        .daemon_logs()
        .unwrap()
        .into_iter()
        .filter(|row| row.event() == LogEvent::TaskWaitsForAgent)
        .collect()
}

/// Records the `task_waits_for_agent` of a real wait — the second task of claude-1 behind the first — into `observed`,
/// after checking its row names the first task.
pub(super) async fn record_task_waits_for_agent(observed: &mut std::collections::BTreeSet<LogEvent>) {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "hold"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        false,
    )
    .await;
    rig.start_with_log_layer(agent_runtime::history::HistoryLayer::deferred(
        agent_core::log::LogLevel::Info,
        rig.history_test_clock(),
        agent_core::config::Secrets::default(),
    ))
    .await;
    let (_, first) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&first[0], TaskStatus::Running).await;
    let (_, second) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    // The loop of the daemon looks at the queue after every message: the wait is logged before the stop is served. The
    // history is read after the stop, which writes what the layer still holds.
    rig.status_is(&second[0], TaskStatus::Queued).await;
    rig.wait_for("the wait in the history", || !wait_rows(&rig).is_empty())
        .await;
    rig.stop().await.unwrap();

    let rows = wait_rows(&rig);
    assert_eq!(rows.len(), 1, "{rows:?}");
    let row = &rows[0];
    assert_eq!(row.level(), agent_core::log::LogLevel::Info);
    let second: &TaskId = &second[0];
    assert_eq!(row.task(), Some(second));
    assert_eq!(
        row.fields().get("holder_task"),
        Some(&serde_json::json!(first[0].as_str()))
    );
    observed.insert(LogEvent::TaskWaitsForAgent);
}

// Rule: a task that waits for its agent is logged as `task_waits_for_agent`, with the run it waits for.
#[tokio::test]
async fn a_wait_for_the_agent_is_logged_with_the_run_it_waits_for() {
    let mut observed = std::collections::BTreeSet::new();

    record_task_waits_for_agent(&mut observed).await;

    assert!(observed.contains(&LogEvent::TaskWaitsForAgent));
}

/// All text files in the `bus/` directory of `rig`.
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

#[tokio::test]
async fn a_busy_member_in_a_team_is_handed_to_the_least_loaded_cli_tells_the_bus_and_delivers_the_round() {
    use agent_core::state::ItemPhase;

    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate"),
            normal("claude-1", "hold"),
            normal("claude-2", "hold"),
            normal("claude-3", "work"),
            normal("codex-1", "work"),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
        ],
        5,
        true,
    )
    .await;
    // Give codex-1 the `codex` CLI binary so `claude` carries two runs and `codex` carries zero.
    let config_text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    let team_cli = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/team-cli.sh");
    let updated = config_text
        .replace(
            "[agents.codex-1]\ncli = \"claude\"",
            "[agents.codex-1]\ncli = \"codex\"",
        )
        .replace(
            "[binaries]\n",
            &format!("[binaries]\ncodex = \"{}\"\n", team_cli.display()),
        )
        + "\n[replacement]\nagents = [\"claude-3\", \"codex-1\"]\nbusy_after = 5\n";
    std::fs::write(rig.home.config_file(), updated).unwrap();

    rig.start().await;
    let (_, busy_a) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let (_, busy_c) = rig.run(&["claude-2"], ONE, MergeMode::NoMerge, None).await;
    rig.status_is(&busy_a[0], TaskStatus::Running).await;
    rig.status_is(&busy_c[0], TaskStatus::Running).await;

    let (_, team_b_tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    let member_task = team_b_tasks[1].clone();
    rig.state_where("member task is queued and waiting for claude-1", |state| {
        let member = state.find_task(&member_task).unwrap().member;
        member.status() == TaskStatus::Queued && member.waiting_since().is_some()
    })
    .await;

    // Advance the injected clock past busy_after without real sleep.
    rig.advance_history_days(1);

    let state = rig
        .item_where("1.1", "delivered after busy hand-over to codex-1", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    let prompt = rig.prompt(&member_task, 1).await;
    let bus = bus_texts(&rig);

    // Release claude-1 in team A and verify it is not woken for the task it lost in team B.
    std::fs::write(rig.worktree(&busy_a[0]).join(".fake-continue"), "").unwrap();
    rig.status_is(&busy_a[0], TaskStatus::Done).await;
    let after_free = rig.state();
    rig.stop().await.unwrap();

    let member = state.find_task(&member_task).unwrap().member;
    assert_eq!(member.agent().name(), "codex-1", "least-loaded CLI codex is chosen");
    assert_eq!(member.past_holders().len(), 1);
    assert_eq!(member.past_holders()[0].agent.name(), "claude-1");
    assert!(
        prompt.starts_with("# A NEW SESSION ON A TASK THAT WAS ALREADY STARTED"),
        "{prompt}"
    );
    assert!(prompt.contains("`busy`"), "{prompt}");
    assert!(
        bus.contains("is held by codex-1 now: claude-1 is busy in another team."),
        "the bus tells the team who holds the item now: {bus}"
    );
    assert_eq!(
        after_free.find_task(&member_task).unwrap().member.runs().len(),
        1,
        "the first holder is not woken for the task when it becomes free"
    );
}

#[tokio::test]
async fn a_restart_during_the_busy_hand_over_keeps_the_new_holder_and_briefing() {
    use agent_core::decide::ladder::Trouble;
    use agent_core::state::{ItemPhase, Participant, Timestamp};

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
    let mut config_text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config_text.push_str("\n[replacement]\nagents = [\"claude-2\"]\nbusy_after = 300\n");
    std::fs::write(rig.home.config_file(), config_text).unwrap();

    rig.start().await;
    let (_, busy_a) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.status_is(&busy_a[0], TaskStatus::Running).await;
    let (team_b, tasks_b) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let waiting_task = tasks_b[0].clone();
    rig.status_is(&waiting_task, TaskStatus::Queued).await;
    rig.stop().await.unwrap();

    // Persist a busy hand-over that has just begun before its first run starts, then restart the daemon.
    let mut state = rig.state();
    let at: Timestamp = "2026-11-22T18:00:00Z".parse().unwrap();
    let claude_2: agent_core::ids::AgentName = "claude-2".parse().unwrap();
    state
        .hand_over_task(&waiting_task, Participant::Normal(claude_2.clone()), at)
        .unwrap();
    let branch = match state
        .team(&team_b)
        .unwrap()
        .item("1.1".parse().unwrap())
        .unwrap()
        .phase()
    {
        ItemPhase::InProgress { branch, .. } => branch.clone(),
        other => panic!("unexpected phase: {other:?}"),
    };
    state
        .team_mut(&team_b)
        .unwrap()
        .item_mut("1.1".parse().unwrap())
        .unwrap()
        .reassign(claude_2, branch)
        .unwrap();
    state.begin_holder_session(&waiting_task, Trouble::Busy, at).unwrap();
    std::fs::write(rig.home.state_file(), state.to_json().unwrap()).unwrap();

    rig.start().await;
    let finished = rig
        .item_where("1.1", "delivered by claude-2 after restart during hand-over", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    let prompt = rig.prompt(&waiting_task, 1).await;
    rig.stop().await.unwrap();

    let member = finished.find_task(&waiting_task).unwrap().member;
    assert_eq!(member.agent().name(), "claude-2");
    assert_eq!(member.past_holders().len(), 1);
    assert_eq!(member.past_holders()[0].agent.name(), "claude-1");
    assert!(prompt.contains("`busy`"), "{prompt}");
}
