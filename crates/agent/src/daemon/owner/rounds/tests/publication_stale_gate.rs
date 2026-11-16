//! End-to-end regression test for publication waiting on a stale gate record.
//!
//! When a task's gate passes at commit A, but subsequent work advances the branch
//! to commit B before publication, the daemon must log [`LogEvent::PublicationWaits`]
//! with reason `gate_at_another_commit`, recording both `tip` (commit B) and `gate_head` (commit A),
//! and refrain from opening a pull request until the gate passes on the new tip.

use agent_core::log::{LogEvent, LogLevel};
use agent_core::protocol::{Client, Command, GateRequest, Reply};
use agent_core::state::MergeMode;
use agent_core::status::TaskStatus;
use agent_runtime::history::Reader;

use super::rig::{ONE, Rig, investigator, normal};
use crate::client::Connection;

#[tokio::test]
async fn gate_pass_followed_by_commit_logs_stale_gate_and_waits_for_tip_gate() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "gated"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = &tasks[0];
    let worktree = rig.worktree(task);

    // Wait for the agent to commit initial work and wait at the gate barrier.
    rig.wait_for("the fake CLI to commit and wait for gate", || {
        worktree.join(".fake-committed-1").exists()
    })
    .await;

    let commit_a = super::rig::git(&worktree, &["rev-parse", "HEAD"]);

    // Run agent gate while the worktree is at commit A.
    let reply = Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request_until_answered(Client::Cli, Command::Gate(GateRequest { task: task.clone() }))
        .await
        .unwrap();
    assert!(matches!(reply, Reply::GateRecorded { .. }));

    // Advance the branch tip to commit B after the gate passed on commit A.
    super::rig::git(&worktree, &["commit", "--allow-empty", "-m", "advance tip past gate"]);
    let commit_b = super::rig::git(&worktree, &["rev-parse", "HEAD"]);
    assert_ne!(commit_a, commit_b);

    // Release the fake CLI gate barrier so the agent turn finishes.
    std::fs::write(worktree.join(".fake-gate-done-1"), "done\n").unwrap();
    rig.status_is(task, TaskStatus::Waiting).await;

    // Verify publication_waits is logged with reason gate_at_another_commit, tip, and gate_head.
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    rig.wait_for("the publication_waits row for gate_at_another_commit", || {
        reader.daemon_logs().is_ok_and(|logs| {
            logs.iter().any(|entry| {
                entry.event() == LogEvent::PublicationWaits
                    && entry.fields().get("reason").and_then(|v| v.as_str()) == Some("gate_at_another_commit")
            })
        })
    })
    .await;

    let logs = reader.daemon_logs().unwrap();
    let stale_wait = logs
        .iter()
        .find(|entry| {
            entry.event() == LogEvent::PublicationWaits
                && entry.fields().get("reason").and_then(|v| v.as_str()) == Some("gate_at_another_commit")
        })
        .expect("missing publication_waits row for gate_at_another_commit");

    assert_eq!(stale_wait.level(), LogLevel::Warn);
    assert_eq!(stale_wait.team(), Some(&team));
    assert_eq!(stale_wait.task(), Some(task));
    assert_eq!(
        stale_wait.fields().get("tip").and_then(|v| v.as_str()),
        Some(commit_b.as_str())
    );
    assert_eq!(
        stale_wait.fields().get("gate_head").and_then(|v| v.as_str()),
        Some(commit_a.as_str())
    );
    assert!(
        rig.github.pulls().is_empty(),
        "pull request must not be opened while publication waits for gate at tip"
    );

    // Run gate again at the new branch tip (commit B).
    let reply = Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request_until_answered(Client::Cli, Command::Gate(GateRequest { task: task.clone() }))
        .await
        .unwrap();
    assert!(matches!(reply, Reply::GateRecorded { .. }));

    // Now publication proceeds and opens the pull request.
    rig.wait_for("the pull request to be opened after tip gate", || {
        !rig.github.pulls().is_empty()
    })
    .await;
    assert_eq!(rig.github.pulls().len(), 1);
    assert_eq!(rig.github.pulls()[0].number, 1);

    rig.stop().await.unwrap();
}
