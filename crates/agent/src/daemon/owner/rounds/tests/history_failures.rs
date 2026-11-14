//! History storage failures must not prevent the fake GitHub round from merging.

use std::os::unix::fs::PermissionsExt;

use agent_core::history::{HistoryHealth, HistoryState, MaxTextBytes};
use agent_core::protocol::{Client, Command, Reply, StatusRequest};
use agent_core::state::{Completion, ItemPhase, MergeMode};
use agent_runtime::history::{HistoryDb, Reader};

use super::rig::{ONE, Rig, investigator, normal};

/// An isolated repository with an agent and two approving reviewers.
async fn rig() -> Rig {
    Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await
}

/// Finish a real daemon round and obtain the same health exposed by the CLI.
async fn merge_and_status(rig: &Rig) -> HistoryHealth {
    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
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
    let Reply::Overview { history, teams } = reply else {
        panic!("{reply:?}")
    };
    assert_eq!(teams.len(), 1);
    history
}

/// Restore permissions even on assertion failure so the temporary home can be removed.
struct ReadOnlyHistory(std::path::PathBuf);

impl Drop for ReadOnlyHistory {
    fn drop(&mut self) {
        std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(self.0.join("history.db"), std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

#[tokio::test]
async fn read_only_history_storage_leaves_state_writable_and_the_round_merges() {
    let mut rig = rig().await;
    let storage = rig.home.root().join("read-only-history");
    std::fs::create_dir(&storage).unwrap();
    let database = storage.join("history.db");
    drop(HistoryDb::open(&database, MaxTextBytes::default()).unwrap());
    std::os::unix::fs::symlink(&database, rig.home.history_file()).unwrap();
    let restore = ReadOnlyHistory(storage.clone());
    std::fs::set_permissions(&database, std::fs::Permissions::from_mode(0o400)).unwrap();
    std::fs::set_permissions(&storage, std::fs::Permissions::from_mode(0o500)).unwrap();
    let error = std::fs::OpenOptions::new().write(true).open(&database).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    rig.start().await;
    let health = merge_and_status(&rig).await;
    rig.stop().await.unwrap();
    assert!(matches!(health.state, HistoryState::Unavailable { .. }), "{health:?}");
    assert!(health.lost >= 3, "{health:?}");
    drop(restore);
}

#[tokio::test]
async fn failing_step_inserts_are_counted_while_other_records_survive_and_the_round_merges() {
    let mut rig = rig().await;
    rig.start().await;
    let db = rusqlite::Connection::open(rig.home.history_file()).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_steps BEFORE INSERT ON steps BEGIN SELECT RAISE(FAIL, 'injected step failure'); END;",
    )
    .unwrap();
    let health = merge_and_status(&rig).await;
    rig.stop().await.unwrap();
    assert_eq!(health.state, HistoryState::Writing);
    assert!(health.lost >= 3, "{health:?}");
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let state = rig.state();
    let runs = reader.runs_of_team(state.teams()[0].id()).unwrap();
    assert_eq!(runs.len(), 3);
    for run in runs {
        assert!(run.end.is_some(), "{run:?}");
        assert_eq!(reader.steps(&run.start.value.run).unwrap(), Vec::new());
    }
}

#[tokio::test]
async fn a_full_history_queue_does_not_prevent_a_round_from_merging() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:burst"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let event =
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"queue pressure\"}]}}\n";
    std::fs::write(scripts.join("burst.jsonl"), event.repeat(12_000)).unwrap();
    std::fs::write(
        scripts.join("burst.sh"),
        r#"set -eu
cat "$AGENTS_HOME/fake-scripts/burst.jsonl"
printf 'queue pressure work\n' > work.txt
git add work.txt
git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -qm 'Complete work during history saturation'
"#,
    )
    .unwrap();
    rig.start().await;
    let db = rusqlite::Connection::open(rig.home.history_file()).unwrap();
    db.execute_batch("BEGIN IMMEDIATE").unwrap();
    let health = merge_and_status(&rig).await;
    // Release the external writer only after the merge, so draining history cannot be required for progress.
    db.execute_batch("ROLLBACK").unwrap();
    rig.stop().await.unwrap();
    assert_eq!(health.state, HistoryState::Writing);
    assert!(health.lost > 0, "{health:?}");
    let integrity: String = db.query_row("PRAGMA integrity_check", [], |row| row.get(0)).unwrap();
    assert_eq!(integrity, "ok");
}

#[tokio::test]
async fn sqlite_full_does_not_prevent_merging_or_the_other_records_surviving() {
    let mut rig = rig().await;
    rig.start().await;
    let db = rusqlite::Connection::open(rig.home.history_file()).unwrap();
    db.execute_batch(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/history-full.sql"
    )))
    .unwrap();
    // This is SQLite's actual FULL code, not a constraint error whose wording resembles disk exhaustion.
    let failure = db
        .execute("INSERT INTO history_fault_full(id) VALUES(NULL)", [])
        .unwrap_err();
    assert!(matches!(failure, rusqlite::Error::SqliteFailure(error, _) if error.code == rusqlite::ErrorCode::DiskFull));
    let health = merge_and_status(&rig).await;
    rig.stop().await.unwrap();
    assert_eq!(health.state, HistoryState::Writing);
    assert!(health.lost >= 3, "{health:?}");
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let runs = reader.runs_of_team(rig.state().teams()[0].id()).unwrap();
    assert_eq!(runs.len(), 3);
    for run in runs {
        assert!(run.end.is_some(), "{run:?}");
        assert_eq!(reader.steps(&run.start.value.run).unwrap(), Vec::new());
    }
    let integrity: String = db.query_row("PRAGMA integrity_check", [], |row| row.get(0)).unwrap();
    assert_eq!(integrity, "ok");
}
