//! A secret a reviewer echoes in its report reaches neither the events of the history nor the bytes of its file: not
//! the report, and not the findings the fix pass sent the team.

use agent_core::history::{HistoryEvent, Record};
use agent_core::state::{ItemPhase, MergeMode};
use agent_runtime::history::Reader;
use std::os::unix::fs::PermissionsExt;

use super::rig::{ONE, Rig, investigator, normal};

/// The value the reviewer echoes and the configuration makes a secret.
const MARKER: &str = "history-event-secret-marker-85";

/// A reviewer that asks for a change in words that carry the secret, then approves.
const REVIEWER: &str = concat!(
    "done:VERDICT: REQUEST_CHANGES\\nBLOCKERS\\n1. [TDD-2] src/lib.rs:7 — leaks ",
    "history-event-secret-marker-85",
    ",done:VERDICT: APPROVE\\nSUMMARY: fixed"
);

#[tokio::test]
async fn a_secret_in_a_review_report_is_redacted_in_the_report_and_in_the_findings_of_the_fix_pass() {
    let marker = MARKER;
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", REVIEWER),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    let secrets = rig.home.root().join("secrets.toml");
    std::fs::write(
        &secrets,
        format!("github_token = \"ghp_test_only\"\n[agent_env]\nHISTORY_TEST_SECRET = {marker:?}\n"),
    )
    .unwrap();
    std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o600)).unwrap();
    rig.start().await;
    // A connection kept until the bytes are read, so the writer cannot remove the WAL first.
    let db = rusqlite::Connection::open(rig.home.history_file()).unwrap();
    db.query_row("SELECT count(*) FROM teams", [], |r| r.get::<_, u32>(0))
        .unwrap();

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let events: Vec<HistoryEvent> = reader
        .team_records(&team)
        .unwrap()
        .into_iter()
        .filter_map(|stored| match stored.record {
            Record::Event(event) => Some(event.event),
            _ => None,
        })
        .collect();
    let reports: Vec<&String> = events
        .iter()
        .filter_map(|event| match event {
            HistoryEvent::ReviewSubmitted { report, .. } => Some(report),
            _ => None,
        })
        .collect();
    let findings: Vec<&String> = events
        .iter()
        .filter_map(|event| match event {
            HistoryEvent::FixPassStarted { findings, .. } => Some(findings),
            _ => None,
        })
        .collect();
    assert!(
        reports.iter().any(|report| report.contains("leaks [REDACTED]")),
        "{reports:?}"
    );
    assert!(
        findings.iter().any(|text| text.contains("leaks [REDACTED]")),
        "{findings:?}"
    );
    assert!(reports.iter().chain(&findings).all(|text| !text.contains(marker)));

    let wal = std::fs::read(rig.home.history_file().with_extension("db-wal")).unwrap();
    assert!(!wal.windows(marker.len()).any(|bytes| bytes == marker.as_bytes()));
    drop(reader);
    drop(db);
    let file = std::fs::read(rig.home.history_file()).unwrap();
    assert!(!file.windows(marker.len()).any(|bytes| bytes == marker.as_bytes()));
}
