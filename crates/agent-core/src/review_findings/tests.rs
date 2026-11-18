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
