//! Secret-bearing CLI output must remain useful without disclosing known credentials.

use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

use agent_core::cli::ActivityEvent;
use agent_core::config::Secrets;
use agent_core::kinds::Role;
use agent_core::log::LogEvent;
use agent_core::state::{Completion, ItemPhase, MergeMode};
use agent_runtime::clock::{SharedClock, SystemClock};
use agent_runtime::history::{HistoryLayer, Reader};

use super::rig::{ONE, Rig, investigator, normal};

#[tokio::test]
async fn a_merged_round_redacts_echoed_secrets_in_prompts_arguments_steps_messages_and_commits() {
    let marker = "history-echo-secret-marker-84";
    let approving = "done:history-echo-secret-marker-84\\nVERDICT: APPROVE\\nSUMMARY: fine";
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:echo-secret"),
            investigator("inv-r1", approving),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    prepare_echo(&rig, marker);
    rig.start().await;
    // Keep a connection alive until after shutdown so the writer cannot remove the WAL before it is inspected.
    let db = rusqlite::Connection::open(rig.home.history_file()).unwrap();
    db.query_row("SELECT count(*) FROM teams", [], |r| r.get::<_, u32>(0))
        .unwrap();
    let tasks = ONE.replace(
        "Expected result:",
        &format!("Secret context: {marker}\n    Expected result:"),
    );
    let (team, _) = rig.run(&["claude-1"], &tasks, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    assert!(matches!(
        state.teams()[0].item("1.1".parse().unwrap()).unwrap().phase(),
        ItemPhase::Done {
            completion: Completion::Merged { .. }
        }
    ));
    rig.stop().await.unwrap();
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let runs = reader.runs_of_team(&team).unwrap();
    assert_eq!(runs.len(), 3);
    let solo = runs.iter().find(|r| r.start.value.role == Role::Solo).unwrap();
    assert!(solo.start.value.prompt.contains("Secret context: [REDACTED]"));
    assert!(
        solo.launch
            .as_ref()
            .unwrap()
            .value
            .args
            .iter()
            .any(|s| s.contains("Secret context: [REDACTED]"))
    );
    assert!(
        reader
            .steps(&solo.start.value.run)
            .unwrap()
            .iter()
            .any(|s| s.value.event == ActivityEvent::Text("before [REDACTED] after".to_owned()))
    );
    assert_eq!(
        reader.changes(&solo.start.value.run).unwrap().unwrap().value.commits[0].subject,
        "Commit [REDACTED]"
    );
    let reviewer = runs.iter().find(|r| r.start.value.agent.name == "inv-r1").unwrap();
    assert!(
        reviewer
            .end
            .as_ref()
            .unwrap()
            .value
            .final_message
            .as_ref()
            .unwrap()
            .starts_with("[REDACTED]\n")
    );
    assert!(
        reviewer
            .launch
            .as_ref()
            .unwrap()
            .value
            .args
            .iter()
            .any(|s| s.contains("done:[REDACTED]"))
    );
    assert_marker_absent(&rig.home.history_file().with_extension("db-wal"), marker);
    drop(reader);
    drop(db);
    assert_marker_absent(&rig.home.history_file(), marker);
    // Check that redaction affected the history, not the CLI's input.
    let received = std::fs::read_to_string(solo.start.value.worktree.as_path().join(".fake-prompt-1")).unwrap();
    assert!(received.contains(marker));
}

#[tokio::test]
async fn a_real_round_redacts_daemon_logs_and_records_run_ended() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:daemon-log-secrets"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    prepare_daemon_log_secrets(&rig);
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&stderr);
    let layer = HistoryLayer::deferred(
        agent_core::log::LogLevel::Info,
        SharedClock::new(SystemClock),
        Secrets::default(),
    );
    layer.with_stderr_sink(Arc::new(move |line: &str| {
        captured.lock().unwrap().push(line.to_owned());
    }));
    rig.start_with_log_layer(layer).await;
    let db = rusqlite::Connection::open(rig.home.history_file()).unwrap();
    db.query_row("SELECT count(*) FROM teams", [], |row| row.get::<_, u32>(0))
        .unwrap();
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let initial = rig.state();
    let round = initial.teams()[0].rounds()[0].id().clone();
    let task = tasks[0].clone();
    let state = rig
        .item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    assert!(matches!(
        state.teams()[0].item("1.1".parse().unwrap()).unwrap().phase(),
        ItemPhase::Done {
            completion: Completion::Merged { .. }
        }
    ));
    let run = state.find_task(&task).unwrap().member.runs()[0].id().clone();
    rig.stop().await.unwrap();

    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let logs = reader.daemon_logs().unwrap();
    let ended = logs.iter().find(|row| {
        row.event() == LogEvent::RunEnded
            && row.team() == Some(&team)
            && row.rounds().contains(&round)
            && row.task() == Some(&task)
            && row.run() == Some(&run)
    });
    assert!(
        ended.is_some(),
        "production RunEnded with its real scope is missing: {logs:?}"
    );
    assert!(
        ended.unwrap().fields()["end"].to_string().contains("Reported"),
        "{ended:?}"
    );
    assert!(
        logs.iter().any(|row| {
            row.event() == LogEvent::CliOutputUnreadable && row.task() == Some(&task) && row.run() == Some(&run)
        }),
        "the malformed fake-CLI line did not produce its real unreadable-output warning: {logs:?}"
    );
    let joined = stderr.lock().unwrap().join("\n");
    assert!(
        joined.contains("event=run_ended") && joined.contains("event=cli_output_unreadable"),
        "{joined}"
    );
    assert!(joined.contains("[REDACTED]"), "{joined}");
    let wal = rig.home.history_file().with_extension("db-wal");
    for marker in [
        "\"private-marker\"",
        "private-marker",
        "  whitespace-marker  ",
        "whitespace-marker",
    ] {
        assert!(!joined.contains(marker), "secret {marker:?} in daemon stderr: {joined}");
        assert!(
            !format!("{logs:?}").contains(marker),
            "secret {marker:?} in daemon log rows"
        );
        assert_marker_absent(&wal, marker);
    }
    drop(reader);
    drop(db);
    for marker in [
        "\"private-marker\"",
        "private-marker",
        "  whitespace-marker  ",
        "whitespace-marker",
    ] {
        assert_marker_absent(&rig.home.history_file(), marker);
    }
}

