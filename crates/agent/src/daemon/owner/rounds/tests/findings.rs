//! Findings are delivered once per task, including after restarting the daemon.
use std::fmt::Write as _;

use super::rig::{ONE, Rig, git, investigator, normal};
use agent_core::ids::TaskId;
use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;
use serde_json::json;

fn write_finding(rig: &Rig, task: &TaskId, resolved: bool) {
    let directory = rig.home.root().join("bus/notebooks/example");
    std::fs::create_dir_all(&directory).unwrap();
    let record = json!({"op":"add","id":"F-1","ts":"2099-01-01T00:00:00Z","author":"claude-1","task":task.as_str(),"kind":"trap","paths":["src/lib.rs"],"text":"Use the checked helper"});
    let mut lines = format!("{record}\n");
    if resolved {
        writeln!(lines, "{}", json!({"op":"resolve","id":"F-1","ts":"2099-01-01T00:00:01Z","author":"other","task":task.as_str(),"reason":"fixed"})).unwrap();
    }
    std::fs::write(directory.join("findings.jsonl"), lines).unwrap();
}

async fn rig() -> Rig {
    let rig = Rig::new(
        &[
            investigator("inv-1", "sleep"),
            investigator("inv-2", "sleep"),
            normal("claude-1", "sleep,sleep,sleep,sleep"),
            normal("codex-1", "sleep,sleep,sleep,sleep"),
        ],
        5,
        false,
    )
    .await;
    std::fs::create_dir(rig.repo.join("src")).unwrap();
    std::fs::write(rig.repo.join("src/lib.rs"), "base").unwrap();
    git(&rig.repo, &["add", "src/lib.rs"]);
    git(&rig.repo, &["commit", "-m", "Source"]);
    git(&rig.repo, &["push", "origin", "main"]);
    rig
}

#[tokio::test]
async fn findings_follow_changed_files_once_and_survive_restart_excluding_own_task() {
    let mut rig = rig().await;
    rig.start().await;
    let (_, a) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&a[0], TaskStatus::Waiting).await;
    let (_, b) = rig.run(&["codex-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&b[0], TaskStatus::Waiting).await;
    std::fs::write(rig.worktree(&b[0]).join("src/lib.rs"), "changed").unwrap();
    std::fs::write(rig.worktree(&a[0]).join("src/lib.rs"), "changed").unwrap();
    write_finding(&rig, &a[0], false);
    rig.resume(&b[0]).await;
    let prompt = rig.prompt(&b[0], 2).await;
    rig.status_is(&b[0], TaskStatus::Waiting).await;
    assert!(
        prompt.contains("## Findings of other agents on the files of your work"),
        "{prompt}"
    );
    assert!(
        prompt.contains("- F-1 [trap] src/lib.rs — Use the checked helper (claude-1,"),
        "{prompt}"
    );
    let shown: serde_json::Value =
        serde_json::from_slice(&std::fs::read(rig.home.task_dir(&b[0]).join("findings-shown.json")).unwrap()).unwrap();
    assert_eq!(shown, json!({"shown":["F-1"]}));
    rig.resume(&a[0]).await;
    let own = rig.prompt(&a[0], 2).await;
    rig.status_is(&a[0], TaskStatus::Waiting).await;
    assert!(!own.contains("## Findings of other agents"), "{own}");
    rig.resume(&b[0]).await;
    let next = rig.prompt(&b[0], 3).await;
    rig.status_is(&b[0], TaskStatus::Waiting).await;
    assert!(!next.contains("## Findings of other agents"), "{next}");
    rig.stop().await.unwrap();
    rig.start().await;
    rig.resume(&b[0]).await;
    let restarted = rig.prompt(&b[0], 4).await;
    assert!(!restarted.contains("## Findings of other agents"), "{restarted}");
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn resolved_findings_are_not_delivered_on_item_paths() {
    let mut rig = rig().await;
    rig.start().await;
    let (_, a) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&a[0], TaskStatus::Waiting).await;
    write_finding(&rig, &a[0], true);
    let text = ONE.replace("Do the first thing in the repository.", "Change `src/lib.rs`.");
    let (_, b) = rig.run(&["codex-1"], &text, MergeMode::Autopilot, None).await;
    let prompt = rig.prompt(&b[0], 1).await;
    rig.stop().await.unwrap();
    assert!(!prompt.contains("## Findings of other agents"), "{prompt}");
}

#[tokio::test]
async fn open_findings_are_delivered_on_existing_item_paths() {
    let mut rig = rig().await;
    rig.start().await;
    let (_, a) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&a[0], TaskStatus::Waiting).await;
    write_finding(&rig, &a[0], false);
    let text = ONE.replace("Do the first thing in the repository.", "Change `src/lib.rs`.");
    let (_, b) = rig.run(&["codex-1"], &text, MergeMode::Autopilot, None).await;
    let prompt = rig.prompt(&b[0], 1).await;
    rig.stop().await.unwrap();
    assert!(prompt.contains("## Findings of other agents"), "{prompt}");
}

