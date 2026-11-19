//! Assignments survive subsequent reviews and retain their actual outcomes.
use super::*;
use crate::ids::ItemId;
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

#[test]
fn activity_input_refuses_an_end_before_assignment_and_repeated_rounds() {
    let base = serde_json::json!({
        "investigator": "inv-opus", "task": task(1), "rounds": [round_id(1)],
        "request": { "kind": "first_review" }, "asked_at": at(20),
        "outcome": { "status": "finished", "at": at(19), "end": { "kind": "reported" } }
    });
    let error = serde_json::from_value::<super::Activity>(base.clone()).unwrap_err();
    assert_eq!(error.classify(), serde_json::error::Category::Data);
    let mut duplicate = base;
    duplicate["outcome"] = serde_json::json!({"status": "working"});
    duplicate["rounds"] = serde_json::json!([round_id(1), round_id(1)]);
    let error = serde_json::from_value::<super::Activity>(duplicate).unwrap_err();
    assert_eq!(error.classify(), serde_json::error::Category::Data);
}

#[test]
fn shared_verification_uses_the_pull_request_pass_for_every_item() {
    let mut team = team(1);
    let first: ItemId = "1.2".parse().unwrap();
    let joined: ItemId = "2.1".parse().unwrap();
    drive(&mut team, "1.2", PhaseKind::InReview);
    team.item_mut(first)
        .unwrap()
        .ask_reviews(vec![review("inv-codex", 21, ReviewOutcome::Pending)])
        .unwrap();
    team.item_mut(first)
        .unwrap()
        .submit_review(&task(21), ReviewVerdict::RequestChanges, at(30))
        .unwrap();
    let mut item = team.item_mut(first).unwrap();
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
    item.ask_reviews(vec![verification.clone()]).unwrap();
    // The second item joins the pull request after its first fix pass: it has no fix pass of its own.
    let mut item = team.item_mut(joined).unwrap();
    item.advance(ItemPhase::InProgress {
        assignee: "codex-1".parse().unwrap(),
        branch: pr.branch.clone(),
    })
    .unwrap();
    item.advance(ItemPhase::InReview { pr, reviews: vec![] }).unwrap();
    item.ask_reviews(vec![verification]).unwrap();
    team.record_activities(at(41));
    let of_reviewer: Vec<_> = team.activities().iter().filter(|a| a.task() == &task(21)).collect();
    assert_eq!(of_reviewer.len(), 2);
    assert_eq!(of_reviewer[0].request(), &ActivityRequest::FirstReview);
    assert_eq!(of_reviewer[1].request(), &ActivityRequest::Verification { pass: nz(1) });
    assert_eq!(of_reviewer[1].rounds().len(), 0);
    team.record_activities(at(42));
    assert_eq!(team.activities().iter().filter(|a| a.task() == &task(21)).count(), 2);
}

#[test]
fn a_task_stopped_while_it_waits_ends_when_it_was_stopped_not_when_its_last_run_ended() {
    let mut team = team(1);
    let (_, mut coordinator) = team.find_mut(&task(1)).unwrap();
    coordinator.transition(TaskStatus::Running).unwrap();
    coordinator.begin_run(run(1), at(10), None).unwrap();
    coordinator
        .end_run(
            at(20),
            RunEnd::Reported {
                outcome: RunOutcome::Sleeping,
            },
        )
        .unwrap();
    coordinator.transition(TaskStatus::Waiting).unwrap();
    coordinator.transition(TaskStatus::Stopped).unwrap();
    team.record_activities(at(30));
    assert_eq!(
        team.activities()[0].outcome(),
        &ActivityOutcome::Finished {
            at: at(30),
            end: ActivityEnd::Stopped
        }
    );
}

