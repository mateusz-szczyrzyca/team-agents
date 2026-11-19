//! Fix-response syntax and one-shot explanation boundaries.
use super::*;

#[test]
fn accepts_fixed_blockers_and_one_major_explanation() {
    let open = [
        (NonZeroU32::MIN, Severity::Blocker, false),
        (NonZeroU32::new(2).unwrap(), Severity::Major, false),
    ];
    assert_eq!(
        parse_lead(
            "FINDING 1: FIXED\nFINDING 2: EXPLAINED — nécessaire\nSTATUS: DONE",
            &open
        )
        .unwrap(),
        vec![
            (open[0].0, Response::Fixed),
            (open[1].0, Response::Explained("nécessaire".into()))
        ]
    );
}

#[test]
fn refuses_missing_duplicate_unknown_and_forbidden_responses() {
    for (text, severity, used, line) in [
        ("STATUS: DONE", Severity::Major, false, 2),
        ("FINDING 1: EXPLAINED — why", Severity::Blocker, false, 1),
        ("FINDING 1: EXPLAINED — why", Severity::Major, true, 1),
        ("FINDING 1: SECOND OPINION — why", Severity::Blocker, false, 1),
        ("FINDING 1: EXPLAINED — ", Severity::Major, false, 1),
        ("FINDING 1: FIXED\nFINDING 1: FIXED", Severity::Major, false, 2),
        ("FINDING 2: FIXED", Severity::Major, false, 1),
        ("STATUS: DONE\nFINDING 1: FIXED", Severity::Major, false, 2),
    ] {
        let err = parse_lead(text, &[(NonZeroU32::MIN, severity, used)]).expect_err(text);
        assert_eq!((err.line, err.number), (line, NonZeroU32::MIN));
    }
    assert_eq!(parse_lead("STATUS: DONE", &[]).unwrap(), vec![]);
}

#[test]
fn fenced_examples_never_count_as_lead_answers() {
    for (open, inside, close) in [
        ("```rust", "FINDING 1: FIXED", "```"),
        ("~~~text", "FINDING 1: FIXED", "~~~"),
        ("````text", "```\nFINDING 1: FIXED", "````"),
    ] {
        let example = format!("{open}\n{inside}\n{close}\n");
        let missing = parse_lead(&example, &[(NonZeroU32::MIN, Severity::Blocker, false)]);
        assert!(
            matches!(
                missing,
                Err(ResponseError {
                    number: NonZeroU32::MIN,
                    ..
                })
            ),
            "{example}"
        );
        assert_eq!(
            parse_lead(
                &format!("{example}FINDING 1: FIXED\nSTATUS: DONE"),
                &[(NonZeroU32::MIN, Severity::Blocker, false)]
            )
            .unwrap(),
            vec![(NonZeroU32::MIN, Response::Fixed)]
        );
    }
}

#[test]
fn inline_backticks_do_not_hide_the_real_lead_answer() {
    assert_eq!(
        parse_lead(
            "```example `inline`\nFINDING 1: FIXED\nSTATUS: DONE",
            &[(NonZeroU32::MIN, Severity::Blocker, false)]
        )
        .unwrap(),
        vec![(NonZeroU32::MIN, Response::Fixed)]
    );
}

#[test]
fn correction_reasons_identify_the_allowed_protocol_without_agent_instructions() {
    for (message, severity, used, expected) in [
        ("FINDING 7: FIXED", Severity::Major, false, Correction::OnePerFinding),
        ("FINDING 1: WRONG", Severity::Major, false, Correction::FixedOrExplained),
        ("STATUS: DONE", Severity::Major, false, Correction::FixedOrExplained),
        (
            "FINDING 1: EXPLAINED — why",
            Severity::Major,
            true,
            Correction::FixedOnly,
        ),
        (
            "FINDING 1: EXPLAINED — why",
            Severity::Blocker,
            false,
            Correction::FixedOnly,
        ),
        ("STATUS: DONE", Severity::Blocker, false, Correction::FixedOnly),
    ] {
        let error = parse_lead(message, &[(NonZeroU32::MIN, severity, used)]).unwrap_err();
        assert_eq!(error.correction, expected, "{message}");
    }
}

