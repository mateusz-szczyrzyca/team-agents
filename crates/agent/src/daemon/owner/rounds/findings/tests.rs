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
    let mut ledger = Findings::default();
    for (number, severity, status) in [
        (1, Severity::Blocker, FindingStatus::Resolved),
        (2, Severity::Major, FindingStatus::Open),
        (3, Severity::Minor, FindingStatus::Open),
    ] {
        ledger.entries.push(RecordedFinding {
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
    assert_eq!(table(&texts, &ledger).unwrap(), include_str!("table.md"));
    assert_eq!(
        open_text(&texts, Some(&ledger)).unwrap(),
        "2. [MAJOR] ERR-3 src/lib.rs:42 — problem | <tag>\n→ fix café\n   CHECK: read src/lib.rs:42\n\n3. [MINOR] ERR-3 src/lib.rs:42 — problem | <tag>\n→ fix café\n   CHECK: read src/lib.rs:42\n"
    );
}