#[test]
fn a_reviewer_that_cannot_start_fails_with_the_reason_of_a_run_that_ended_without_status() {
    let mut team = team(1);
    team.add_reviewer(member(21, inv("inv-codex"))).unwrap();
    drive(&mut team, "1.2", PhaseKind::InReview);
    team.item_mut("1.2".parse().unwrap())
        .unwrap()
        .ask_reviews(vec![review("inv-codex", 21, ReviewOutcome::Pending)])
        .unwrap();
    let (_, mut reviewer) = team.find_mut(&task(21)).unwrap();
    reviewer.transition(TaskStatus::Running).unwrap();
    reviewer.transition(TaskStatus::Failed).unwrap();
    team.record_activities(at(25));
    assert_eq!(
        team.activities().last().unwrap().outcome(),
        &ActivityOutcome::Finished {
            at: at(25),
            end: ActivityEnd::Failed {
                reason: Some(RunEnd::NoStatus { exit_code: None })
            }
        }
    );
}

#[test]
fn a_round_that_opens_after_an_assignment_is_added_to_it_without_a_second_record() {
    let mut team = team(1);
    assert_eq!(team.activities()[0].rounds(), &[]);
    team.open_round(round_id(1), Some("1.2".parse().unwrap()), at(1))
        .unwrap();
    team.record_activities(at(2));
    assert_eq!(team.activities().len(), 1);
    assert_eq!(team.activities()[0].rounds(), &[round_id(1)]);
    team.record_activities(at(3));
    assert_eq!(team.activities()[0].rounds(), &[round_id(1)]);
}

#[test]
fn a_report_that_was_accepted_is_not_replaced_when_its_reviewer_is_stopped_or_fails_later() {
    for status in [TaskStatus::Stopped, TaskStatus::Failed] {
        let mut team = team(1);
        team.add_reviewer(member(21, inv("inv-codex"))).unwrap();
        drive(&mut team, "1.2", PhaseKind::InReview);
        let id = "1.2".parse().unwrap();
        team.item_mut(id)
            .unwrap()
            .ask_reviews(vec![review("inv-codex", 21, ReviewOutcome::Pending)])
            .unwrap();
        team.item_mut(id)
            .unwrap()
            .submit_review(&task(21), ReviewVerdict::Approve, at(30))
            .unwrap();
        team.record_activities(at(31));
        let (_, mut reviewer) = team.find_mut(&task(21)).unwrap();
        reviewer.transition(TaskStatus::Running).unwrap();
        reviewer.begin_run(run(1), at(32), None).unwrap();
        reviewer.end_run(at(35), RunEnd::Stopped).unwrap();
        reviewer.transition(status).unwrap();
        team.record_activities(at(99));
        assert_eq!(
            team.activities().last().unwrap().outcome(),
            &ActivityOutcome::Finished {
                at: at(30),
                end: ActivityEnd::Reported
            }
        );
    }
}

#[test]
fn an_investigator_of_the_crew_that_is_done_has_reported() {
    let mut team = team(1);
    let (_, mut coordinator) = team.find_mut(&task(1)).unwrap();
    coordinator.transition(TaskStatus::Running).unwrap();
    coordinator.begin_run(run(1), at(10), None).unwrap();
    coordinator
        .end_run(
            at(20),
            RunEnd::Reported {
                outcome: RunOutcome::Done,
            },
        )
        .unwrap();
    coordinator.transition(TaskStatus::Done).unwrap();
    team.record_activities(at(30));
    assert_eq!(
        team.activities()[0].outcome(),
        &ActivityOutcome::Finished {
            at: at(20),
            end: ActivityEnd::Reported
        }
    );
}

