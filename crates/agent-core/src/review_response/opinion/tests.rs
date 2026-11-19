//! Second-opinion syntax and eligibility through the pure public API.
use super::*;

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
                    assert_eq!(
                        result,
                        Err(OpinionError::Forbidden {
                            number: NonZeroU32::MIN
                        })
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
    for text in [
        "FINDING 1: SECOND OPINION — reason",
        "FINDING 1: SECOND OPINION — reason\nFINDING 1: FIXED\nFINDING 2: FIXED",
        "FINDING 9: SECOND OPINION — reason",
        "STATUS: DONE\nFINDING 1: SECOND OPINION — reason\nFINDING 2: FIXED",
        "```\nFINDING 1: SECOND OPINION — reason\n```\nFINDING 2: FIXED",
    ] {
        assert!(
            matches!(
                parse_lead(text, &open, Permission::Enabled),
                Err(OpinionError::Response(_))
            ),
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
