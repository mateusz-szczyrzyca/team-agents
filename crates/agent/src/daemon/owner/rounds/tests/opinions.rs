//! Live second opinions while a pull request is being fixed.
use super::rig::{ONE, Rig, investigator, normal};
use agent_core::state::{FindingStatus, ItemPhase, OpinionStage};

#[tokio::test]
async fn opinions_settle_together_before_verification_or_unchanged_approval() {
    for (lead, probe, changed, confirmed) in [
        ("work,opinion-ask", "opinion-probe:NOT CONFIRMED", false, false),
        ("work,opinion-ask-changed", "opinion-probe:BAD", true, false),
        (
            "work,opinion-ask,fix-work-pushed",
            "opinion-probe:CONFIRMED",
            true,
            true,
        ),
    ] {
        let mut rig = Rig::new(
            &[
                normal("claude-1", lead),
                investigator("inv-r1", probe),
                investigator("inv-r2", probe),
                investigator("inv-r3", probe),
                investigator("inv-r4", probe),
            ],
            1,
            false,
        )
        .await;
        rig.start().await;
        let (team, tasks) = rig
            .run(&["claude-1"], ONE, agent_core::state::MergeMode::Autopilot, None)
            .await;
        let checked = rig
            .state_where("first fix response checked", |state| {
                state.team(&team).unwrap().items()[0]
                    .findings()
                    .and_then(|l| l.disputes())
                    .is_some_and(|d| d.checked_run.is_some())
            })
            .await;
        assert_eq!(
            checked.team(&team).unwrap().items()[0]
                .findings()
                .unwrap()
                .opinions()
                .len(),
            2
        );
        rig.item_where("1.1", "done after opinions", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
        rig.stop().await.unwrap();
        let state = rig.state();
        let item = &state.team(&team).unwrap().items()[0];
        assert_eq!(item.fix_passes().len(), 1);
        let ledger = item.findings().unwrap();
        assert_eq!(ledger.opinions().len(), 2);
        assert_eq!(rig.github.reviews(1).len(), if changed { 4 } else { 2 });
        for (number, opinion) in ledger.opinions() {
            let OpinionStage::Reported { investigator, task, .. } = &opinion.stage else {
                panic!("opinion incomplete");
            };
            assert_ne!(
                investigator,
                &ledger.entries()[usize::try_from(number.get()).unwrap() - 1].reporter
            );
            assert!(
                !item.fix_passes()[0]
                    .requested_by
                    .iter()
                    .any(|review| &review.reviewer == investigator)
            );
            let prompt = rig.kept_prompt(task, 1);
            assert!(prompt.contains("# SECOND OPINION REQUEST"));
            assert!(prompt.contains(opinion.request.head.as_str()));
            assert!(prompt.contains("café path is unreachable"));
            assert!(!prompt.contains("A PULL REQUEST TO REVIEW"));
            assert_eq!(
                ledger.entries()[usize::try_from(number.get()).unwrap() - 1].status,
                if confirmed {
                    FindingStatus::Resolved
                } else {
                    FindingStatus::NotConfirmed
                }
            );
            let comments = rig.github.calls("PATCH", "/repos/example/project/issues/comments/1");
            let all = format!("{:?}", rig.github.requests());
            assert!(all.contains(investigator.as_str()), "{comments:?}");
            assert!(all.contains("café path examined") || all.contains("café path is unreachable"));
        }
        assert_eq!(rig.kept_runs(&tasks[0]), if confirmed { 3 } else { 2 });
    }
}