/// The review of task 21 shared by `fixing`, which has had a fix pass, and `joined`, which has had none; recorded once.
fn team_with_a_shared_verification(fixing: &str, joined: &str) -> Team {
    let mut team = team(1);
    let fixing_id: ItemId = fixing.parse().unwrap();
    drive(&mut team, fixing, PhaseKind::InReview);
    let mut item = team.item_mut(fixing_id).unwrap();
    item.ask_reviews(vec![review("inv-codex", 21, ReviewOutcome::Pending)])
        .unwrap();
    item.submit_review(&task(21), ReviewVerdict::RequestChanges, at(30))
        .unwrap();
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
    item.ask_reviews(vec![verification.clone()]).unwrap();
    let mut item = team.item_mut(joined.parse().unwrap()).unwrap();
    item.advance(ItemPhase::InProgress {
        assignee: "codex-1".parse().unwrap(),
        branch: pr.branch.clone(),
    })
    .unwrap();
    item.advance(ItemPhase::InReview { pr, reviews: vec![] }).unwrap();
    item.ask_reviews(vec![verification]).unwrap();
    team.record_activities(at(41));
    team
}

#[test]
fn a_shared_verification_is_one_activity_whichever_item_of_the_list_has_had_the_fix_pass() {
    let team = team_with_a_shared_verification("2.1", "1.2");
    let mut of_reviewer: Vec<_> = team.activities().iter().filter(|a| a.task() == &task(21)).collect();
    of_reviewer.sort_by_key(|activity| activity.asked_at());
    assert_eq!(of_reviewer.len(), 2);
    assert_eq!(of_reviewer[0].request(), &ActivityRequest::FirstReview);
    assert_eq!(of_reviewer[1].request(), &ActivityRequest::Verification { pass: nz(1) });
}

#[test]
fn a_task_that_cannot_resume_fails_when_it_was_seen_and_not_with_its_earlier_sleep() {
    for (slept_at, failed_at) in [(35, 50), (22, 30)] {
        let mut team = team(1);
        team.add_reviewer(member(21, inv("inv-codex"))).unwrap();
        drive(&mut team, "1.2", PhaseKind::InReview);
        team.item_mut("1.2".parse().unwrap())
            .unwrap()
            .ask_reviews(vec![review("inv-codex", 21, ReviewOutcome::Pending)])
            .unwrap();
        let (_, mut reviewer) = team.find_mut(&task(21)).unwrap();
        reviewer.transition(TaskStatus::Running).unwrap();
        reviewer.begin_run(run(1), at(slept_at - 5), None).unwrap();
        reviewer
            .end_run(
                at(slept_at),
                RunEnd::Reported {
                    outcome: RunOutcome::Sleeping,
                },
            )
            .unwrap();
        reviewer.transition(TaskStatus::Waiting).unwrap();
        team.record_activities(at(slept_at + 1));
        let (_, mut reviewer) = team.find_mut(&task(21)).unwrap();
        reviewer.transition(TaskStatus::Queued).unwrap();
        reviewer.transition(TaskStatus::Running).unwrap();
        reviewer.transition(TaskStatus::Failed).unwrap();
        team.record_activities(at(failed_at));
        assert_eq!(
            team.activities().last().unwrap().outcome(),
            &ActivityOutcome::Finished {
                at: at(failed_at),
                end: ActivityEnd::Failed {
                    reason: Some(RunEnd::NoStatus { exit_code: None })
                }
            }
        );
    }
}

#[test]
fn a_joined_item_shares_the_verification_and_not_the_equal_time_first_review() {
    let mut team = team(1);
    let first: ItemId = "1.2".parse().unwrap();
    let joined: ItemId = "2.1".parse().unwrap();
    team.open_round(round_id(1), Some(first), at(1)).unwrap();
    team.open_round(round_id(2), Some(joined), at(2)).unwrap();
    drive(&mut team, "1.2", PhaseKind::InReview);
    let mut item = team.item_mut(first).unwrap();
    item.ask_reviews(vec![review("inv-codex", 21, ReviewOutcome::Pending)])
        .unwrap();
    item.submit_review(&task(21), ReviewVerdict::RequestChanges, at(30))
        .unwrap();
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
    // The verification is asked at the same second as the first review.
    item.ask_reviews(vec![review("inv-codex", 21, ReviewOutcome::Pending)])
        .unwrap();
    let mut item = team.item_mut(joined).unwrap();
    item.advance(ItemPhase::InProgress {
        assignee: "codex-1".parse().unwrap(),
        branch: pr.branch.clone(),
    })
    .unwrap();
    item.advance(ItemPhase::InReview { pr, reviews: vec![] }).unwrap();
    item.ask_reviews(vec![review("inv-codex", 21, ReviewOutcome::Pending)])
        .unwrap();
    team.record_activities(at(41));
    let of_reviewer: Vec<_> = team.activities().iter().filter(|a| a.task() == &task(21)).collect();
    assert_eq!(of_reviewer.len(), 2);
    assert_eq!(of_reviewer[0].request(), &ActivityRequest::FirstReview);
    assert_eq!(of_reviewer[0].rounds(), &[round_id(1)]);
    assert_eq!(of_reviewer[1].request(), &ActivityRequest::Verification { pass: nz(1) });
    assert_eq!(of_reviewer[1].rounds(), &[round_id(1), round_id(2)]);
}

