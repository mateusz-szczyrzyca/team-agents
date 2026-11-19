//! Second-opinion syntax and eligibility through the pure public API.
use super::*;
use crate::review_response::{Correction, ResponseError};

fn finding(severity: Severity, opinion_used: bool) -> OpenFinding {
    OpenFinding {
        number: NonZeroU32::MIN,
        severity,
        explained: false,
        opinion_used,
    }
}

#[test]
fn only_unused_blockers_with_permission_accept_opinions() {
    for severity in [Severity::Blocker, Severity::Major, Severity::Minor] {
        for used in [false, true] {
            for permission in [Permission::Enabled, Permission::Disabled] {
                let result = parse_lead(
                    "FINDING 1: SECOND OPINION — café is unreachable",
                    &[finding(severity, used)],
                    permission,
                );
                if severity == Severity::Blocker && !used && permission == Permission::Enabled {
                    assert_eq!(
                        result,
                        Ok(vec![(
                            NonZeroU32::MIN,
                            LeadAnswer::SecondOpinion("café is unreachable".to_owned().try_into().unwrap())
                        )])
                    );
                } else {
                    assert!(
                        matches!(result, Err(OpinionError::Forbidden { number, .. }) if number == NonZeroU32::MIN),
                        "{result:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn reads_mixed_answers_atomically_and_retains_legacy_validation() {
    let open = [
        finding(Severity::Blocker, false),
        OpenFinding {
            number: NonZeroU32::new(2).unwrap(),
            ..finding(Severity::Major, false)
        },
    ];
    assert_eq!(
        parse_lead(
            "FINDING 1: SECOND OPINION — reason\nFINDING 2: EXPLAINED — intent",
            &open,
            Permission::Enabled
        ),
        Ok(vec![
            (
                NonZeroU32::MIN,
                LeadAnswer::SecondOpinion("reason".to_owned().try_into().unwrap())
            ),
            (NonZeroU32::new(2).unwrap(), LeadAnswer::Explained("intent".to_owned()))
        ])
    );
    assert_eq!(
        parse_lead("FINDING 1: FIXED\nFINDING 2: FIXED", &open, Permission::Enabled),
        Ok(vec![
            (NonZeroU32::MIN, LeadAnswer::Fixed),
            (NonZeroU32::new(2).unwrap(), LeadAnswer::Fixed)
        ])
    );
    for (text, line, number, correction) in [
        ("FINDING 1: SECOND OPINION — reason", 2, 2, Correction::FixedOrExplained),
        (
            "FINDING 1: SECOND OPINION — reason\nFINDING 1: FIXED\nFINDING 2: FIXED",
            2,
            1,
            Correction::FixedOnly,
        ),
        ("FINDING 9: SECOND OPINION — reason", 1, 1, Correction::OnePerFinding),
        (
            "STATUS: DONE\nFINDING 1: SECOND OPINION — reason\nFINDING 2: FIXED",
            2,
            1,
            Correction::FixedOnly,
        ),
        (
            "```\nFINDING 1: SECOND OPINION — reason\n```\nFINDING 2: FIXED",
            5,
            1,
            Correction::FixedOnly,
        ),
    ] {
        assert_eq!(
            parse_lead(text, &open, Permission::Enabled),
            Err(OpinionError::Response(ResponseError {
                line,
                number: NonZeroU32::new(number).unwrap(),
                correction,
            })),
            "{text}"
        );
    }
    assert_eq!(
        parse_lead("FINDING 1: SECOND OPINION — ", &open[..1], Permission::Enabled),
        Err(OpinionError::MissingEvidence)
    );
}

#[test]
fn reads_both_outcomes_with_markup_and_requires_opening_verdict_and_evidence() {
    for (word, outcome) in [
        ("CONFIRMED", Outcome::Confirmed),
        ("NOT CONFIRMED", Outcome::NotConfirmed),
    ] {
        for markup in ["", "**", "`"] {
            let text = format!("{markup}OPINION: {word}{markup}\nRead café.rs: the branch is unreachable.");
            assert_eq!(
                parse_report(&text),
                Ok(OpinionReport {
                    outcome,
                    evidence: "Read café.rs: the branch is unreachable."
                        .to_owned()
                        .try_into()
                        .unwrap()
                })
            );
        }
    }
    for text in [
        "",
        "Evidence only",
        "OPINION: MAYBE\nevidence",
        "```\nOPINION: CONFIRMED\n```",
        "preamble\nOPINION: CONFIRMED\nevidence",
        "OPINION: CONFIRMED\nOPINION: NOT CONFIRMED\nevidence",
    ] {
        assert_eq!(parse_report(text), Err(OpinionError::MissingVerdict));
    }
    assert_eq!(
        parse_report("OPINION: CONFIRMED\n  "),
        Err(OpinionError::MissingEvidence)
    );
    assert_eq!(Evidence::try_from(" \n".to_owned()), Err(OpinionError::MissingEvidence));
    let error = serde_json::from_str::<Evidence>("\" \"").expect_err("empty evidence must be refused");
    assert_eq!(error.classify(), serde_json::error::Category::Data);
}

#[test]
fn a_final_status_line_is_not_evidence() {
    assert_eq!(
        parse_report("OPINION: CONFIRMED\nSTATUS: DONE"),
        Err(OpinionError::MissingEvidence)
    );
    assert_eq!(
        parse_report("OPINION: NOT CONFIRMED\n\nSTATUS: DONE\n"),
        Err(OpinionError::MissingEvidence)
    );
    assert_eq!(
        parse_report("OPINION: CONFIRMED\nRead café.rs: it is unreachable.\n\nSTATUS: DONE"),
        Ok(OpinionReport {
            outcome: Outcome::Confirmed,
            evidence: "Read café.rs: it is unreachable.".to_owned().try_into().unwrap(),
        })
    );
}

#[test]
fn markup_inside_the_keyword_line_is_ignored() {
    for (text, outcome) in [
        ("**OPINION:** CONFIRMED\nevidence", Outcome::Confirmed),
        ("`OPINION:` `NOT CONFIRMED`\nevidence", Outcome::NotConfirmed),
        ("**OPINION: NOT CONFIRMED**\nevidence", Outcome::NotConfirmed),
    ] {
        assert_eq!(
            parse_report(text),
            Ok(OpinionReport {
                outcome,
                evidence: "evidence".to_owned().try_into().unwrap(),
            }),
            "{text}"
        );
    }
}

#[test]
fn a_refused_request_names_its_line_and_the_reason() {
    use Permission::{Disabled, Enabled};
    use Refusal::{AlreadyAsked, KeyDisabled, NotBlocker};
    for (severity, used, permission, reason) in [
        (Severity::Blocker, false, Disabled, KeyDisabled),
        (Severity::Blocker, true, Disabled, KeyDisabled),
        (Severity::Major, false, Disabled, KeyDisabled),
        (Severity::Blocker, true, Enabled, AlreadyAsked),
        (Severity::Major, false, Enabled, NotBlocker),
        (Severity::Minor, false, Enabled, NotBlocker),
    ] {
        let number = NonZeroU32::new(2).unwrap();
        let open = [
            finding(Severity::Blocker, false),
            OpenFinding {
                number,
                ..finding(severity, used)
            },
        ];

        let result = parse_lead(
            "FINDING 1: FIXED\nFINDING 2: SECOND OPINION — it is not so",
            &open,
            permission,
        );

        assert_eq!(
            result,
            Err(OpinionError::Forbidden {
                number,
                line: 2,
                reason
            }),
            "{severity:?} used={used} {permission:?}"
        );
    }
}

#[test]
fn minor_answers_do_not_block_opinion_protocol() {
    let open = [
        finding(Severity::Blocker, false),
        OpenFinding {
            number: NonZeroU32::new(2).unwrap(),
            ..finding(Severity::Minor, false)
        },
    ];
    for minor in [
        "FIXED",
        "SECOND OPINION — optional",
        "EXPLAINED — optional",
        "advice only",
    ] {
        let text = format!("FINDING 1: FIXED — tested\nFINDING 2: {minor}\nSTATUS: DONE");
        let mut expected = vec![(NonZeroU32::MIN, LeadAnswer::Fixed)];
        if minor == "FIXED" {
            expected.push((NonZeroU32::new(2).unwrap(), LeadAnswer::Fixed));
        }
        assert_eq!(parse_lead(&text, &open, Permission::Enabled), Ok(expected));
    }
}
