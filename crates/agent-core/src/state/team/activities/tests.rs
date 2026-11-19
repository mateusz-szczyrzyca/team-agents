//! Assignments survive subsequent reviews and retain their actual outcomes.
use super::*;
use crate::state::fixtures::{at, drive, inv, member, nz, review, round_id, run, task, team};
use crate::state::{ItemPhase, PhaseKind, ReviewOutcome};
use crate::status::{ReviewVerdict, RunOutcome, TaskStatus};

#[test]
fn investigator_crew_is_recorded_when_the_team_starts() {
    let team = team(1);
    assert_eq!(team.activities().len(), 1);
    let entry = &team.activities()[0];
    assert_eq!(entry.investigator().as_str(), "inv-opus");
    assert_eq!(entry.task(), &task(1));
    assert_eq!(entry.request(), &ActivityRequest::TeamMember);
    assert_eq!(entry.asked_at(), at(0));
    assert_eq!(entry.outcome(), &ActivityOutcome::Working);
}

#[test]
fn reviews_are_retained_across_fix_passes_and_merge_with_request_times_and_rounds() {
    let mut team = team(1);
    let id = "1.2".parse().unwrap();
    team.open_round(round_id(1), Some(id), at(1)).unwrap();
    drive(&mut team, "1.2", PhaseKind::InReview);
    team.item_mut(id)
        .unwrap()
        .ask_reviews(vec![review("inv-codex", 21, ReviewOutcome::Pending)])
        .unwrap();
    team.record_activities(at(21));
    assert_eq!(team.activities().len(), 2);
    let first = team.activities().last().unwrap();
    assert_eq!(first.request(), &ActivityRequest::FirstReview);
    assert_eq!(first.asked_at(), at(20));
    assert_eq!(first.rounds(), &[round_id(1)]);
    assert_eq!(first.outcome(), &ActivityOutcome::Working);
    team.item_mut(id)
        .unwrap()
        .submit_review(&task(21), ReviewVerdict::RequestChanges, at(30))
        .unwrap();
    team.record_activities(at(31));
    let first = team.activities().last().unwrap();
    assert_eq!(
        first.outcome(),
        &ActivityOutcome::Finished {
            at: at(30),
            end: ActivityEnd::Reported
        }
    );
    let mut item = team.item_mut(id).unwrap();
    let ItemPhase::InReview { pr, .. } = item.phase().clone() else {
        panic!("in review")
    };
    item.advance(ItemPhase::Fixing {
        assignee: "codex-1".parse().unwrap(),
        branch: pr.branch.clone(),
        pr: pr.clone(),
    })
    .unwrap();
    item.advance(ItemPhase::InReview {
        pr: pr.clone(),
        reviews: vec![],
    })
    .unwrap();
    let mut verification = review("inv-codex", 21, ReviewOutcome::Pending);
    verification.requested_at = at(40);
    item.ask_reviews(vec![verification]).unwrap();
    team.record_activities(at(41));
    assert_eq!(
        team.activities().last().unwrap().request(),
        &ActivityRequest::Verification { pass: nz(1) }
    );
    team.item_mut(id)
        .unwrap()
        .submit_review(&task(21), ReviewVerdict::Approve, at(50))
        .unwrap();
    team.record_activities(at(51));
    let mut item = team.item_mut(id).unwrap();
    let ItemPhase::InReview { reviews, .. } = item.phase().clone() else {
        panic!("in review")
    };
    item.advance(ItemPhase::Approved {
        pr: pr.clone(),
        reviews,
    })
    .unwrap();
    item.advance(ItemPhase::Done {
        completion: crate::state::Completion::Merged { pr, at: at(60) },
    })
    .unwrap();
    team.record_activities(at(61));
    assert_eq!(team.activities().len(), 3);
    assert_eq!(
        team.activities()[2].outcome(),
        &ActivityOutcome::Finished {
            at: at(50),
            end: ActivityEnd::Reported
        }
    );
    assert_eq!(
        serde_json::from_str::<Team>(&serde_json::to_string(&team).unwrap())
            .unwrap()
            .activities(),
        team.activities()
    );
}

