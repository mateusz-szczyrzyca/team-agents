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
