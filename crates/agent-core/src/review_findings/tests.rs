//! Report parsing through its public boundary.
use super::*;

fn number(n: u32) -> NonZeroU32 {
    NonZeroU32::new(n).unwrap()
}

#[test]
fn reads_all_severities_and_markup_around_the_severity() {
    for (bracket, severity) in [
        ("[BLOCKER]", Severity::Blocker),
        ("**[MAJOR]**", Severity::Major),
        ("`[MINOR]`", Severity::Minor),
    ] {
        let text =
            format!("FINDINGS\n1. {bracket} ERR-3 crates/é.rs:42 — problème → fix\n   CHECK: cargo test check\n");
        let verdict = if severity.blocks() {
            ReviewVerdict::RequestChanges
        } else {
            ReviewVerdict::Approve
        };
        let report = parse_report(&text, verdict, &[]).unwrap();
        assert_eq!(
            report.findings,
            vec![Finding {
                severity,
                rule: "ERR-3".into(),
                place: "crates/é.rs:42".into(),
                text: "problème → fix".into(),
                check: "cargo test check".into()
            }]
        );
    }
}

#[test]
fn refuses_missing_unknown_or_rule_id_severities_and_missing_checks() {
    for (line, kind, source_line) in [
        (
            "1. ERR-3 x:1 — problem → fix\n CHECK: read x",
            ReportProblem::Finding,
            2,
        ),
        (
            "1. [CRITICAL] ERR-3 x:1 — problem → fix\n CHECK: read x",
            ReportProblem::Finding,
            2,
        ),
        (
            "1. [ERR-3] x:1 — problem → fix\n CHECK: read x",
            ReportProblem::Finding,
            2,
        ),
        ("1. [MAJOR] ERR-3 x:1 — problem → fix", ReportProblem::Check, 2),
        ("1. [MAJOR] ERR-3 x:1 — problem → fix\n CHECK:", ReportProblem::Check, 2),
    ] {
        assert_eq!(
            parse_report(&format!("FINDINGS\n{line}"), ReviewVerdict::RequestChanges, &[]),
            Err(ReportError {
                line: source_line,
                kind
            })
        );
    }
}

#[test]
fn refuses_verdicts_that_contradict_new_findings() {
    for (text, verdict) in [
        (
            "FINDINGS\n1. [MAJOR] ERR-3 x:1 — p → f\n CHECK: x",
            ReviewVerdict::Approve,
        ),
        ("FINDINGS\nNone.", ReviewVerdict::RequestChanges),
        (
            "FINDINGS\n1. [MINOR] API-1 x:1 — p → f\n CHECK: x",
            ReviewVerdict::RequestChanges,
        ),
    ] {
        assert_eq!(
            parse_report(text, verdict, &[]).unwrap_err().kind,
            ReportProblem::Verdict
        );
    }
}

#[test]
fn later_pass_requires_exactly_one_answer_for_every_open_finding_before_new_ones() {
    let open = [(number(3), Severity::Major), (number(4), Severity::Minor)];
    for text in [
        "FINDING 3: RESOLVED",
        "FINDING 3: RESOLVED\nFINDING 3: RESOLVED\nFINDING 4: RESOLVED",
        "FINDING 3: RESOLVED\nFINDING 4: RESOLVED\nFINDING 5: RESOLVED",
        "FINDING 3: OPEN\nFINDING 4: RESOLVED",
        "FINDINGS\n1. [MINOR] API-1 x:1 — p → f\n CHECK: x\nFINDING 3: RESOLVED\nFINDING 4: RESOLVED",
    ] {
        assert_eq!(
            parse_report(text, ReviewVerdict::Approve, &open).unwrap_err().kind,
            ReportProblem::Answer,
            "{text}"
        );
    }
}

#[test]
fn later_pass_checks_old_severities_and_keeps_open_reasons() {
    let open = [(number(3), Severity::Major), (number(4), Severity::Minor)];
    let text = "FINDING 3: OPEN — still broken\nFINDING 4: RESOLVED\nFINDINGS\nNone.";
    assert_eq!(
        parse_report(text, ReviewVerdict::Approve, &open).unwrap_err().kind,
        ReportProblem::Verdict
    );
    assert_eq!(
        parse_report(text, ReviewVerdict::RequestChanges, &open)
            .unwrap()
            .answers,
        vec![
            (
                number(3),
                Answer::Open {
                    why: "still broken".into()
                }
            ),
            (number(4), Answer::Resolved)
        ]
    );
    assert_eq!(
        parse_report(
            "FINDING 3: RESOLVED\nFINDING 4: OPEN — optional",
            ReviewVerdict::Approve,
            &open
        )
        .unwrap()
        .answers,
        vec![
            (number(3), Answer::Resolved),
            (number(4), Answer::Open { why: "optional".into() })
        ]
    );
}

