//! A worker's request file that cannot be used is logged as `worker_request_damaged` with its name, and the daemon goes on.

use agent_core::log::{LogEvent, LogLevel};
use agent_core::state::MergeMode;

use super::rig::{ONE, Rig, investigator, normal};

/// A reviewer that approves.
const APPROVE: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// The rows of `worker_request_damaged` in the history of `rig`.
fn damage_rows(rig: &Rig) -> Vec<agent_core::log::LogEntry> {
    agent_runtime::history::Reader::open(&rig.home.history_file())
        .unwrap()
        .daemon_logs()
        .unwrap()
        .into_iter()
        .filter(|row| row.event() == LogEvent::WorkerRequestDamaged)
        .collect()
}

/// Records the `worker_request_damaged` of a real damaged file — a worker of a team has a file `worker-request-1.json` that is
/// not JSON — into `observed`, after checking its row names the file.
pub(super) async fn record_worker_request_damaged(observed: &mut std::collections::BTreeSet<LogEvent>) {
    let mut rig = Rig::new(
        &[
            investigator("inv-opus", APPROVE),
            investigator("inv-r1", APPROVE),
            investigator("inv-r2", APPROVE),
            normal("claude-1", "sleep"),
            normal("glm-1", "sleep"),
        ],
        5,
        true,
    )
    .await;
    // The rig knows normal agents and investigators: the last of the normal agents is a worker of the configuration.
    let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    std::fs::write(
        rig.home.config_file(),
        config.replace(
            "normal = [\"claude-1\", \"glm-1\"]",
            "normal = [\"claude-1\"]\nworkers = [\"glm-1\"]",
        ),
    )
    .unwrap();
    rig.start_with_log_layer(agent_runtime::history::HistoryLayer::deferred(
        agent_core::log::LogLevel::Info,
        rig.history_test_clock(),
        agent_core::config::Secrets::default(),
    ))
    .await;
    let (_, tasks) = rig
        .run(&["inv-opus", "claude-1", "glm-1"], ONE, MergeMode::Autopilot, None)
        .await;
    let worker = &tasks[2];
    std::fs::create_dir_all(rig.home.task_dir(worker)).unwrap();
    std::fs::write(
        rig.home.task_dir(worker).join("worker-request-1.json"),
        "{ not the file of a request",
    )
    .unwrap();
    rig.wait_for("the damage in the history", || !damage_rows(&rig).is_empty())
        .await;
    rig.stop().await.unwrap();

    let rows = damage_rows(&rig);
    let row = &rows[0];
    assert_eq!(row.level(), LogLevel::Warn);
    assert!(
        row.fields()
            .get("file")
            .is_some_and(|file| file.to_string().contains("worker-request-1.json")),
        "{row:?}"
    );
    assert!(rig.home.task_dir(worker).join("worker-request-1.json.damaged").exists());
    observed.insert(LogEvent::WorkerRequestDamaged);
}

// Rule: a worker's request file that cannot be used is logged as `worker_request_damaged`, with the file's name.
#[tokio::test]
async fn a_damaged_file_of_a_workers_request_is_logged_with_its_name() {
    let mut observed = std::collections::BTreeSet::new();

    record_worker_request_damaged(&mut observed).await;

    assert!(observed.contains(&LogEvent::WorkerRequestDamaged));
}
