//! The capture deadline must release the round to review and merge, with an explicit lost record.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

use agent_core::history::HistoryState;
use agent_core::kinds::Role;
use agent_core::protocol::{Client, Command, Reply, StatusRequest};
use agent_core::state::{Completion, ItemPhase, MergeMode};
use agent_runtime::history::Reader;

use super::rig::{ONE, Rig, investigator, normal};

#[tokio::test]
async fn a_capture_timeout_is_counted_and_the_round_still_merges() {
    let Some(ready) = std::env::var_os("HISTORY_CAPTURE_READY") else {
        run_with_private_git_wrapper().await;
        return;
    };
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let ready = PathBuf::from(ready);
    rig.wait_for("history git snapshot entered", || ready.exists()).await;
    tokio::time::pause();
    // Tokio rounds timer deadlines up to the next millisecond; deliver the two-second budget's timer.
    tokio::time::advance(Duration::from_millis(2001)).await;
    tokio::time::resume();
    let state = rig
        .item_where("1.1", "merged after the capture timeout", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
    assert!(matches!(
        state.teams()[0].item("1.1".parse().unwrap()).unwrap().phase(),
        ItemPhase::Done {
            completion: Completion::Merged { .. }
        }
    ));
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(Client::Cli, Command::Status(StatusRequest { target: None }))
        .await
        .unwrap();
    let Reply::Overview { history, .. } = reply else {
        panic!("{reply:?}")
    };
    assert_eq!(history.state, HistoryState::Writing);
    assert_eq!(history.lost, 1);
    rig.stop().await.unwrap();
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let runs = reader.runs_of_team(&team).unwrap();
    assert_eq!(runs.len(), 3);
    for run in runs {
        assert!(run.end.is_some(), "{run:?}");
        let changes = reader.changes(&run.start.value.run).unwrap();
        assert_eq!(changes.is_none(), run.start.value.role == Role::Solo, "{run:?}");
    }
}

/// A child test process owns its PATH; no concurrent test sees the deliberately stalled Git command.
async fn run_with_private_git_wrapper() {
    let scratch = tempfile::tempdir().unwrap();
    let path = std::env::var_os("PATH").unwrap();
    let real_git = std::env::split_paths(&path)
        .map(|dir| dir.join("git"))
        .find(|file| file.is_file())
        .unwrap();
    let git = scratch.path().join("git");
    std::fs::write(
        &git,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/history-slow-git.sh"
        )),
    )
    .unwrap();
    std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o700)).unwrap();
    let paths = std::iter::once(scratch.path().to_path_buf()).chain(std::env::split_paths(&path));
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "daemon::owner::rounds::history_deadline_tests::a_capture_timeout_is_counted_and_the_round_still_merges",
            "--nocapture",
        ])
        .env("PATH", std::env::join_paths(paths).unwrap())
        .env("HISTORY_REAL_GIT", real_git)
        .env("HISTORY_CAPTURE_READY", scratch.path().join("ready"))
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
