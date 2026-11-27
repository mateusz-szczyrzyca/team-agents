//! Findings are delivered once per task, including after restarting the daemon.
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
        lines.push_str(&format!("{}\n", json!({"op":"resolve","id":"F-1","ts":"2099-01-01T00:00:01Z","author":"other","task":task.as_str(),"reason":"fixed"})));
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
