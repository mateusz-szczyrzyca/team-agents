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

/// A recorded finding of these tests: rule ERR-3 at `src/lib.rs:42`, reported by `inv-codex`.
fn recorded(number: u32, severity: Severity, status: FindingStatus, text: &str) -> RecordedFinding {
    RecordedFinding {
        number: NonZeroU32::new(number).unwrap(),
        finding: Finding {
            severity,
            rule: "ERR-3".into(),
            place: "src/lib.rs:42".into(),
            text: text.into(),
            check: "read src/lib.rs:42".into(),
        },
        reporter: "inv-codex".parse().unwrap(),
        pass: NonZeroU32::MIN,
        status,
    }
}

/// The autopilot texts as the repository ships them.
async fn load_texts() -> AutopilotPrompts {
    AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap()
}

/// The comment of one open minor finding with `text`.
async fn comment_with_text(text: &str) -> String {
    let texts = load_texts().await;
    let ledger = Findings::new(vec![recorded(1, Severity::Minor, FindingStatus::Open, text)]).unwrap();
    comment(&texts, &ledger).unwrap()
}

/// The text line that follows the first line of entry `number` in `comment`.
fn text_line(comment: &str, number: u32) -> &str {
    let head = format!("**{number}** ");
    let mut lines = comment.lines().skip_while(|line| !line.starts_with(&head));
    lines.next();
    lines.next().expect("the entry has a text line")
}

#[tokio::test]
async fn comment_lists_open_findings_and_folds_closed_ones_in_details() {
    let texts = load_texts().await;
    let ledger = Findings::new(vec![
        recorded(1, Severity::Blocker, FindingStatus::Resolved, "problem | <tag>\n→ fix café & more"),
        recorded(2, Severity::Major, FindingStatus::Open, "problem | <tag>\n→ fix café & more"),
        recorded(3, Severity::Minor, FindingStatus::Open, "problem | <tag>\n→ fix café & more"),
    ])
    .unwrap();
    assert_eq!(comment(&texts, &ledger).unwrap(), include_str!("comment.md"));
}

