//! Multiple opinions settle together; confirmed findings remain in the same fix pass.
use super::*;
use crate::review_response::opinion::OpinionReport;
use crate::state::State;
use crate::state::{OpinionRequest, RecordedFinding};

#[test]
fn waits_for_every_opinion_then_fixes_confirmations_and_verifies_only_a_new_tip() {
    let state = State::from_json(include_str!(
        "../../../tests/fixtures/state-before-second-opinions.json"
    ))
    .unwrap();
    let first = state.teams()[0].items()[1].findings().unwrap().entries()[0].clone();
    let second = NonZeroU32::new(2).unwrap();
    let mut findings = Findings::new(vec![
        first.clone(),
        RecordedFinding {
            number: second,
            ..first
        },
    ])
    .unwrap();
    let pass = NonZeroU32::MIN;
    let task: crate::ids::TaskId = "task-20261111-193007-k3x9".parse().unwrap();
    for number in [pass, second] {
        findings
            .request_opinion(
                number,
                OpinionRequest {
                    pass,
                    head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
                    reason: "unreachable".to_owned().try_into().unwrap(),
                },
            )
            .unwrap();
        findings
            .assign_opinion(number, "inv-opus".parse().unwrap(), task.clone())
            .unwrap();
    }
    assert_eq!(
        after_opinions(&findings, pass, &BTreeSet::new(), Tip::Changed),
        Next::AwaitOpinions
    );
    findings
        .record_opinion(
            pass,
            &task,
            OpinionReport {
                outcome: Outcome::Confirmed,
                evidence: "path executes".to_owned().try_into().unwrap(),
            },
        )
        .unwrap();
    assert_eq!(
        after_opinions(&findings, pass, &BTreeSet::new(), Tip::Changed),
        Next::AwaitOpinions
    );
    findings
        .record_opinion(
            second,
            &task,
            OpinionReport {
                outcome: Outcome::NotConfirmed,
                evidence: "path is unreachable".to_owned().try_into().unwrap(),
            },
        )
        .unwrap();
    assert_eq!(
        after_opinions(&findings, pass, &BTreeSet::new(), Tip::Changed),
        Next::FixConfirmed(vec![pass])
    );
    assert_eq!(
        after_opinions(&findings, pass, &BTreeSet::from([pass]), Tip::Unchanged),
        Next::AwaitTip
    );
    assert_eq!(
        after_opinions(&findings, pass, &BTreeSet::from([pass]), Tip::Changed),
        Next::Verify
    );
}

#[test]
fn all_unconfirmed_with_no_code_change_decides_without_a_tip_or_verifiers() {
    let state = State::from_json(include_str!(
        "../../../tests/fixtures/state-before-second-opinions.json"
    ))
    .unwrap();
    let mut findings = state.teams()[0].items()[1].findings().unwrap().clone();
    let n = NonZeroU32::MIN;
    let task: crate::ids::TaskId = "task-20261111-193007-k3x9".parse().unwrap();
    findings
        .request_opinion(
            n,
            OpinionRequest {
                pass: n,
                head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
                reason: "dead branch".to_owned().try_into().unwrap(),
            },
        )
        .unwrap();
    assert_eq!(
        after_opinions(&findings, n, &BTreeSet::new(), Tip::Unchanged),
        Next::AwaitOpinions
    );
    findings
        .assign_opinion(n, "inv-opus".parse().unwrap(), task.clone())
        .unwrap();
    findings
        .record_opinion(
            n,
            &task,
            OpinionReport {
                outcome: Outcome::NotConfirmed,
                evidence: "read branch".to_owned().try_into().unwrap(),
            },
        )
        .unwrap();
    assert_eq!(
        after_opinions(&findings, n, &BTreeSet::new(), Tip::Unchanged),
        Next::DecideUnchanged
    );
    assert_eq!(
        after_opinions(&findings, n, &BTreeSet::new(), Tip::Changed),
        Next::Verify
    );
    assert_eq!(
        after_opinions(&findings, NonZeroU32::new(2).unwrap(), &BTreeSet::new(), Tip::Unchanged),
        Next::AwaitTip
    );
}
