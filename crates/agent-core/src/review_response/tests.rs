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
