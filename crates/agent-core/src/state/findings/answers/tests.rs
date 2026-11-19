//! The answers offered for each open finding, and a report that gives any of them is read.
use super::*;
use crate::state::State;
use crate::status::ReviewVerdict;

/// The ledger of item 1.1 of the evidence: finding 28 with a pending explanation, 17 accepted and changed since, 3, 18
/// and 19 rejected, 27 a blocker.
fn evidence() -> Findings {
    let state = State::from_json(include_str!("../../../../tests/fixtures/state-split-explanations.json")).unwrap();
    state.teams()[0]
        .item("1.1".parse().unwrap())
        .unwrap()
        .findings()
        .unwrap()
        .clone()
}

fn n(number: u32) -> NonZeroU32 {
    NonZeroU32::new(number).unwrap()
}

fn answers(number: u32, decision: Decision, raise: Raise) -> VerifierAnswers {
    VerifierAnswers {
        number: n(number),
        decision,
        raise,
    }
}

#[test]
fn every_open_finding_of_the_evidence_has_the_answers_its_state_allows() {
    use Decision::{Explanation, Outcome};
    use Raise::{Never, ToBlocker, ToMajorOrBlocker};

    assert_eq!(
        evidence().verifier_answers(),
        vec![
            answers(3, Outcome, ToBlocker),
            answers(8, Outcome, ToMajorOrBlocker),
            answers(17, Outcome, ToMajorOrBlocker),
            answers(18, Outcome, ToBlocker),
            answers(19, Outcome, ToBlocker),
            answers(20, Outcome, ToBlocker),
            answers(21, Outcome, ToMajorOrBlocker),
            answers(25, Outcome, ToMajorOrBlocker),
            answers(26, Outcome, ToMajorOrBlocker),
            answers(27, Outcome, Never),
            answers(28, Explanation, ToBlocker),
        ]
    );
}

#[test]
fn an_accepted_explanation_whose_file_did_not_change_may_not_be_raised() {
    let mut ledger = evidence();
    ledger.disputes_mut().unwrap().changed_findings.remove(&n(17));

    let offered = ledger.verifier_answers();

    assert_eq!(
        offered.iter().find(|answers| answers.number == n(17)),
        Some(&answers(17, Decision::Outcome, Raise::Never))
    );
}

#[test]
fn a_pull_request_before_explanations_offers_resolved_or_open_only() {
    let legacy = Findings::new(evidence().entries.clone()).unwrap();

    let offered = legacy.verifier_answers();

    assert_eq!(offered.len(), 11);
    assert!(
        offered
            .iter()
            .all(|answers| answers.decision == Decision::Outcome && answers.raise == Raise::Never),
        "{offered:?}"
    );
}

/// Every answer line `offered` names for its finding.
fn forms(offered: &VerifierAnswers) -> Vec<String> {
    let mut forms = match offered.decision {
        Decision::Explanation => vec!["ACCEPTED — it holds — ünïcode", "REJECTED — it does not hold"],
        Decision::Outcome => vec!["RESOLVED", "OPEN — the check still fails"],
    };
    match offered.raise {
        Raise::Never => {}
        Raise::ToBlocker => forms.push("RAISED [BLOCKER] — it breaks the build"),
        Raise::ToMajorOrBlocker => {
            forms.push("RAISED [MAJOR] — it deviates from the rule");
            forms.push("RAISED [BLOCKER] — it breaks the build");
        }
    }
    forms
        .into_iter()
        .map(|form| format!("FINDING {}: {form}", offered.number))
        .collect()
}

/// A report that answers each finding with the line `pick` chooses among its forms, and adds a new blocker, so that
/// `REQUEST_CHANGES` fits whatever the answers are.
fn report(lines: &[String]) -> String {
    format!(
        "VERDICT: REQUEST_CHANGES\nSUMMARY: checked\n\n{}\n\nFINDINGS\n1. [BLOCKER] ERR-3 src/lib.rs:1 — a new problem → fix it\n   CHECK: run it\n\nSTATUS: DONE\n",
        lines.join("\n")
    )
}

#[test]
fn a_report_that_gives_any_answer_its_brief_offers_for_every_finding_is_read() {
    let mut unchanged = evidence();
    unchanged.disputes_mut().unwrap().changed_findings.clear();
    let ledgers = [
        evidence(),
        unchanged,
        Findings::new(evidence().entries.clone()).unwrap(),
    ];
    for ledger in &ledgers {
        let offered: Vec<Vec<String>> = ledger.verifier_answers().iter().map(forms).collect();
        let first: Vec<String> = offered.iter().map(|forms| forms[0].clone()).collect();
        // Each form of each finding once, the others with their first form; then every finding with its k-th form.
        let mut reports = Vec::new();
        for (index, forms) in offered.iter().enumerate() {
            for form in forms {
                let mut lines = first.clone();
                lines[index].clone_from(form);
                reports.push(lines);
            }
        }
        let most = offered.iter().map(Vec::len).max().unwrap();
        for k in 0..most {
            reports.push(offered.iter().map(|forms| forms[k % forms.len()].clone()).collect());
        }
        for lines in reports {
            let text = report(&lines);

            let read = ledger.parse_report(&text, ReviewVerdict::RequestChanges);

            assert!(
                matches!(&read, Ok(report) if report.answers.len() == offered.len()),
                "{read:?}\n{text}"
            );
        }
    }
}

#[test]
fn an_answer_the_brief_does_not_offer_is_refused() {
    let ledger = evidence();
    let lines: Vec<String> = ledger
        .verifier_answers()
        .iter()
        .map(|offered| {
            if offered.number == n(28) {
                "FINDING 28: RESOLVED".to_owned()
            } else {
                forms(offered)[0].clone()
            }
        })
        .collect();

    let refused = ledger
        .parse_report(&report(&lines), ReviewVerdict::RequestChanges)
        .unwrap_err();

    assert_eq!(refused.detail.number, Some(n(28)));
}
