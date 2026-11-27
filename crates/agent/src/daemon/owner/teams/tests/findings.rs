//! A pooled worker sees findings on its step files before its first run.

use super::*;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::process::Stdio;

#[tokio::test]
async fn a_pooled_worker_sees_findings_on_its_step_files_and_records_them() {
    let mut f = Fixture::new(&[
        investigator("inv-opus", "sleep"),
        investigator("inv-codex", "sleep"),
        investigator("inv-gemini", "sleep"),
        normal("claude-1", "sleep"),
        worker("glm-1", "hold"),
    ]);
    std::fs::create_dir(f.repo.join("src")).unwrap();
    std::fs::write(f.repo.join("src/lib.rs"), "base").unwrap();
    git(&f.repo, &["add", "src/lib.rs"]);
    git(&f.repo, &["commit", "-m", "Source"]);
    base_origin(&f);
    f.start().await;
    let (_, tasks) = f.start_team(&["inv-opus", "claude-1"], PLAIN).await;
    for task in &tasks {
        f.status_is(task, TaskStatus::Waiting).await;
    }
    let directory = f.home.root().join("bus/notebooks/example");
    std::fs::create_dir_all(&directory).unwrap();
    let finding = serde_json::json!({"op":"add","id":"F-1","ts":"2099-01-01T00:00:00Z","author":"other","task":"task-20261120-000000-aaaa","kind":"trap","paths":["src/lib.rs"],"text":"Use the checked helper"});
    std::fs::write(directory.join("findings.jsonl"), format!("{finding}\n")).unwrap();
    let accept = f.home.root().join("accepting-agent");
    std::fs::write(&accept, "#!/bin/sh\nprintf 'ACCEPTED\\nStep 1.1 Write it\\nRequest files: src/lib.rs\\nRequest check: exists src/lib.rs\\nDo: Write it.\\n'\n").unwrap();
    std::fs::set_permissions(&accept, std::fs::Permissions::from_mode(0o700)).unwrap();
    let plan = f.home.root().join("plan.md");
    std::fs::write(&plan, "1. Work\n[>] 1.1 Write it\n").unwrap();
    let task = &tasks[1];
    let worktree = f.worktree(task);
    let call = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"delegate","arguments":{"goal":"write it","workdir":worktree,"steps":"write it","returns":"the file","for":"1.1","files":"src/lib.rs","check":"exists src/lib.rs","do":"Write it."}}});
    let mut child = Process::new(agentbus_binary())
        .args(["serve", "--agent", "claude-1", "--task", task.as_str(), "--dir"])
        .arg(f.home.root().join("bus"))
        .arg("--home")
        .arg(f.home.root())
        .args(["--repo", "example", "--branch"])
        .arg(format!("agents/claude-1/{task}"))
        .arg("--worktree")
        .arg(&worktree)
        .arg("--plan")
        .arg(plan)
        .arg("--agent-cmd")
        .arg(accept)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(child.stdin.take().unwrap(), "{call}").unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let answer = String::from_utf8(output.stdout).unwrap();
    assert!(answer.contains("Request H1 accepted"), "{answer}");
    let request = f
        .request_with(RequestId::new(std::num::NonZeroU64::MIN), DelegationStatus::Running)
        .await;
    let task = request.helper_task.unwrap();
    first_run(&f, &task).await;
    let prompt = wait_for_file(f.home.session_prompt_file(&task)).await;
    f.stop().await.unwrap();
    assert!(
        prompt.contains("- F-1 [trap] src/lib.rs — Use the checked helper"),
        "{prompt}"
    );
    let shown: serde_json::Value =
        serde_json::from_slice(&std::fs::read(f.home.task_dir(&task).join("findings-shown.json")).unwrap()).unwrap();
    assert_eq!(shown, serde_json::json!({"shown":["F-1"]}));
}
