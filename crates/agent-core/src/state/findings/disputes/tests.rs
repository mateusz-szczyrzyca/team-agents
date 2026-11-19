//! Unanimity, rejection, persistence and the one-shot explanation rule.
use super::*;
use crate::config::Config;
use crate::review_findings::Finding;
use crate::state::{Review, ReviewOutcome, Timestamp};

fn review(name: &str, suffix: &str) -> Review {
    Review {
        reviewer: name.parse().unwrap(),
        task: format!("task-20261111-153002-{suffix}").parse().unwrap(),
        requested_at: "2026-11-11T15:30:02Z".parse::<Timestamp>().unwrap(),
        outcome: ReviewOutcome::Pending,
    }
}
fn ledger() -> (Findings, Vec<Review>) {
    let reviews = vec![review("inv-codex", "k3x9"), review("inv-opus", "k3x8")];
    let mut ledger = Findings::verifying(Config::from_toml_str("").unwrap().review().min_first_full_reviews());
    ledger.record(
        reviews[0].task.clone(),
        Report {
            findings: vec![Finding {
                severity: Severity::Major,
                rule: "TYP-3".into(),
                place: "src/lib.rs:1".into(),
                text: "units → newtype".into(),
                check: "read the signature".into(),
            }],
            answers: vec![],
        },
    );
    ledger.finish(&reviews, NonZeroU32::MIN).unwrap();
    (ledger, reviews)
}
#[test]
fn every_acceptance_downgrades_once_and_retains_each_verifier_evidence() {
    let (mut ledger, reviews) = ledger();
    ledger
        .answer_lead("FINDING 1: EXPLAINED — explicit units in the API", 1)
        .unwrap();
    assert_eq!(ledger.disputes().unwrap().explanations.len(), 1);
    for review in &reviews {
        let report = ledger
            .parse_report("FINDING 1: ACCEPTED — sufficient contract", ReviewVerdict::Approve)
            .unwrap();
        ledger.record(review.task.clone(), report);
    }
    ledger.finish(&reviews, NonZeroU32::new(2).unwrap()).unwrap();
    assert_eq!(ledger.entries()[0].finding.severity, Severity::Minor);
    assert_eq!(ledger.entries()[0].status, FindingStatus::Open);
    let explanation = &ledger.disputes().unwrap().explanations[&NonZeroU32::MIN];
    assert_eq!(
        (explanation.status, explanation.why.as_str(), explanation.votes.len()),
        (ExplanationStatus::Accepted, "explicit units in the API", 2)
    );
    assert_eq!(
        serde_json::from_str::<Findings>(&serde_json::to_string(&ledger).unwrap()).unwrap(),
        ledger
    );
}
#[test]
fn one_rejection_requires_a_fix_and_any_verifier_can_raise() {
    let (mut ledger, reviews) = ledger();
    ledger.answer_lead("FINDING 1: EXPLAINED — clear units", 1).unwrap();
    assert_eq!(ledger.disputes().unwrap().answered_pass, 1);
    for (review, line, verdict) in [
        (&reviews[0], "ACCEPTED — fine", ReviewVerdict::Approve),
        (
            &reviews[1],
            "REJECTED — caller can swap units",
            ReviewVerdict::RequestChanges,
        ),
    ] {
        ledger.record(
            review.task.clone(),
            ledger.parse_report(&format!("FINDING 1: {line}"), verdict).unwrap(),
        );
    }
    ledger.finish(&reviews, NonZeroU32::new(2).unwrap()).unwrap();
    assert_eq!(
        (
            ledger.entries()[0].finding.severity,
            ledger.disputes().unwrap().explanations[&NonZeroU32::MIN].status
        ),
        (Severity::Major, ExplanationStatus::Rejected)
    );
    assert!(matches!(
        ledger.answer_lead("FINDING 1: EXPLAINED — retry", 2),
        Err(ResponseError {
            number: NonZeroU32::MIN,
            ..
        })
    ));
    ledger.answer_lead("FINDING 1: FIXED", 2).unwrap();
    for (review, line) in [
        (&reviews[0], "RESOLVED"),
        (&reviews[1], "RAISED [BLOCKER] — still broken"),
    ] {
        let verdict = if line == "RESOLVED" {
            ReviewVerdict::Approve
        } else {
            ReviewVerdict::RequestChanges
        };
        ledger.record(
            review.task.clone(),
            ledger.parse_report(&format!("FINDING 1: {line}"), verdict).unwrap(),
        );
    }
    ledger.finish(&reviews, NonZeroU32::new(3).unwrap()).unwrap();
    assert_eq!(
        (ledger.entries()[0].finding.severity, ledger.entries()[0].status),
        (Severity::Blocker, FindingStatus::Open)
    );
    assert_eq!(ledger.disputes().unwrap().raised[0].reviewer, reviews[1].reviewer);
}