#[test]
fn fixed_accepts_optional_reason_separators() {
    for suffix in ["", " — vérifié", " - vérifié", ": vérifié"] {
        assert_eq!(
            parse_lead(
                &format!("FINDING 1: FIXED{suffix}\nSTATUS: DONE"),
                &[(NonZeroU32::MIN, Severity::Blocker, false)]
            ),
            Ok(vec![(NonZeroU32::MIN, Response::Fixed)])
        );
    }
}

#[test]
fn six_answers_include_two_optional_minor_findings() {
    let open: Vec<_> = (15..=20)
        .map(|n| {
            (
                NonZeroU32::new(n).unwrap(),
                if n >= 19 { Severity::Minor } else { Severity::Blocker },
                false,
            )
        })
        .collect();
    let text = "FINDING 15: FIXED\nFINDING 16: FIXED\nFINDING 17: FIXED\nFINDING 18: FIXED\nFINDING 19: FIXED\nFINDING 20: FIXED\nSTATUS: DONE";
    assert_eq!(
        parse_lead(text, &open),
        Ok(open.iter().map(|(n, _, _)| (*n, Response::Fixed)).collect())
    );
    assert_eq!(
        parse_lead("FINDING 1: advisory only", &[(NonZeroU32::MIN, Severity::Minor, false)]),
        Ok(vec![])
    );
    assert_eq!(
        parse_lead("STATUS: DONE", &[(NonZeroU32::MIN, Severity::Minor, false)]),
        Ok(vec![])
    );
}

#[test]
fn lead_diagnostics_retain_the_refused_number_line_and_mistake() {
    use crate::review_findings::{AnswerDiagnostic, AnswerMistake};
    for (text, source_line, number, mistake) in [
        (
            "  FINDING 99: FIXED",
            Some("  FINDING 99: FIXED"),
            99,
            AnswerMistake::UnknownNumber,
        ),
        (
            "FINDING 1: FIXED\nFINDING 1: FIXED",
            Some("FINDING 1: FIXED"),
            1,
            AnswerMistake::Duplicate,
        ),
        (
            "STATUS: DONE\nFINDING 1: FIXED",
            Some("FINDING 1: FIXED"),
            1,
            AnswerMistake::AfterStatus,
        ),
        (
            "FINDING 1: EXPLAINED — why",
            Some("FINDING 1: EXPLAINED — why"),
            1,
            AnswerMistake::NotAllowed,
        ),
        ("STATUS: DONE", None, 1, AnswerMistake::Missing),
    ] {
        let error = parse_lead(text, &[(NonZeroU32::MIN, Severity::Blocker, false)]).unwrap_err();
        assert_eq!(
            error.diagnostic(),
            Some(&AnswerDiagnostic {
                source_line: source_line.map(str::to_owned),
                number: NonZeroU32::new(number),
                mistake,
            }),
            "{text}"
        );
    }
}

#[test]
fn six_line_reproduction_keeps_finding_twenty_on_source_line_fifteen() {
    let open = [
        (NonZeroU32::new(15).unwrap(), Severity::Blocker, false),
        (NonZeroU32::new(16).unwrap(), Severity::Major, false),
        (NonZeroU32::new(17).unwrap(), Severity::Major, false),
        (NonZeroU32::new(18).unwrap(), Severity::Minor, false),
        (NonZeroU32::new(19).unwrap(), Severity::Major, false),
        (NonZeroU32::new(20).unwrap(), Severity::Minor, false),
    ];
    let text = format!(
        "{}FINDING 15: FIXED\nFINDING 16: FIXED\nFINDING 17: FIXED\nFINDING 18: FIXED\nFINDING 19: FIXED\nFINDING 20: FIXED\nSTATUS: DONE",
        "\n".repeat(9)
    );
    assert_eq!(text.lines().nth(14), Some("FINDING 20: FIXED"));
    assert_eq!(
        parse_lead(&text, &open),
        Ok(open.iter().map(|(n, _, _)| (*n, Response::Fixed)).collect())
    );
}

#[test]
fn invalid_numeric_labels_remain_in_the_refused_source_without_inventing_a_number() {
    use crate::review_findings::{AnswerDiagnostic, AnswerMistake};
    for label in ["0", "café", "4294967296"] {
        let text = format!("  FINDING {label}: FIXED");
        let error = parse_lead(&text, &[(NonZeroU32::MIN, Severity::Blocker, false)]).unwrap_err();
        assert_eq!(
            error.detail,
            AnswerDiagnostic {
                source_line: Some(text),
                number: None,
                mistake: AnswerMistake::UnknownNumber,
            }
        );
    }
}
