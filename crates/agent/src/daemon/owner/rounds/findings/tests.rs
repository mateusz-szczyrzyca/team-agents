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

#[tokio::test]
async fn explanations_and_verifier_evidence_are_in_the_table_and_request() {
    use agent_core::review_findings::Answer;
    use agent_core::state::{Explanation, ExplanationStatus};
    let texts = AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap();
    let mut ledger = Findings::verifying(agent_core::config::ReviewCount::TWO);
    let wire = serde_json::json!({"entries":[{
        "number":1,"finding":{"severity":"MINOR","rule":"TYP-3","place":"src/lib.rs:1","text":"units → newtype","check":"read signature"},
        "reporter":"inv-codex","pass":1,"status":"open"
    }]});
    let entries: Findings = serde_json::from_value(wire).unwrap();
    // Keep the new protocol marker while installing a recorded finding.
    let mut json = serde_json::to_value(&ledger).unwrap();
    json["entries"] = serde_json::to_value(entries.entries()).unwrap();
    ledger = serde_json::from_value(json).unwrap();
    ledger.disputes_mut().unwrap().explanations.insert(
        NonZeroU32::MIN,
        Explanation {
            why: "explicit café units".into(),
            status: ExplanationStatus::Accepted,
            votes: [(
                "inv-opus".parse().unwrap(),
                Answer::Accepted {
                    why: "contract is clear".into(),
                },
            )]
            .into(),
        },
    );
    assert_eq!(table(&texts, &ledger).unwrap(), include_str!("explanation-table.md"));
    assert!(
        open_text(&texts, Some(&ledger))
            .unwrap()
            .contains("explicit café units")
    );
    assert!(
        open_text(&texts, Some(&ledger))
            .unwrap()
            .contains("inv-opus: ACCEPTED — contract is clear")
    );
}

#[tokio::test]
async fn findings_table_shows_a_not_confirmed_blocker_with_its_reporter() {
    let texts = AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap();
    let entry = RecordedFinding {
        number: NonZeroU32::MIN,
        finding: Finding {
            severity: Severity::Blocker,
            rule: "ERR-3".into(),
            place: "src/lib.rs:42".into(),
            text: "fix café".into(),
            check: "read src/lib.rs:42".into(),
        },
        reporter: "inv-codex".parse().unwrap(),
        pass: NonZeroU32::MIN,
        status: FindingStatus::NotConfirmed,
    };
    let ledger = Findings::new(vec![entry]).unwrap();

    let table = table(&texts, &ledger).unwrap();

    let row = table
        .lines()
        .find(|line| line.contains("fix café"))
        .expect("the finding has a row");
    assert!(row.contains("not confirmed"), "{row}");
    assert!(row.contains("inv-codex"), "{row}");
}

/// The ledger of item 1.1 of the evidence: finding 28 with a pending explanation, finding 8 a plain open minor one.
fn evidence_ledger() -> Findings {
    let state = agent_core::state::State::from_json(include_str!(
        "../../../../../../agent-core/tests/fixtures/state-split-explanations.json"
    ))
    .unwrap();
    state.teams()[0]
        .item("1.1".parse().unwrap())
        .unwrap()
        .findings()
        .unwrap()
        .clone()
}

/// The lines of finding `number` in a rendered list of open findings: from its own line to the next finding's.
fn block_of(text: &str, number: u32) -> String {
    let start = format!("{number}. [");
    text.lines()
        .skip_while(|line| !line.starts_with(&start))
        .enumerate()
        .take_while(|(index, line)| *index == 0 || !line.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn the_brief_of_a_verifier_names_beside_each_open_finding_the_answers_it_may_get() {
    let texts = AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap();
    let ledger = evidence_ledger();

    let text = verification_text(&texts, Some(&ledger)).unwrap();

    let explained = block_of(&text, 28);
    assert!(
        explained.contains("FINDING 28: ACCEPTED — why or REJECTED — why or RAISED [BLOCKER] — why"),
        "{explained}"
    );
    assert!(!explained.contains("RESOLVED"), "{explained}");
    let plain = block_of(&text, 8);
    assert!(
        plain.contains("FINDING 8: RESOLVED or OPEN — why or RAISED [MAJOR] — why or RAISED [BLOCKER] — why"),
        "{plain}"
    );
    let blocker = block_of(&text, 27);
    assert!(blocker.contains("FINDING 27: RESOLVED or OPEN — why"), "{blocker}");
    assert!(!blocker.contains("RAISED"), "{blocker}");
    for number in [3, 8, 17, 18, 19, 20, 21, 25, 26, 27, 28] {
        assert_eq!(
            text.matches(&format!("FINDING {number}: ")).count(),
            1,
            "finding {number}: {text}"
        );
    }
}
