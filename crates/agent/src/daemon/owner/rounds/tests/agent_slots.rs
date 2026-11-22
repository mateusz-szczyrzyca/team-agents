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