#[test]
fn stopped_and_failed_assignments_keep_the_time_and_reason_without_reclosing_reports() {
    for (status, end, expected) in [
        (TaskStatus::Stopped, RunEnd::Stopped, ActivityEnd::Stopped),
        (
            TaskStatus::Failed,
            RunEnd::NoStatus { exit_code: Some(17) },
            ActivityEnd::Failed {
                reason: Some(RunEnd::NoStatus { exit_code: Some(17) }),
            },
        ),
        (
            TaskStatus::Failed,
            RunEnd::Reported {
                outcome: RunOutcome::Failed,
            },
            ActivityEnd::Failed {
                reason: Some(RunEnd::Reported {
                    outcome: RunOutcome::Failed,
                }),
            },
        ),
    ] {
        let mut team = team(1);
        team.add_reviewer(member(21, inv("inv-codex"))).unwrap();
        drive(&mut team, "1.2", PhaseKind::InReview);
        team.item_mut("1.2".parse().unwrap())
            .unwrap()
            .ask_reviews(vec![review("inv-codex", 21, ReviewOutcome::Pending)])
            .unwrap();
        team.record_activities(at(21));
        assert_eq!(team.activities().len(), 2);
        let (_, mut reviewer) = team.find_mut(&task(21)).unwrap();
        reviewer.transition(TaskStatus::Running).unwrap();
        reviewer.begin_run(run(1), at(22), None).unwrap();
        reviewer.end_run(at(30), end).unwrap();
        reviewer.transition(status).unwrap();
        team.record_activities(at(31));
        team.record_activities(at(99));
        assert_eq!(
            team.activities().last().unwrap().outcome(),
            &ActivityOutcome::Finished {
                at: at(30),
                end: expected
            }
        );
    }
}

#[test]
fn all_request_kinds_round_trip_including_future_workflows() {
    let requests = [
        ActivityRequest::TeamMember,
        ActivityRequest::FirstReview,
        ActivityRequest::Verification { pass: nz(2) },
        ActivityRequest::SecondOpinion {
            pass: nz(2),
            finding: nz(3),
        },
        ActivityRequest::Investigation,
        ActivityRequest::TaskAllocation,
    ];
    let kinds = [
        ActivityKind::TeamMember,
        ActivityKind::Reviewer,
        ActivityKind::Reviewer,
        ActivityKind::Reviewer,
        ActivityKind::Investigator,
        ActivityKind::TaskAllocator,
    ];
    for (request, kind) in requests.into_iter().zip(kinds) {
        assert_eq!(request.kind(), kind);
        assert_eq!(
            serde_json::from_str::<ActivityRequest>(&serde_json::to_string(&request).unwrap()).unwrap(),
            request
        );
    }
}

#[test]
fn second_opinions_retain_replacements_reports_and_each_failure_reason() {
    use crate::review_response::opinion::{OpinionReport, Outcome};
    use crate::state::{OpinionRequest, State};
    for failure in [
        OpinionFailure::Failed,
        OpinionFailure::NoStatus,
        OpinionFailure::MissingReport,
        OpinionFailure::Silent,
        OpinionFailure::Interrupted,
    ] {
        let mut state = State::from_json(include_str!(
            "../../../../tests/fixtures/state-before-opinion-retries.json"
        ))
        .unwrap();
        let id = state.teams()[0].id().clone();
        let mut team = state.team_mut(&id).unwrap();
        let item_id = "1.2".parse().unwrap();
        {
            let mut item = team.item_mut(item_id).unwrap();
            let ledger = item.findings_mut().unwrap();
            ledger
                .request_opinion(
                    nz(1),
                    OpinionRequest {
                        pass: nz(1),
                        head: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".parse().unwrap(),
                        reason: "café boundary".to_owned().try_into().unwrap(),
                    },
                )
                .unwrap();
            ledger
                .assign_opinion_at(nz(1), "inv-first".parse().unwrap(), task(31), at(40))
                .unwrap();
        }
        team.record_activities(at(41));
        assert_eq!(team.activities().iter().filter(|a| a.task() == &task(31)).count(), 1);
        let first = team.activities().iter().find(|a| a.task() == &task(31)).unwrap();
        assert_eq!(
            first.request(),
            &ActivityRequest::SecondOpinion {
                pass: nz(1),
                finding: nz(1)
            }
        );
        assert_eq!(first.asked_at(), at(40));
        assert_eq!(first.outcome(), &ActivityOutcome::Working);
        team.item_mut(item_id)
            .unwrap()
            .findings_mut()
            .unwrap()
            .fail_opinion(nz(1), &task(31), failure)
            .unwrap();
        team.record_activities(at(50));
        team.item_mut(item_id)
            .unwrap()
            .findings_mut()
            .unwrap()
            .assign_opinion_at(nz(1), "inv-next".parse().unwrap(), task(32), at(60))
            .unwrap();
        team.record_activities(at(61));
        team.item_mut(item_id)
            .unwrap()
            .findings_mut()
            .unwrap()
            .record_opinion(
                nz(1),
                &task(32),
                OpinionReport {
                    outcome: Outcome::Confirmed,
                    evidence: "checked café branch".to_owned().try_into().unwrap(),
                },
            )
            .unwrap();
        team.record_activities(at(70));
        team.record_activities(at(99));
        let entries: Vec<_> = team
            .activities()
            .iter()
            .filter(|a| matches!(a.request(), ActivityRequest::SecondOpinion { .. }))
            .collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].outcome(),
            &ActivityOutcome::Finished {
                at: at(50),
                end: ActivityEnd::OpinionFailed { reason: failure }
            }
        );
        assert_eq!(
            entries[1].outcome(),
            &ActivityOutcome::Finished {
                at: at(70),
                end: ActivityEnd::Reported
            }
        );
    }
}