/// Exercises both production findings events for the exhaustive daemon log contract.
pub(super) async fn record_findings_events(observed: &mut std::collections::BTreeSet<agent_core::log::LogEvent>) {
    use agent_core::config::Secrets;
    use agent_core::log::{LogEvent, LogLevel};
    use agent_runtime::history::{HistoryLayer, Reader};
    use std::os::unix::fs::PermissionsExt;

    for fails in [false, true] {
        let mut rig = rig().await;
        if fails {
            let cfg = std::fs::read_to_string(rig.home.config_file()).unwrap();
            let line = cfg.lines().find(|line| line.starts_with("agentbus = ")).unwrap();
            let original = line.strip_prefix("agentbus = \"").unwrap().strip_suffix('"').unwrap();
            let wrapper = rig.home.root().join("findings-bus");
            std::fs::write(
                &wrapper,
                format!("#!/bin/sh\nif [ \"$1\" = findings ]; then exit 1; fi\nexec '{original}' \"$@\"\n"),
            )
            .unwrap();
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::write(
                rig.home.config_file(),
                cfg.replace(line, &format!("agentbus = \"{}\"", wrapper.display())),
            )
            .unwrap();
        }
        let layer = HistoryLayer::deferred(LogLevel::Info, rig.history_test_clock(), Secrets::default());
        rig.start_with_log_layer(layer).await;
        let author: TaskId = "task-20261120-000000-aaaa".parse().unwrap();
        write_finding(&rig, &author, false);
        let text = ONE.replace("Do the first thing in the repository.", "Change `src/lib.rs`.");
        let (team, tasks) = rig.run(&["codex-1"], &text, MergeMode::Autopilot, None).await;
        let prompt = rig.prompt(&tasks[0], 1).await;
        rig.status_is(&tasks[0], TaskStatus::Waiting).await;
        rig.stop().await.unwrap();
        assert_eq!(prompt.contains("## Findings of other agents"), !fails);
        let event = if fails {
            LogEvent::FindingsUnavailable
        } else {
            LogEvent::FindingsShown
        };
        let logs = Reader::open(&rig.home.history_file()).unwrap().daemon_logs().unwrap();
        let row = logs
            .iter()
            .find(|row| row.event() == event)
            .expect("production findings event");
        assert_eq!(row.level(), event.level());
        assert_eq!(row.task(), Some(&tasks[0]));
        assert_eq!(row.team(), Some(&team));
        if !fails {
            assert_eq!(row.fields().get("ids"), Some(&json!("[\"F-1\"]")));
            assert_eq!(row.fields().get("scores"), Some(&json!("[3]")));
        }
        observed.insert(event);
    }
}

#[tokio::test]
async fn findings_delivery_and_bus_failure_emit_scoped_events_without_blocking_runs() {
    let mut observed = std::collections::BTreeSet::new();
    record_findings_events(&mut observed).await;
    assert_eq!(
        observed,
        [
            agent_core::log::LogEvent::FindingsShown,
            agent_core::log::LogEvent::FindingsUnavailable
        ]
        .into_iter()
        .collect()
    );
}

#[tokio::test]
async fn findings_without_open_records_do_not_discover_git_paths() {
    finding_path_failure_scope(false).await;
}

#[tokio::test]
async fn findings_path_failures_keep_task_and_team_scope() {
    finding_path_failure_scope(true).await;
}

async fn finding_path_failure_scope(has_finding: bool) {
    use agent_core::config::Secrets;
    use agent_core::log::{LogEvent, LogLevel};
    use agent_runtime::history::{HistoryLayer, Reader};

    let mut rig = rig().await;
    let layer = HistoryLayer::deferred(LogLevel::Info, rig.history_test_clock(), Secrets::default());
    rig.start_with_log_layer(layer).await;
    let (team, tasks) = rig.run(&["codex-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    git(
        &rig.worktree(&tasks[0]),
        &["update-ref", "-d", "refs/remotes/origin/main"],
    );
    if has_finding {
        let author: TaskId = "task-20261120-000000-aaaa".parse().unwrap();
        write_finding(&rig, &author, false);
    }
    rig.resume(&tasks[0]).await;
    let prompt = rig.prompt(&tasks[0], 2).await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    rig.stop().await.unwrap();
    assert!(!prompt.contains("## Findings of other agents"), "{prompt}");
    let logs = Reader::open(&rig.home.history_file()).unwrap().daemon_logs().unwrap();
    let git_failure = logs.iter().find(|row| row.event() == LogEvent::GitCallFailed);
    if has_finding {
        let row = git_failure.expect("git discovery failure");
        assert_eq!(row.task(), Some(&tasks[0]));
        assert_eq!(row.team(), Some(&team));
        let row = logs
            .iter()
            .find(|row| row.event() == LogEvent::FindingsUnavailable)
            .expect("optional fallback");
        assert_eq!(row.task(), Some(&tasks[0]));
        assert_eq!(row.team(), Some(&team));
    } else {
        assert_eq!(git_failure, None);
        assert_eq!(
            logs.iter().find(|row| row.event() == LogEvent::FindingsUnavailable),
            None
        );
    }
}