#[test]
fn finding_text_and_checks_preserve_code_delimiters_and_globs() {
    let report = parse_report(
        "FINDINGS\n1. **[MINOR]** API-1 x:1 — replace `&String` → `&str`\n   CHECK: rg '*' src",
        ReviewVerdict::Approve,
        &[],
    )
    .unwrap();
    assert_eq!(
        report.findings,
        vec![Finding {
            severity: Severity::Minor,
            rule: "API-1".into(),
            place: "x:1".into(),
            text: "replace `&String` → `&str`".into(),
            check: "rg '*' src".into(),
        }]
    );
}

#[test]
fn refuses_a_numbered_finding_outside_the_findings_section() {
    for text in [
        "VERDICT: APPROVE\nSUMMARY: ok\n1. [MAJOR] ERR-3 x:1 — problem → fix\n   CHECK: read x\n",
        "VERDICT: APPROVE\nBLOCKERS\n1. [COR] x:1 — problem → fix\n",
    ] {
        assert_eq!(
            parse_report(text, ReviewVerdict::Approve, &[]),
            Err(ReportError {
                line: 3,
                kind: ReportProblem::Finding
            }),
            "{text}"
        );
    }
}

#[test]
fn refuses_an_unnumbered_finding_line_anywhere() {
    for text in [
        "VERDICT: APPROVE\nFINDINGS\n[BLOCKER] ERR-3 x:1 — problem → fix\nCHECK: read x\n",
        "VERDICT: APPROVE\nSUMMARY: ok\n**[MAJOR]** ERR-3 x:1 — problem → fix\n",
    ] {
        let line = text
            .lines()
            .position(|l| l.contains("[BLOCKER]") || l.contains("[MAJOR]"))
            .unwrap()
            + 1;
        assert_eq!(
            parse_report(text, ReviewVerdict::Approve, &[]),
            Err(ReportError {
                line,
                kind: ReportProblem::Finding
            }),
            "{text}"
        );
    }
}

#[test]
fn ignores_the_contents_of_code_fences() {
    let text = "VERDICT: REQUEST_CHANGES\nFINDINGS\n\
1. [MINOR] API-1 x.rs:3 — takes &String → &str\n   CHECK: grep -n '&String' x.rs\n\n\
```rust\n1. Install\n1. [BLOCKER] ERR-3 y:1 — in a fence → ignored\nFINDING 9: RESOLVED\n```\n\n\
2. [MAJOR] ERR-3 y.rs:4 — problem → fix\n   CHECK: read y.rs\n";
    let report = parse_report(text, ReviewVerdict::RequestChanges, &[]).unwrap();
    assert_eq!(
        report
            .findings
            .iter()
            .map(|f| (f.severity, f.place.as_str()))
            .collect::<Vec<_>>(),
        vec![(Severity::Minor, "x.rs:3"), (Severity::Major, "y.rs:4")]
    );
    assert_eq!(report.answers, vec![]);
}

#[test]
fn links_and_prose_that_look_like_brackets_are_not_findings() {
    let text = "VERDICT: APPROVE\nSUMMARY: see [the docs](https://example.com) and [note]\n1. [the docs](https://example.com) explain it\n";
    assert_eq!(parse_report(text, ReviewVerdict::Approve, &[]), Ok(Report::default()));
}

#[test]
fn a_longer_fence_is_closed_only_by_an_equal_or_longer_fence_of_its_character() {
    for (open, inner, close) in [
        ("````text", "```", "````"),
        ("~~~text", "```", "~~~"),
        ("`````", "````", "`````"),
        ("```", "~~~", "```"),
    ] {
        let text = format!(
            "VERDICT: APPROVE\nSUMMARY: ok\n{open}\n{inner}\n1. [BLOCKER] ERR-3 y:1 — example → ignored\n{close}\n\
FINDINGS\n1. [MAJOR] ERR-3 y.rs:4 — real problem → fix\n   CHECK: read y.rs\n"
        );
        assert_eq!(
            parse_report(&text, ReviewVerdict::Approve, &[]),
            Err(ReportError {
                line: 1,
                kind: ReportProblem::Verdict
            }),
            "{text}"
        );
        let report = parse_report(&text, ReviewVerdict::RequestChanges, &[]).unwrap();
        assert_eq!(report.findings.len(), 1, "{text}");
    }
}

