//! Stable numbering and unanimous verification through the ledger API.
use super::*;
use crate::state::{ReviewOutcome, Timestamp};
fn review(name: &str, n: &str) -> Review {
    Review {
        reviewer: name.parse().unwrap(),
        task: format!("task-20261111-153002-{n}").parse().unwrap(),
        requested_at: "2026-11-11T15:30:02Z".parse::<Timestamp>().unwrap(),
        outcome: ReviewOutcome::Pending,
    }
}
fn finding(severity: Severity, text: &str) -> Finding {
    Finding {
        severity,
        rule: "ERR-3".into(),
        place: "src/lib.rs:1".into(),
        text: text.into(),
        check: "read src/lib.rs:1".into(),
    }
}
#[test]
fn numbers_follow_request_order_over_two_reports_and_two_passes() {
    let reviews = [review("inv-codex", "k3x9"), review("inv-opus", "k3x8")];
    let mut ledger = Findings::default();
    ledger.record(
        reviews[1].task.clone(),
        Report {
            findings: vec![finding(Severity::Major, "second")],
            answers: vec![],
        },
    );
    ledger.record(
        reviews[0].task.clone(),
        Report {
            findings: vec![finding(Severity::Minor, "first")],
            answers: vec![],
        },
    );
    let mut ledger: Findings = serde_json::from_str(&serde_json::to_string(&ledger).unwrap()).unwrap();
    ledger.finish(&reviews, NonZeroU32::MIN).unwrap();
    assert_eq!(
        ledger
            .entries
            .iter()
            .map(|entry| (
                entry.number.get(),
                entry.finding.text.as_str(),
                entry.reporter.as_str(),
                entry.pass.get(),
                entry.status
            ))
            .collect::<Vec<_>>(),
        vec![
            (1, "first", "inv-codex", 1, FindingStatus::Open),
            (2, "second", "inv-opus", 1, FindingStatus::Open)
        ]
    );
    for review in &reviews {
        ledger.record(
            review.task.clone(),
            Report {
                findings: vec![finding(Severity::Minor, review.reviewer.as_str())],
                answers: vec![
                    (NonZeroU32::MIN, Answer::Resolved),
                    (NonZeroU32::new(2).unwrap(), Answer::Resolved),
                ],
            },
        );
    }
    ledger.finish(&reviews, NonZeroU32::new(2).unwrap()).unwrap();
    assert_eq!(
        ledger
            .entries
            .iter()
            .map(|entry| (entry.number.get(), entry.pass.get(), entry.status))
            .collect::<Vec<_>>(),
        vec![
            (1, 1, FindingStatus::Resolved),
            (2, 1, FindingStatus::Resolved),
            (3, 2, FindingStatus::Open),
            (4, 2, FindingStatus::Open)
        ]
    );
    assert_eq!(
        serde_json::from_str::<Findings>(&serde_json::to_string(&ledger).unwrap()).unwrap(),
        ledger
    );
}
#[test]
fn one_open_vote_keeps_the_finding_open_and_missing_reports_do_not_resolve() {
    let reviews = [review("inv-codex", "k3x9"), review("inv-opus", "k3x8")];
    let mut ledger = Findings::default();
    ledger.record(
        reviews[0].task.clone(),
        Report {
            findings: vec![finding(Severity::Major, "broken")],
            answers: vec![],
        },
    );
    ledger.finish(&reviews, NonZeroU32::MIN).unwrap();
    assert_eq!(ledger.open(), vec![(NonZeroU32::MIN, Severity::Major)]);
    ledger.record(
        reviews[0].task.clone(),
        Report {
            findings: vec![],
            answers: vec![(NonZeroU32::MIN, Answer::Resolved)],
        },
    );
    ledger.record(
        reviews[1].task.clone(),
        Report {
            findings: vec![],
            answers: vec![(
                NonZeroU32::MIN,
                Answer::Open {
                    why: "still broken".into(),
                },
            )],
        },
    );
    ledger.finish(&reviews, NonZeroU32::new(2).unwrap()).unwrap();
    assert_eq!(ledger.open(), vec![(NonZeroU32::MIN, Severity::Major)]);
    ledger.record(
        reviews[0].task.clone(),
        Report {
            findings: vec![],
            answers: vec![(NonZeroU32::MIN, Answer::Resolved)],
        },
    );
    ledger.finish(&reviews, NonZeroU32::new(3).unwrap()).unwrap();
    assert_eq!(ledger.open(), vec![(NonZeroU32::MIN, Severity::Major)]);
}

fn recorded(number: u32) -> RecordedFinding {
    RecordedFinding {
        number: NonZeroU32::new(number).unwrap(),
        finding: finding(Severity::Minor, "text"),
        reporter: "inv-codex".parse().unwrap(),
        pass: NonZeroU32::MIN,
        status: FindingStatus::Open,
    }
}

#[test]
fn a_ledger_is_built_only_from_entries_numbered_one_to_n_in_order() {
    for (numbers, expected) in [
        (vec![1, 1], InvalidFindingNumbers { expected: 2, found: 1 }),
        (vec![2, 1], InvalidFindingNumbers { expected: 1, found: 2 }),
        (vec![1, 3], InvalidFindingNumbers { expected: 2, found: 3 }),
        (vec![2], InvalidFindingNumbers { expected: 1, found: 2 }),
    ] {
        let entries = numbers.iter().map(|n| recorded(*n)).collect();
        assert_eq!(Findings::new(entries), Err(expected), "numbers {numbers:?}");
    }
}

#[test]
fn a_ledger_of_entries_numbered_in_order_keeps_them() {
    for count in [0, 1, 3] {
        let entries: Vec<_> = (1..=count).map(recorded).collect();
        let ledger = Findings::new(entries.clone()).unwrap();
        assert_eq!(ledger.entries(), entries.as_slice());
        assert_eq!(ledger.comment, None);
    }
}