#[tokio::test]
async fn comment_without_open_findings_counts_zero_and_folds_everything() {
    let texts = load_texts().await;
    let ledger = Findings::new(vec![recorded(1, Severity::Major, FindingStatus::Resolved, "fix café")]).unwrap();
    assert_eq!(
        comment(&texts, &ledger).unwrap(),
        r#"<!-- agent-findings -->
## Review findings

open — 0 BLOCKER, 0 MAJOR, 0 MINOR; closed — 1

<details>
<summary>Closed (1)</summary>

**1** **$\color{#FF4D4D}{\textsf{[MAJOR]}}$** ERR-3 · resolved · inv-codex — src/lib.rs:42
fix café

</details>
"#
    );
}

#[tokio::test]
async fn comment_without_closed_findings_has_no_details_block() {
    let texts = load_texts().await;
    let ledger = Findings::new(vec![recorded(1, Severity::Minor, FindingStatus::Open, "fix café")]).unwrap();
    assert_eq!(
        comment(&texts, &ledger).unwrap(),
        r#"<!-- agent-findings -->
## Review findings

open — 0 BLOCKER, 0 MAJOR, 1 MINOR; closed — 0

**1** **$\color{orange}{\textsf{[MINOR]}}$** ERR-3 · open · inv-codex — src/lib.rs:42
fix café
"#
    );
}

#[tokio::test]
async fn text_of_159_characters_is_not_cut() {
    let text = "x".repeat(159);
    assert_eq!(text_line(&comment_with_text(&text).await, 1), text);
}

#[tokio::test]
async fn text_of_160_characters_is_not_cut() {
    let text = "x".repeat(160);
    assert_eq!(text_line(&comment_with_text(&text).await, 1), text);
}

#[tokio::test]
async fn text_of_400_characters_without_a_space_is_cut_at_160_with_an_ellipsis() {
    let text = "x".repeat(400);
    assert_eq!(text_line(&comment_with_text(&text).await, 1), format!("{}…", "x".repeat(160)));
}

#[tokio::test]
async fn long_text_is_cut_at_the_last_word_boundary_before_160_characters() {
    let text = "word ".repeat(100);
    assert_eq!(
        text_line(&comment_with_text(&text).await, 1),
        format!("{}word…", "word ".repeat(31))
    );
}

#[tokio::test]
async fn text_with_a_line_break_a_fence_a_tag_and_a_bar_stays_on_one_line() {
    let comment = comment_with_text("# first\n```\n<details>\n| a |").await;
    assert_eq!(
        text_line(&comment, 1),
        "&#35; first ``` &lt;details&gt; &#124; a &#124;"
    );
    assert!(!comment.contains("<details>"), "{comment}");
    assert!(!comment.lines().any(|line| line.starts_with("```")), "{comment}");
}

#[tokio::test]
async fn explained_finding_is_closed_and_its_explanation_is_the_third_line() {
    use agent_core::review_findings::Answer;
    use agent_core::state::{Explanation, ExplanationStatus};
    let texts = load_texts().await;
    let mut ledger = Findings::verifying(agent_core::config::ReviewCount::TWO);
    let wire = serde_json::json!({"entries":[{
        "number":1,"finding":{"severity":"MINOR","rule":"TYP-3","place":"src/lib.rs:1","text":"units → newtype","check":"read signature"},
        "reporter":"inv-codex","pass":1,"status":"open"
    }]});
    let entries: Findings = serde_json::from_value(wire).unwrap();
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
    assert_eq!(
        comment(&texts, &ledger).unwrap(),
        "<!-- agent-findings -->\n## Review findings\n\nopen — 0 BLOCKER, 0 MAJOR, 0 MINOR; closed — 1\n\n<details>\n<summary>Closed (1)</summary>\n\n**1** **$\\color{orange}{\\textsf{[MINOR]}}$** TYP-3 · open · inv-codex — src/lib.rs:1\nunits → newtype\nExplanation (accepted; cannot be offered again): explicit café units inv-opus: ACCEPTED — contract is clear\n\n</details>\n"
    );
}

#[tokio::test]
async fn second_opinion_is_the_third_line_of_its_finding() {
    use agent_core::review_response::opinion::{Evidence, Outcome, OpinionReport};
    use agent_core::state::OpinionRequest;
    let texts = load_texts().await;
    let mut ledger = Findings::new(vec![recorded(1, Severity::Blocker, FindingStatus::Open, "fix café")]).unwrap();
    let n = NonZeroU32::MIN;
    let task: agent_core::ids::TaskId = "task-20261111-193007-k3x9".parse().unwrap();
    let by: agent_core::ids::InvestigatorName = "inv-opus".parse().unwrap();
    ledger
        .request_opinion(
            n,
            OpinionRequest {
                pass: NonZeroU32::MIN,
                head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
                reason: "café branch is unreachable".to_owned().try_into().unwrap(),
            },
        )
        .unwrap();
    ledger.assign_opinion(n, by.clone(), task.clone()).unwrap();
    ledger
        .record_opinion(
            n,
            &task,
            OpinionReport {
                outcome: Outcome::Confirmed,
                evidence: "read café.rs: branch is live".to_owned().try_into().unwrap(),
            },
        )
        .unwrap();
    assert_eq!(
        comment(&texts, &ledger).unwrap(),
        "<!-- agent-findings -->\n## Review findings\n\nopen — 1 BLOCKER, 0 MAJOR, 0 MINOR; closed — 0\n\n**1** **$\\color{#8B0000}{\\textsf{[BLOCKER]}}$** ERR-3 · open · inv-codex — src/lib.rs:42\nfix café\nSecond opinion by inv-opus: CONFIRMED — read café.rs: branch is live\n"
    );
}
