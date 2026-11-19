//! The findings comment as GitHub receives it, with every severity and status.
use super::*;
use agent_core::config::Language;
use agent_core::review_findings::Finding;
use agent_core::state::RecordedFinding;
use std::num::NonZeroU32;
use std::path::Path;

#[tokio::test]
async fn findings_table_matches_the_golden_and_keeps_report_data_in_one_cell() {
    let texts = AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap();
    let mut entries = Vec::new();
    for (number, severity, status) in [
        (1, Severity::Blocker, FindingStatus::Resolved),
        (2, Severity::Major, FindingStatus::Open),
        (3, Severity::Minor, FindingStatus::Open),
    ] {
        entries.push(RecordedFinding {
            number: NonZeroU32::new(number).unwrap(),
            finding: Finding {
                severity,
                rule: "ERR-3".into(),
                place: "src/lib.rs:42".into(),
                text: "problem | <tag>\n→ fix café".into(),
                check: "read src/lib.rs:42".into(),
            },
            reporter: "inv-codex".parse().unwrap(),
            pass: NonZeroU32::MIN,
            status,
        });
    }
    let ledger = Findings::new(entries).unwrap();
    assert_eq!(table(&texts, &ledger).unwrap(), include_str!("table.md"));
    assert_eq!(
        open_text(&texts, Some(&ledger)).unwrap(),
        "2. [MAJOR] ERR-3 src/lib.rs:42 — problem | <tag>\n→ fix café\n   CHECK: read src/lib.rs:42\n\n3. [MINOR] ERR-3 src/lib.rs:42 — problem | <tag>\n→ fix café\n   CHECK: read src/lib.rs:42\n"
    );
}

#[tokio::test]
async fn human_decision_lists_only_unresolved_blocking_findings() {
    let texts = AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap();
    let entries = [
        (Severity::Blocker, FindingStatus::Open),
        (Severity::Major, FindingStatus::Open),
        (Severity::Minor, FindingStatus::Open),
        (Severity::Major, FindingStatus::Resolved),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (severity, status))| RecordedFinding {
        number: NonZeroU32::new(u32::try_from(i + 1).unwrap()).unwrap(),
        finding: Finding {
            severity,
            rule: "ERR-3".into(),
            place: "src/lib.rs:42".into(),
            text: "fix café".into(),
            check: "read src/lib.rs:42".into(),
        },
        reporter: "inv-codex".parse().unwrap(),
        pass: NonZeroU32::MIN,
        status,
    })
    .collect();
    let ledger = Findings::new(entries).unwrap();
    assert_eq!(
        blocking_text(&texts, Some(&ledger)).unwrap(),
        "1. [BLOCKER] ERR-3 src/lib.rs:42 — fix café\n   CHECK: read src/lib.rs:42\n\n2. [MAJOR] ERR-3 src/lib.rs:42 — fix café\n   CHECK: read src/lib.rs:42\n"
    );
    assert_eq!(blocking_text(&texts, None).unwrap(), "");
}
