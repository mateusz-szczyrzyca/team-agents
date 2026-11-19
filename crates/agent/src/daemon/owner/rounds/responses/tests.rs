//! Corrections without a located finding must not invent one.
use super::*;

#[tokio::test]
async fn an_unlocated_error_requests_all_numbers_without_offering_a_finding_specific_answer() {
    let texts = agent_runtime::autopilot_prompts::AutopilotPrompts::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &agent_core::config::Language::default(),
    )
    .await
    .unwrap();

    assert_eq!(
        unlocated_correction(&texts).unwrap(),
        texts
            .render(AutopilotPrompt::ReviewResponseNumbers, &Variables::new())
            .unwrap()
    );
}

#[tokio::test]
async fn rendered_diagnostic_quotes_the_refused_line_and_its_own_unknown_number() {
    let texts = agent_runtime::autopilot_prompts::AutopilotPrompts::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &agent_core::config::Language::default(),
    )
    .await
    .unwrap();
    let detail = agent_core::review_findings::AnswerDiagnostic {
        source_line: Some("  FINDING 99: FIXED — café".to_owned()),
        number: std::num::NonZeroU32::new(99),
        mistake: agent_core::review_findings::AnswerMistake::UnknownNumber,
    };
    let rendered = answer_diagnostic(&texts, &detail, "FINDING 1: FIXED\nFINDING 2: FIXED or EXPLAINED — why").unwrap();
    assert!(rendered.contains("FINDING 99"), "{rendered}");
    assert!(rendered.contains("unknown number"), "{rendered}");
    assert!(rendered.contains("  FINDING 99: FIXED — café"), "{rendered}");
    assert!(
        rendered.contains("FINDING 1: FIXED\nFINDING 2: FIXED or EXPLAINED — why"),
        "{rendered}"
    );
}

#[tokio::test]
async fn a_verifier_is_shown_a_form_for_every_open_finding_minor_ones_included() {
    use agent_core::review_findings::{Finding, Severity};
    use agent_core::state::{FindingStatus, Findings, RecordedFinding};
    let texts = agent_runtime::autopilot_prompts::AutopilotPrompts::load(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &agent_core::config::Language::default(),
    )
    .await
    .unwrap();
    let entry = |number: u32, severity| RecordedFinding {
        number: std::num::NonZeroU32::new(number).unwrap(),
        finding: Finding {
            severity,
            rule: "DOC-3".into(),
            place: "src/lib.rs:1".into(),
            text: "wording → clarify".into(),
            check: "read src/lib.rs:1".into(),
        },
        reporter: "inv-codex".parse().unwrap(),
        pass: std::num::NonZeroU32::MIN,
        status: FindingStatus::Open,
    };
    let ledger = Findings::new(vec![entry(1, Severity::Blocker), entry(2, Severity::Minor)]).unwrap();

    let forms = verifier_forms(&texts, &ledger).unwrap();

    assert!(forms.contains("FINDING 1:"), "{forms}");
    assert!(forms.contains("FINDING 2:"), "{forms}");
}
