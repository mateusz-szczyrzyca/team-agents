//! Persisted activity history follows the actual daemon and fake CLI lifecycle.
use super::rig::{ONE, Rig, investigator, normal};
use agent_core::protocol::{Client, Command, ControlRequest, Reply, Target};
use agent_core::state::{ActivityEnd, ActivityOutcome, ActivityRequest, ItemPhase, MergeMode, RunState};
use agent_core::status::TaskStatus;

#[tokio::test]
async fn activity_history_retains_initial_reviews_and_two_verifications_after_merge() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-work-pushed,fix-work-pushed"),
            investigator("inv-r1", "rotating-verify"),
            investigator("inv-r2", "rotating-verify"),
            investigator("inv-r3", "rotating-verify"),
            investigator("inv-r4", "rotating-verify"),
            investigator("inv-r5", "rotating-verify"),
            investigator("inv-r6", "rotating-verify"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (id, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "merged", |p| matches!(p, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();
    let state = rig.state();
    let team = state.team(&id).unwrap();
    let entries = team.activities();
    assert_eq!(entries.len(), 6);
    let requests: Vec<_> = entries.iter().map(|a| a.request().clone()).collect();
    assert_eq!(
        requests,
        [
            ActivityRequest::FirstReview,
            ActivityRequest::FirstReview,
            ActivityRequest::Verification {
                pass: std::num::NonZeroU32::MIN
            },
            ActivityRequest::Verification {
                pass: std::num::NonZeroU32::MIN
            },
            ActivityRequest::Verification {
                pass: std::num::NonZeroU32::new(2).unwrap()
            },
            ActivityRequest::Verification {
                pass: std::num::NonZeroU32::new(2).unwrap()
            }
        ]
    );
    for entry in entries {
        assert_eq!(entry.rounds(), &[team.rounds()[0].id().clone()]);
        assert!(
            matches!(entry.outcome(), ActivityOutcome::Finished { at, end: ActivityEnd::Reported } if *at >= entry.asked_at())
        );
    }
    rig.start().await;
    assert_eq!(rig.state().team(&id).unwrap().activities(), entries);
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn activity_history_records_a_queued_review_and_its_human_stop() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "sleep"),
            investigator("inv-r2", "sleep"),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (id, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where(
            "1.1",
            "reviews assigned",
            |p| matches!(p, ItemPhase::InReview { reviews, .. } if reviews.len() == 2),
        )
        .await;
    let team = state.team(&id).unwrap();
    assert_eq!(team.activities().len(), 2);
    for activity in team.activities() {
        assert_eq!(activity.outcome(), &ActivityOutcome::Working);
    }
    let task = team.activities()[0].task().clone();
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Task(task.clone()),
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "{reply:?}");
    let state = rig.status_is(&task, TaskStatus::Stopped).await;
    let team = state.team(&id).unwrap();
    let member = team.find(&task).unwrap().1;
    let RunState::Ended { at, .. } = member.runs().last().unwrap().state() else {
        panic!("stopped run")
    };
    // The reviewer's last run ended by itself (it sleeps), before the human stopped the task: the stop ends the
    // assignment when it happened, not when the earlier run ended.
    let ActivityOutcome::Finished {
        at: stopped_at,
        end: ActivityEnd::Stopped,
    } = team.activities().iter().find(|a| a.task() == &task).unwrap().outcome()
    else {
        panic!("the assignment ended with the stop")
    };
    assert!(stopped_at >= at, "{stopped_at:?} is not after the run end {at:?}");
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn activity_history_keeps_the_failed_reviewer_after_replacement() {
    let fail = "fail-first:VERDICT: APPROVE\\nSUMMARY: fine";
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", fail),
            investigator("inv-r2", fail),
            investigator("inv-r3", fail),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (id, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "merged", |p| matches!(p, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();
    let state = rig.state();
    let team = state.team(&id).unwrap();
    assert_eq!(team.activities().len(), 3);
    let failed = team
        .reviewers()
        .iter()
        .find(|m| m.status() == TaskStatus::Failed)
        .unwrap();
    let RunState::Ended { at, end } = failed.runs().last().unwrap().state() else {
        panic!("failed run")
    };
    assert_eq!(
        team.activities()
            .iter()
            .find(|a| a.task() == failed.task())
            .unwrap()
            .outcome(),
        &ActivityOutcome::Finished {
            at: *at,
            end: ActivityEnd::Failed {
                reason: Some(end.clone())
            }
        }
    );
    assert_eq!(
        team.activities()
            .iter()
            .filter(|a| matches!(
                a.outcome(),
                ActivityOutcome::Finished {
                    end: ActivityEnd::Reported,
                    ..
                }
            ))
            .count(),
        2
    );
}

#[tokio::test]
async fn activity_history_records_the_reports_of_second_opinions() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,opinion-ask"),
            investigator("inv-r1", "opinion-probe:NOT CONFIRMED"),
            investigator("inv-r2", "opinion-probe:NOT CONFIRMED"),
            investigator("inv-r3", "opinion-probe:NOT CONFIRMED"),
            investigator("inv-r4", "opinion-probe:NOT CONFIRMED"),
        ],
        1,
        false,
    )
    .await;
    rig.start().await;
    let (id, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "merged", |p| matches!(p, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();
    let state = rig.state();
    let team = state.team(&id).unwrap();
    let opinions: Vec<_> = team
        .activities()
        .iter()
        .filter(|a| matches!(a.request(), ActivityRequest::SecondOpinion { .. }))
        .collect();
    assert_eq!(opinions.len(), 2);
    for entry in opinions {
        assert!(
            matches!(entry.outcome(), ActivityOutcome::Finished { end: ActivityEnd::Reported, at } if *at >= entry.asked_at())
        );
    }
}