#[test]
fn a_line_with_inline_triple_backticks_does_not_open_a_fence() {
    let text = "VERDICT: APPROVE\n```cargo test``` passed\n\
FINDINGS\n1. [MAJOR] ERR-3 y.rs:4 — problem → fix\n   CHECK: read y.rs\n";
    assert_eq!(
        parse_report(text, ReviewVerdict::Approve, &[]),
        Err(ReportError {
            line: 1,
            kind: ReportProblem::Verdict
        })
    );
    let report = parse_report(text, ReviewVerdict::RequestChanges, &[]).unwrap();
    assert_eq!(report.findings.len(), 1);
}

#[test]
fn a_tilde_fence_may_carry_backticks_in_its_info_string() {
    let text = "VERDICT: REQUEST_CHANGES\n~~~ `text`\n1. [BLOCKER] ERR-3 y:1 — in a fence → ignored\n~~~\n\
FINDINGS\n1. [MAJOR] ERR-3 y.rs:4 — real → fix\n   CHECK: read y.rs\n";
    let report = parse_report(text, ReviewVerdict::RequestChanges, &[]).unwrap();
    assert_eq!(report.findings.len(), 1);
}

#[test]
fn explanations_require_a_reasoned_vote_and_raising_never_lowers_severity() {
    let id = number(3);
    let open = [(id, Severity::Major)];
    for (line, verdict, expected) in [
        (
            "ACCEPTED — justified",
            ReviewVerdict::Approve,
            Answer::Accepted {
                why: "justified".into(),
            },
        ),
        (
            "REJECTED — breaks the task",
            ReviewVerdict::RequestChanges,
            Answer::Rejected {
                why: "breaks the task".into(),
            },
        ),
        (
            "RAISED [BLOCKER] — broken",
            ReviewVerdict::RequestChanges,
            Answer::Raised {
                severity: Severity::Blocker,
                why: "broken".into(),
            },
        ),
    ] {
        let parsed = super::parse_verification(&format!("FINDING 3: {line}"), verdict, &open, &[id]);
        assert_eq!(
            parsed,
            Ok(Report {
                findings: vec![],
                answers: vec![(id, expected)]
            })
        );
    }
    for (line, pending, verdict) in [
        ("ACCEPTED — why", false, ReviewVerdict::Approve),
        ("RESOLVED", true, ReviewVerdict::Approve),
        ("OPEN — why", true, ReviewVerdict::RequestChanges),
        ("ACCEPTED — ", true, ReviewVerdict::Approve),
        ("REJECTED — why", true, ReviewVerdict::Approve),
        ("RAISED [MINOR] — why", false, ReviewVerdict::Approve),
        ("RAISED [MAJOR] — why", false, ReviewVerdict::RequestChanges),
    ] {
        let explained = if pending { vec![id] } else { vec![] };
        assert!(
            matches!(
                super::parse_verification(&format!("FINDING 3: {line}"), verdict, &open, &explained),
                Err(ReportError { .. })
            ),
            "{line}"
        );
    }
    let minor = [(id, Severity::Minor)];
    assert_eq!(
        super::parse_verification(
            "FINDING 3: RAISED [MAJOR] — changed code",
            ReviewVerdict::RequestChanges,
            &minor,
            &[]
        )
        .unwrap()
        .answers,
        vec![(
            id,
            Answer::Raised {
                severity: Severity::Major,
                why: "changed code".into()
            }
        )]
    );
}

#[test]
fn malformed_explanation_votes_receive_the_explanation_protocol_correction() {
    assert_eq!(
        parse_verification(
            "FINDING 3: RESOLVED",
            ReviewVerdict::Approve,
            &[(number(3), Severity::Major)],
            &[number(3)]
        ),
        Err(ReportError {
            line: 1,
            kind: ReportProblem::DisputeAnswer
        })
    );
}

#[test]
fn resolved_accepts_optional_reason_separators() {
    for suffix in ["", " — vérifié", " - vérifié", ": vérifié"] {
        let text = format!("VERDICT: APPROVE\nFINDING 3: RESOLVED{suffix}\nSTATUS: DONE");
        assert_eq!(
            parse_verification(&text, ReviewVerdict::Approve, &[(number(3), Severity::Blocker)], &[]),
            Ok(Report {
                findings: vec![],
                answers: vec![(number(3), Answer::Resolved)]
            })
        );
    }
}
