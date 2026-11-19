//! Ledgers of one pull request that differ are brought together by the most advanced record of each finding.
use super::*;
use crate::review_findings::Severity;
use crate::state::{Explanation, State};
use crate::status::ReviewVerdict;
use std::collections::BTreeMap;

/// The state of the evidence: items 1.1 and 2.1 on one pull request, finding 28 explained in 1.1 only.
fn evidence() -> State {
    State::from_json(include_str!("../../../../tests/fixtures/state-split-explanations.json")).unwrap()
}

/// The ledgers of items 1.1 and 2.1 of the evidence, in that order.
fn evidence_ledgers() -> Vec<Findings> {
    let state = evidence();
    let team = &state.teams()[0];
    ["1.1", "2.1"]
        .iter()
        .map(|id| team.item(id.parse().unwrap()).unwrap().findings().unwrap().clone())
        .collect()
}

fn n(number: u32) -> NonZeroU32 {
    NonZeroU32::new(number).unwrap()
}

fn explanation(status: ExplanationStatus) -> Explanation {
    Explanation {
        why: "the units are explicit — ünïcode".into(),
        status,
        votes: BTreeMap::new(),
    }
}

/// The explanations of `ledger`.
fn explanations(ledger: &Findings) -> &BTreeMap<NonZeroU32, Explanation> {
    &ledger.disputes().unwrap().explanations
}

/// A pair of equal ledgers of the evidence (both as item 1.1), whose finding 3 is then set apart in the second.
fn apart(change: impl FnOnce(&mut Findings)) -> Vec<Findings> {
    let first = evidence_ledgers().remove(0);
    let mut second = first.clone();
    change(&mut second);
    vec![first, second]
}

#[test]
fn the_pending_explanation_of_the_evidence_holds_for_both_items() {
    let mut ledgers = evidence_ledgers();
    assert!(!explanations(&ledgers[1]).contains_key(&n(28)));

    let repaired = reconcile(&mut ledgers);

    assert_eq!(
        repaired,
        vec![Reconciled {
            number: n(28),
            explanation: Some(ExplanationStatus::Pending),
            status: FindingStatus::Open,
        }]
    );
    assert_eq!(explanations(&ledgers[0]), explanations(&ledgers[1]));
    assert_eq!(
        explanations(&ledgers[1]).get(&n(28)).map(|e| e.status),
        Some(ExplanationStatus::Pending)
    );
    assert_eq!(ledgers[0].entries(), ledgers[1].entries());
}

#[test]
fn the_report_of_the_evidence_is_read_alike_for_both_items_once_they_are_repaired() {
    let report = include_str!("../../../../tests/fixtures/report-split-explanations.txt");
    let mut ledgers = evidence_ledgers();
    let refused = ledgers[1]
        .parse_report(report, ReviewVerdict::RequestChanges)
        .unwrap_err();
    assert_eq!((refused.line, refused.detail.number), (19, Some(n(28))));

    reconcile(&mut ledgers);

    let first = ledgers[0].parse_report(report, ReviewVerdict::RequestChanges).unwrap();
    let second = ledgers[1].parse_report(report, ReviewVerdict::RequestChanges).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.findings.len(), 2);
    assert_eq!(first.answers.len(), 11);
}

#[test]
fn ledgers_that_agree_are_not_touched() {
    let mut ledgers = apart(|_| {});
    let before = ledgers.clone();

    assert_eq!(reconcile(&mut ledgers), vec![]);
    assert_eq!(ledgers, before);
}

#[test]
fn a_single_ledger_and_none_are_not_touched() {
    let mut one = vec![evidence_ledgers().remove(1)];
    let before = one.clone();
    assert_eq!(reconcile(&mut one), vec![]);
    assert_eq!(one, before);
    assert_eq!(reconcile(&mut []), vec![]);
}