/// Check actual bytes, not just decoded rows.
fn assert_marker_absent(path: &std::path::Path, marker: &str) {
    let bytes = std::fs::read(path).unwrap();
    assert_ne!(bytes.len(), 0);
    assert!(
        !bytes.windows(marker.len()).any(|b| b == marker.as_bytes()),
        "secret in {path:?}"
    );
}

/// Install synthetic secrets and their echoing CLI script.
fn prepare_echo(rig: &Rig, marker: &str) {
    let secrets = rig.home.root().join("secrets.toml");
    std::fs::write(
        &secrets,
        format!("github_token = \"ghp_test_only\"\n[agent_env]\nHISTORY_TEST_SECRET = {marker:?}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o600)).unwrap();
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(
        scripts.join("echo-secret.sh"),
        r#"set -eu
printf '{"type":"assistant","message":{"content":[{"type":"text","text":"before %s after"}]}}\n' "$HISTORY_TEST_SECRET"
printf 'work\n' > work.txt
git add work.txt
git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -qm "Commit $HISTORY_TEST_SECRET"
"#,
    )
    .unwrap();
}

/// Installs quoted and whitespace-bearing secrets and a fake CLI step that emits them in malformed output.
fn prepare_daemon_log_secrets(rig: &Rig) {
    let secrets = rig.home.root().join("secrets.toml");
    std::fs::write(
        &secrets,
        "github_token = \"ghp_test_token_0123\"\n\
         [agent_env]\n\
         QUOTED_SECRET = '\"private-marker\"'\n\
         SPACED_SECRET = '  whitespace-marker  '\n",
    )
    .unwrap();
    std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o600)).unwrap();
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(
        scripts.join("daemon-log-secrets.sh"),
        r#"set -eu
printf 'not-json quoted=%s padded=%s\n' "$QUOTED_SECRET" "$SPACED_SECRET"
printf 'work\n' > work.txt
git add work.txt
git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -qm Work
"#,
    )
    .unwrap();
}