#[test]
fn a_task_that_waited_for_the_human_and_cannot_resume_fails_when_it_was_seen() {
    let mut team = team(1);
    team.add_reviewer(member(21, inv("inv-codex"))).unwrap();
    drive(&mut team, "1.2", PhaseKind::InReview);
    team.item_mut("1.2".parse().unwrap())
        .unwrap()
        .ask_reviews(vec![review("inv-codex", 21, ReviewOutcome::Pending)])
        .unwrap();
    let (_, mut reviewer) = team.find_mut(&task(21)).unwrap();
    reviewer.transition(TaskStatus::Running).unwrap();
    reviewer.begin_run(run(1), at(30), None).unwrap();
    reviewer
        .end_run(
            at(35),
            RunEnd::Reported {
                outcome: RunOutcome::Questions,
            },
        )
        .unwrap();
    reviewer.transition(TaskStatus::Questions).unwrap();
    team.record_activities(at(36));
    let (_, mut reviewer) = team.find_mut(&task(21)).unwrap();
    reviewer.transition(TaskStatus::Queued).unwrap();
    reviewer.transition(TaskStatus::Running).unwrap();
    reviewer.transition(TaskStatus::Failed).unwrap();
    team.record_activities(at(50));
    assert_eq!(
        team.activities().last().unwrap().outcome(),
        &ActivityOutcome::Finished {
            at: at(50),
            end: ActivityEnd::Failed {
                reason: Some(RunEnd::NoStatus { exit_code: None })
            }
        }
    );
}

#[test]
fn a_start_failure_after_a_run_without_a_status_is_told_to_the_record_and_not_inferred() {
    let mut team = team(1);
    team.add_reviewer(member(21, inv("inv-codex"))).unwrap();
    drive(&mut team, "1.2", PhaseKind::InReview);
    team.item_mut("1.2".parse().unwrap())
        .unwrap()
        .ask_reviews(vec![review("inv-codex", 21, ReviewOutcome::Pending)])
        .unwrap();
    let (_, mut reviewer) = team.find_mut(&task(21)).unwrap();
    reviewer.transition(TaskStatus::Running).unwrap();
    reviewer.begin_run(run(1), at(30), None).unwrap();
    reviewer
        .end_run(at(35), RunEnd::NoStatus { exit_code: Some(1) })
        .unwrap();
    reviewer.transition(TaskStatus::Queued).unwrap();
    team.record_activities(at(36));
    let (_, mut reviewer) = team.find_mut(&task(21)).unwrap();
    reviewer.transition(TaskStatus::Running).unwrap();
    reviewer.transition(TaskStatus::Failed).unwrap();
    team.fail_activities_before_a_run(&task(21), at(50));
    team.record_activities(at(51));
    assert_eq!(
        team.activities().last().unwrap().outcome(),
        &ActivityOutcome::Finished {
            at: at(50),
            end: ActivityEnd::Failed {
                reason: Some(RunEnd::NoStatus { exit_code: None })
            }
        }
    );
}