#[test]
fn the_most_advanced_explanation_wins_whichever_ledger_holds_it() {
    use ExplanationStatus::{Accepted, Pending, Rejected};
    // (in the first ledger, in the second, what both hold after)
    let cases = [
        (None, Some(Pending), Pending),
        (Some(Pending), None, Pending),
        (Some(Pending), Some(Accepted), Accepted),
        (Some(Accepted), Some(Pending), Accepted),
        (Some(Accepted), Some(Rejected), Rejected),
        (Some(Rejected), Some(Accepted), Rejected),
    ];
    for (first, second, after) in cases {
        let set = |ledger: &mut Findings, status: Option<ExplanationStatus>| {
            let disputes = ledger.disputes_mut().unwrap();
            match status {
                Some(status) => disputes.explanations.insert(n(3), explanation(status)),
                None => disputes.explanations.remove(&n(3)),
            };
        };
        let mut ledgers = evidence_ledgers();
        let mut copy = ledgers.remove(0);
        set(&mut copy, first);
        let mut other = copy.clone();
        set(&mut other, second);
        let mut ledgers = vec![copy, other];

        let repaired = reconcile(&mut ledgers);

        assert_eq!(
            repaired,
            vec![Reconciled {
                number: n(3),
                explanation: Some(after),
                status: FindingStatus::Open,
            }],
            "{first:?} and {second:?}"
        );
        for ledger in &ledgers {
            assert_eq!(explanations(ledger).get(&n(3)).map(|e| e.status), Some(after));
        }
    }
}

#[test]
fn an_accepted_explanation_brings_its_lowered_severity_with_it() {
    let mut ledgers = apart(|ledger| {
        ledger
            .disputes_mut()
            .unwrap()
            .explanations
            .insert(n(28), explanation(ExplanationStatus::Accepted));
        let entry = ledger.entries.iter_mut().find(|entry| entry.number == n(28)).unwrap();
        entry.finding.severity = Severity::Minor;
    });
    ledgers.reverse();

    reconcile(&mut ledgers);

    for ledger in &ledgers {
        let entry = ledger.entries().iter().find(|entry| entry.number == n(28)).unwrap();
        assert_eq!(entry.finding.severity, Severity::Minor);
    }
    assert_eq!(ledgers[0], ledgers[1]);
}

#[test]
fn a_closed_finding_wins_over_the_same_finding_still_open() {
    let mut ledgers = apart(|ledger| {
        let entry = ledger.entries.iter_mut().find(|entry| entry.number == n(8)).unwrap();
        entry.status = FindingStatus::Resolved;
    });

    let repaired = reconcile(&mut ledgers);

    assert_eq!(
        repaired,
        vec![Reconciled {
            number: n(8),
            explanation: None,
            status: FindingStatus::Resolved,
        }]
    );
    assert_eq!(ledgers[0].entries(), ledgers[1].entries());
}

#[test]
fn every_differing_finding_is_reported_once_in_number_order() {
    let mut ledgers = evidence_ledgers();
    let entry = ledgers[0]
        .entries
        .iter_mut()
        .find(|entry| entry.number == n(8))
        .unwrap();
    entry.status = FindingStatus::Resolved;
    let third = ledgers[1].clone();
    ledgers.push(third);

    let repaired = reconcile(&mut ledgers);

    assert_eq!(repaired.iter().map(|r| r.number).collect::<Vec<_>>(), vec![n(8), n(28)]);
    for ledger in &ledgers[1..] {
        assert_eq!(ledger.entries(), ledgers[0].entries());
        assert_eq!(explanations(ledger), explanations(&ledgers[0]));
    }
}

#[test]
fn ledgers_of_the_protocol_before_explanations_are_left_alone() {
    let legacy = Findings::new(evidence_ledgers().remove(0).entries.clone()).unwrap();
    let mut ledgers = vec![legacy, evidence_ledgers().remove(1)];
    let before = ledgers.clone();

    assert_eq!(reconcile(&mut ledgers), vec![]);
    assert_eq!(ledgers, before);
}
