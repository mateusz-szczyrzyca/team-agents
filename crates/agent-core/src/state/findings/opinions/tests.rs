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

#[test]
fn refuses_persisted_opinions_with_missing_findings_or_inconsistent_outcomes() {
    let n = NonZeroU32::MIN;
    let mut pending = ledger(Severity::Blocker);
    pending.request_opinion(n, request()).unwrap();
    let mut unknown = serde_json::to_value(&pending).unwrap();
    unknown["entries"] = serde_json::json!([]);
    let mut non_blocker = serde_json::to_value(&pending).unwrap();
    non_blocker["entries"][0]["finding"]["severity"] = serde_json::to_value(Severity::Major).unwrap();
    let mut closed_pending = serde_json::to_value(&pending).unwrap();
    closed_pending["entries"][0]["status"] = serde_json::json!("resolved");
    let task: TaskId = "task-20261111-193007-k3x9".parse().unwrap();
    pending
        .assign_opinion(n, "inv-opus".parse().unwrap(), task.clone())
        .unwrap();
    pending
        .record_opinion(
            n,
            &task,
            OpinionReport {
                outcome: Outcome::NotConfirmed,
                evidence: "no defect at this head".to_owned().try_into().unwrap(),
            },
        )
        .unwrap();
    let mut open_unconfirmed = serde_json::to_value(&pending).unwrap();
    open_unconfirmed["entries"][0]["status"] = serde_json::json!("open");
    let mut unsupported_closure = serde_json::to_value(&pending).unwrap();
    unsupported_closure["opinions"] = serde_json::json!({});
    for value in [
        unknown,
        non_blocker,
        closed_pending,
        open_unconfirmed,
        unsupported_closure,
    ] {
        let error = serde_json::from_value::<Findings>(value).expect_err("inconsistent opinion state must be refused");
        assert_eq!(error.classify(), serde_json::error::Category::Data);
    }
}

#[test]
fn accepts_atomic_opinion_answers_and_keeps_fixed_answers_after_reload() {
    use crate::review_response::opinion::Permission;
    let mut ledger = ledger(Severity::Blocker);
    let mut second = ledger.entries()[0].clone();
    second.number = NonZeroU32::new(2).unwrap();
    ledger.entries.push(second);
    ledger.disputes = Some(super::super::Disputes::default());
    let pass = NonZeroU32::MIN;
    let head = request().head;
    let before = ledger.clone();
    let error = ledger
        .answer_opinions(
            "FINDING 1: FIXED\nFINDING 2: SECOND OPINION — café",
            pass,
            &head,
            Permission::Disabled,
        )
        .unwrap_err();
    assert!(
        matches!(error, crate::review_response::opinion::OpinionError::Forbidden { number, .. } if number.get() == 2)
    );
    assert_eq!(ledger, before);
    ledger
        .answer_opinions(
            "FINDING 1: FIXED\nFINDING 2: SECOND OPINION — café",
            pass,
            &head,
            Permission::Enabled,
        )
        .unwrap();
    assert_eq!(ledger.fixed_answers(pass), [NonZeroU32::MIN].into());
    assert_eq!(
        ledger.opinions()[&NonZeroU32::new(2).unwrap()].request.reason.as_str(),
        "café"
    );
    let mut ledger: Findings = serde_json::from_str(&serde_json::to_string(&ledger).unwrap()).unwrap();
    assert_eq!(ledger.fixed_answers(pass), [NonZeroU32::MIN].into());
    let task: TaskId = "task-20261111-193007-k3x9".parse().unwrap();
    ledger
        .assign_opinion(NonZeroU32::new(2).unwrap(), "inv-opus".parse().unwrap(), task.clone())
        .unwrap();
    ledger
        .record_opinion(
            NonZeroU32::new(2).unwrap(),
            &task,
            OpinionReport {
                outcome: Outcome::Confirmed,
                evidence: "read branch".to_owned().try_into().unwrap(),
            },
        )
        .unwrap();
    assert!(matches!(
        ledger.answer_opinions("FINDING 2: SECOND OPINION — again", pass, &head, Permission::Enabled),
        Err(crate::review_response::opinion::OpinionError::Forbidden { .. })
    ));
    ledger
        .answer_opinions("FINDING 2: FIXED", pass, &head, Permission::Enabled)
        .unwrap();
    assert_eq!(
        ledger.fixed_answers(pass),
        [NonZeroU32::MIN, NonZeroU32::new(2).unwrap()].into()
    );
    assert_eq!(
        ledger.fixed_answers(NonZeroU32::new(2).unwrap()),
        std::collections::BTreeSet::default()
    );
}

