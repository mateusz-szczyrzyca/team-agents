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
    let git_failure = logs
        .iter()
        .find(|row| row.event() == LogEvent::GitCallFailed && row.task() == Some(&tasks[0]));
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
        assert_eq!(
            logs.iter().find(|row| row.event() == LogEvent::FindingsUnavailable),
            None
        );
    }
}

#[tokio::test]
async fn findings_lookup_preserves_registered_tasks_and_eventual_launch() {
    let mut rig = rig().await;
    rig.start().await;
    let (_, tasks) = rig.run(&["codex-1"], ONE, MergeMode::NoMerge, None).await;
    let text = std::fs::read_to_string(rig.home.state_file()).unwrap();
    let state = agent_core::state::State::from_json(&text).unwrap();
    assert_eq!(state.find_task(&tasks[0]).unwrap().member.task(), &tasks[0]);
    let launched = rig
        .state_where("the initial findings lookup finishes before launch", |state| {
            state
                .find_task(&tasks[0])
                .is_some_and(|found| !found.member.runs().is_empty())
        })
        .await;
    rig.stop().await.unwrap();
    assert_eq!(launched.find_task(&tasks[0]).unwrap().member.runs().len(), 1);
}

#[tokio::test]
async fn findings_for_investigator_match_requesters_worktree() {
    let mut rig = Rig::new(
        &[
            normal("claude-lead", "sleep"),
            normal("claude-1", "inv-add-mod:Help with src/lib.rs,sleep"),
            investigator("inv-1", "inv-done:Here is the solution to your issue"),
            investigator("inv-2", "inv-done:Here is the solution to your issue"),
        ],
        1,
        true,
    )
    .await;
    std::fs::create_dir(rig.repo.join("src")).unwrap();
    std::fs::write(rig.repo.join("src/lib.rs"), "base").unwrap();
    git(&rig.repo, &["add", "src/lib.rs"]);
    git(&rig.repo, &["commit", "-m", "Source"]);
    git(&rig.repo, &["push", "origin", "main"]);

    let other_author: TaskId = "task-20261120-000000-aaaa".parse().unwrap();
    write_finding(&rig, &other_author, false);

    rig.start().await;
    let (team, _) = rig
        .run(
            &["claude-lead", "claude-1"],
            ONE,
            MergeMode::Autopilot,
            Some("claude-lead"),
        )
        .await;

    let inv_state = rig
        .state_where("investigator task is Done", |state| {
            state.team(&team).is_some_and(|t| {
                t.reviewers()
                    .iter()
                    .any(|m| m.role() == Some(agent_core::kinds::Role::Investigator) && m.status() == TaskStatus::Done)
            })
        })
        .await;

    let inv_task = inv_state
        .team(&team)
        .unwrap()
        .reviewers()
        .iter()
        .find(|m| m.role() == Some(agent_core::kinds::Role::Investigator))
        .unwrap()
        .task()
        .clone();

    let prompt = rig.prompt(&inv_task, 1).await;
    rig.stop().await.unwrap();

    assert!(
        prompt.contains("## Findings of other agents on the files of your work"),
        "{prompt}"
    );
    assert!(
        prompt.contains("- F-1 [trap] src/lib.rs — Use the checked helper"),
        "{prompt}"
    );
}

fn add_worker(rig: &Rig, name: &str, scenario: &str) {
    let text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    let root = rig.home.root().display();
    let table = format!(
        "[agents.{name}]\ncli = \"claude\"\nargs = [\"--scenario\", \"{scenario}\", \"--as\", \"{name}\", \"--home\", \"{root}\", \
         \"--bus-bin\", \"{}\", \"--bus-dir\", \"{root}/bus\"]\n",
        crate::daemon::owner::fixture::agentbus_binary().display(),
    );
    let with_tier = text.replace(
        "[tiers]\nnormal = [",
        &format!("[tiers]\nworkers = [\"{name}\"]\nnormal = ["),
    );
    std::fs::write(rig.home.config_file(), format!("{table}{with_tier}")).unwrap();
}

