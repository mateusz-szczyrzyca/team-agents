//! Persisted waiting, ownership, outcomes and one-shot enforcement.
use super::*;
use crate::review_findings::Finding;
use crate::state::RecordedFinding;

fn ledger(severity: Severity) -> Findings {
    Findings::new(vec![RecordedFinding {
        number: NonZeroU32::MIN,
        finding: Finding {
            severity,
            rule: "ERR-1".into(),
            place: "src/lib.rs:1".into(),
            text: "café defect".into(),
            check: "read path".into(),
        },
        reporter: "inv-codex".parse().unwrap(),
        pass: NonZeroU32::MIN,
        status: FindingStatus::Open,
    }])
    .unwrap()
}
fn request() -> OpinionRequest {
    OpinionRequest {
        pass: NonZeroU32::MIN,
        head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
        reason: "café branch is unreachable".to_owned().try_into().unwrap(),
    }
}

#[test]
fn records_each_outcome_once_and_persists_waiting_and_task_ownership() {
    for outcome in [Outcome::Confirmed, Outcome::NotConfirmed] {
        let mut ledger = ledger(Severity::Blocker);
        let n = NonZeroU32::MIN;
        let task: TaskId = "task-20261111-193007-k3x9".parse().unwrap();
        let by: InvestigatorName = "inv-opus".parse().unwrap();
        let report = OpinionReport {
            outcome,
            evidence: "read café.rs: branch is dead".to_owned().try_into().unwrap(),
        };
        assert_eq!(ledger.request_opinion(n, request()), Ok(()));
        assert_eq!(
            ledger.opinions().get(&n),
            Some(&SecondOpinion {
                request: request(),
                stage: OpinionStage::Waiting
            })
        );
        assert_eq!(
            serde_json::from_str::<Findings>(&serde_json::to_string(&ledger).unwrap()).unwrap(),
            ledger
        );
        assert_eq!(
            ledger.record_opinion(n, &task, report.clone()),
            Err(OpinionStateError { number: n })
        );
        assert_eq!(ledger.assign_opinion(n, by.clone(), task.clone()), Ok(()));
        assert_eq!(
            ledger.opinions()[&n].stage,
            OpinionStage::Assigned {
                investigator: by.clone(),
                task: task.clone()
            }
        );
        assert_eq!(
            ledger.record_opinion(n, &"task-20261111-193008-k3x9".parse().unwrap(), report.clone()),
            Err(OpinionStateError { number: n })
        );
        assert_eq!(ledger.record_opinion(n, &task, report.clone()), Ok(()));
        assert_eq!(
            ledger.opinions()[&n].stage,
            OpinionStage::Reported {
                investigator: by.clone(),
                task: task.clone(),
                report: report.clone()
            }
        );
        assert_eq!(
            ledger.entries()[0].status,
            if outcome == Outcome::Confirmed {
                FindingStatus::Open
            } else {
                FindingStatus::NotConfirmed
            }
        );
        assert_eq!(
            ledger.open(),
            if outcome == Outcome::Confirmed {
                vec![(n, Severity::Blocker)]
            } else {
                vec![]
            }
        );
        assert_eq!(
            ledger.request_opinion(n, request()),
            Err(OpinionStateError { number: n })
        );
        assert_eq!(
            ledger.record_opinion(n, &task, report),
            Err(OpinionStateError { number: n })
        );
        assert_eq!(ledger.assign_opinion(n, by, task), Err(OpinionStateError { number: n }));
        assert_eq!(
            serde_json::from_str::<Findings>(&serde_json::to_string(&ledger).unwrap()).unwrap(),
            ledger
        );
    }
}

#[test]
fn refuses_unknown_non_blocker_and_closed_findings_and_allows_pending_replacement() {
    for severity in [Severity::Major, Severity::Minor] {
        assert_eq!(
            ledger(severity).request_opinion(NonZeroU32::MIN, request()),
            Err(OpinionStateError {
                number: NonZeroU32::MIN
            })
        );
    }
    let mut ledger = ledger(Severity::Blocker);
    let absent = NonZeroU32::new(2).unwrap();
    let task = "task-20261111-193007-k3x9".parse().unwrap();
    assert_eq!(
        ledger.request_opinion(absent, request()),
        Err(OpinionStateError { number: absent })
    );
    assert_eq!(
        ledger.assign_opinion(absent, "inv-opus".parse().unwrap(), task),
        Err(OpinionStateError { number: absent })
    );
    ledger.entries.first_mut().unwrap().status = FindingStatus::Resolved;
    assert_eq!(
        ledger.request_opinion(NonZeroU32::MIN, request()),
        Err(OpinionStateError {
            number: NonZeroU32::MIN
        })
    );
    ledger.entries.first_mut().unwrap().status = FindingStatus::Open;
    assert_eq!(ledger.request_opinion(NonZeroU32::MIN, request()), Ok(()));
    for name in ["inv-opus", "inv-gemini"] {
        ledger
            .assign_opinion(
                NonZeroU32::MIN,
                name.parse().unwrap(),
                "task-20261111-193007-k3x9".parse().unwrap(),
            )
            .unwrap();
    }
    assert!(
        matches!(&ledger.opinions()[&NonZeroU32::MIN].stage, OpinionStage::Assigned { investigator, .. } if investigator.as_str() == "inv-gemini")
    );
}