#[test]
fn minor_findings_need_no_answer_but_cannot_request_an_opinion() {
    use crate::review_response::opinion::{OpinionError, Permission, Refusal};
    let mut ledger = ledger(Severity::Blocker);
    let mut minor = ledger.entries()[0].clone();
    minor.number = NonZeroU32::new(2).unwrap();
    minor.finding.severity = Severity::Minor;
    ledger.entries.push(minor);
    let pass = NonZeroU32::MIN;
    let head = request().head;
    let before = ledger.clone();
    assert_eq!(
        ledger.answer_opinions(
            "FINDING 1: FIXED\nFINDING 2: SECOND OPINION — café",
            pass,
            &head,
            Permission::Enabled
        ),
        Err(OpinionError::Forbidden {
            number: NonZeroU32::new(2).unwrap(),
            line: 2,
            reason: Refusal::NotBlocker
        })
    );
    assert_eq!(ledger, before);
    assert_eq!(
        ledger.answer_opinions("FINDING 1: FIXED", pass, &head, Permission::Enabled),
        Ok(vec![(
            NonZeroU32::MIN,
            crate::review_response::opinion::LeadAnswer::Fixed
        )])
    );
}

#[test]
fn a_missing_opinion_reason_identifies_the_actual_response_line_atomically() {
    use crate::review_response::opinion::Permission;
    let mut ledger = ledger(Severity::Blocker);
    let mut second = ledger.entries()[0].clone();
    second.number = NonZeroU32::new(2).unwrap();
    ledger.entries.push(second);
    let before = ledger.clone();
    let error = ledger
        .answer_opinions(
            "Summary café\nFINDING 1: FIXED\nFINDING 2: SECOND OPINION — ",
            NonZeroU32::MIN,
            &request().head,
            Permission::Enabled,
        )
        .unwrap_err();
    assert_eq!(
        error,
        crate::review_response::opinion::OpinionError::MissingReason {
            number: NonZeroU32::new(2).unwrap(),
            line: 3,
        }
    );
    assert_eq!(ledger, before);
}

#[test]
fn failed_assignments_are_bounded_durable_and_reject_late_reports() {
    let mut ledger = ledger(Severity::Blocker);
    let number = NonZeroU32::MIN;
    ledger.request_opinion(number, request()).unwrap();
    let tasks: Vec<TaskId> = [
        "task-20261111-193007-k3x9",
        "task-20261111-193008-k3x9",
        "task-20261111-193009-k3x9",
    ]
    .into_iter()
    .map(|s| s.parse().unwrap())
    .collect();
    for (index, task) in tasks.iter().enumerate() {
        let name: InvestigatorName = format!("inv-r{index}").parse().unwrap();
        assert_eq!(
            ledger.assign_opinion_at(
                number,
                name.clone(),
                task.clone(),
                "2026-11-19T06:00:00Z".parse().unwrap()
            ),
            Ok(())
        );
        assert_eq!(ledger.opinion_attempts(number).len(), index + 1);
        assert_eq!(ledger.opinion_attempts(number).last().unwrap().investigator, name);
        if index > 0 {
            assert_eq!(
                ledger.fail_opinion(number, &tasks[index - 1], OpinionFailure::Failed),
                Err(OpinionStateError { number })
            );
            assert_eq!(
                ledger.record_opinion(
                    number,
                    &tasks[index - 1],
                    OpinionReport {
                        outcome: Outcome::Confirmed,
                        evidence: "late café report".to_owned().try_into().unwrap()
                    }
                ),
                Err(OpinionStateError { number })
            );
        }
        ledger.fail_opinion(number, task, OpinionFailure::Failed).unwrap();
        assert_eq!(
            ledger.fail_opinion(number, task, OpinionFailure::Failed),
            Err(OpinionStateError { number })
        );
        assert_eq!(
            serde_json::from_str::<Findings>(&serde_json::to_string(&ledger).unwrap()).unwrap(),
            ledger
        );
        assert_eq!(
            matches!(ledger.opinions()[&number].stage, OpinionStage::Waiting),
            index < 2
        );
    }
    assert_eq!(
        ledger.assign_opinion_at(
            number,
            "inv-fourth".parse().unwrap(),
            tasks[0].clone(),
            "2026-11-19T06:00:00Z".parse().unwrap()
        ),
        Err(OpinionStateError { number })
    );
    assert_eq!(ledger.mark_opinion_notice(number), Ok(true));
    assert_eq!(ledger.mark_opinion_notice(number), Ok(false));
}