fn delegate_step(rig: &Rig, task: &TaskId, files: &str) {
    let accept = rig.home.root().join("accepting-agent");
    std::fs::write(
        &accept,
        format!(
            "#!/bin/sh\nprintf 'ACCEPTED\\nStep 1.1 Write it\\nRequest files: {files}\\nRequest check: exists {files}\\nDo: Write it.\\n'\n",
        ),
    )
    .unwrap();
    std::fs::set_permissions(&accept, std::os::unix::fs::PermissionsExt::from_mode(0o700)).unwrap();

    let plan = rig.home.root().join("plan.md");
    std::fs::write(&plan, "1. Work\n[>] 1.1 Write it\n").unwrap();

    let worktree = rig.worktree(task);
    let call = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "delegate",
            "arguments": {
                "goal": "write it",
                "workdir": worktree,
                "steps": "write it",
                "returns": "the file",
                "for": "1.1",
                "files": files,
                "check": format!("exists {files}"),
                "do": "Write it."
            }
        }
    });

    let mut child = std::process::Command::new(crate::daemon::owner::fixture::agentbus_binary())
        .args(["serve", "--agent", "claude-1", "--task", task.as_str(), "--dir"])
        .arg(rig.home.root().join("bus"))
        .arg("--home")
        .arg(rig.home.root())
        .args(["--repo", "example", "--branch"])
        .arg(format!("agents/claude-1/{task}"))
        .arg("--worktree")
        .arg(&worktree)
        .arg("--plan")
        .arg(&plan)
        .arg("--agent-cmd")
        .arg(&accept)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    std::io::Write::write_all(&mut child.stdin.take().unwrap(), call.to_string().as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let answer = String::from_utf8(output.stdout).unwrap();
    assert!(answer.contains("Request H1 accepted"), "{answer}");
}

#[tokio::test]
async fn findings_for_delegated_worker_match_step_files_only() {
    let mut rig = Rig::new(
        &[
            normal("claude-lead", "sleep"),
            normal("claude-1", "sleep"),
            investigator("inv-1", "sleep"),
            investigator("inv-2", "sleep"),
        ],
        1,
        true,
    )
    .await;
    std::fs::create_dir(rig.repo.join("src")).unwrap();
    std::fs::write(rig.repo.join("src/lib.rs"), "base").unwrap();
    git(&rig.repo, &["add", "src/lib.rs"]);
    git(&rig.repo, &["commit", "-m", "Source"]);
    git(&rig.repo, &["push", "origin", "main"]);

    add_worker(&rig, "glm-1", "sleep,sleep");

    let other_author: TaskId = "task-20261120-000000-aaaa".parse().unwrap();
    write_finding(&rig, &other_author, false);

    rig.start().await;
    let (_, tasks) = rig
        .run(
            &["claude-lead", "claude-1", "glm-1"],
            ONE,
            MergeMode::Autopilot,
            Some("claude-lead"),
        )
        .await;

    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    rig.status_is(&tasks[1], TaskStatus::Waiting).await;
    rig.status_is(&tasks[2], TaskStatus::Waiting).await;

    delegate_step(&rig, &tasks[1], "src/lib.rs");

    let worker_task = &tasks[2];
    rig.state_where("worker running delegated task", |state| {
        state
            .find_task(worker_task)
            .is_some_and(|found| found.member.runs().len() == 2)
    })
    .await;
    rig.status_is(worker_task, TaskStatus::Waiting).await;

    let prompt = rig.prompt(worker_task, 2).await;
    rig.stop().await.unwrap();

    assert!(
        prompt.contains("## Findings of other agents on the files of your work"),
        "{prompt}"
    );
    assert!(
        prompt.contains("- F-1 [trap] src/lib.rs — Use the checked helper"),
        "{prompt}"
    );
}
