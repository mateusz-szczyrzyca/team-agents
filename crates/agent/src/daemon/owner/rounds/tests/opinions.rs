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

#[tokio::test]
async fn unanswered_opinions_notify_once_and_keep_waiting_across_restart() {
    for probe in [
        "opinion-failure:FAILED",
        "opinion-failure:EMPTY",
        "opinion-failure:INVALID",
        "opinion-failure:HOLD",
    ] {
        let mut rig = Rig::new(
            &[
                normal("claude-1", "work,opinion-ask"),
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
        let (team, _) = rig
            .run(&["claude-1"], ONE, agent_core::state::MergeMode::Autopilot, None)
            .await;
        rig.state_where("both opinions assigned", |state| {
            state.team(&team).unwrap().items()[0].findings().is_some_and(|ledger| {
                ledger.opinions().len() == 2
                    && ledger
                        .opinions()
                        .values()
                        .all(|o| matches!(o.stage, OpinionStage::Assigned { .. }))
            })
        })
        .await;
        if probe.ends_with("HOLD") {
            rig.state_where("both opinion runs started", |state| {
                let team = state.team(&team).unwrap();
                team.items()[0].findings().unwrap().opinions().values().all(|opinion| {
                    let OpinionStage::Assigned { task, .. } = &opinion.stage else {
                        return false;
                    };
                    team.find(task).unwrap().1.current_run().is_some()
                })
            })
            .await;
        } else {
            rig.wait_for("one notice per unanswered opinion", || {
                rig.notifications()
                    .iter()
                    .filter(|n| n.contains("Second opinion needs attention"))
                    .count()
                    == 2
            })
            .await;
        }
        rig.stop().await.unwrap();
        rig.start().await;
        rig.wait_for("restart notices", || {
            rig.notifications()
                .iter()
                .filter(|n| n.contains("Second opinion needs attention"))
                .count()
                == 2
        })
        .await;
        let state = rig.state();
        let team = state.team(&team).unwrap();
        let item = &team.items()[0];
        let round = &team.rounds()[0];
        let detail = agent_core::protocol::RoundDetail::new(team, round, Vec::new());
        assert_eq!(
            agent_core::round_work::round_work(&detail, agent_core::round_work::DraftPull::Absent)
                .unwrap()
                .to_string(),
            "waiting for a second opinion"
        );
        assert!(matches!(item.phase(), ItemPhase::Fixing { .. }));
        for (number, opinion) in item.findings().unwrap().opinions() {
            let OpinionStage::Assigned { task, .. } = &opinion.stage else {
                panic!("must remain pending")
            };
            assert_eq!(
                rig.notifications()
                    .iter()
                    .filter(|n| n.contains("Second opinion needs attention")
                        && n.contains(round.id().as_str())
                        && n.contains(task.as_str())
                        && n.contains(&format!("FINDING {number}")))
                    .count(),
                1
            );
        }
        rig.stop().await.unwrap();
        assert_eq!(
            rig.notifications()
                .iter()
                .filter(|n| n.contains("Second opinion needs attention"))
                .count(),
            2
        );
        assert_eq!(rig.github.reviews(1).len(), 2);
    }
}

#[tokio::test]
async fn refused_opinions_offer_only_the_answers_allowed_for_the_finding() {
    for (disabled, severity) in [(true, "BLOCKER"), (false, "MAJOR")] {
        let report = if severity == "BLOCKER" {
            "done:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [BLOCKER] COR src/lib.rs:7 — café defect → fix it\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: verified"
        } else {
            "done:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] COR src/lib.rs:7 — café defect → fix it\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: verified"
        };
        let mut rig = Rig::new(
            &[
                normal("claude-1", "work,opinion-ask,fix-work:1"),
                investigator("inv-r1", report),
                investigator(
                    "inv-r2",
                    "done:VERDICT: APPROVE,verify:VERDICT: APPROVE\\nSUMMARY: verified",
                ),
            ],
            1,
            false,
        )
        .await;
        if disabled {
            let config = std::fs::read_to_string(rig.home.config_file())
                .unwrap()
                .replace("[review]", "[review]\nallow_second_opinion_on_blocker = false");
            std::fs::write(rig.home.config_file(), config).unwrap();
        }
        rig.start().await;
        let (team, tasks) = rig
            .run(&["claude-1"], ONE, agent_core::state::MergeMode::Autopilot, None)
            .await;
        rig.item_where("1.1", "fixed after refused request", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
        rig.stop().await.unwrap();
        assert_eq!(rig.kept_runs(&tasks[0]), 3);
        let correction = rig.kept_prompt(&tasks[0], 3);
        assert!(correction.contains("Line 1"), "{correction}");
        assert!(correction.contains("FINDING 1"), "{correction}");
        if disabled {
            assert!(!rig.kept_prompt(&tasks[0], 2).contains("SECOND OPINION"));
        } else {
            assert!(correction.contains("EXPLAINED"), "{correction}");
        }
        let state = rig.state();
        let item = &state.team(&team).unwrap().items()[0];
        assert_eq!(item.fix_passes().len(), 1);
        assert_eq!(item.findings().unwrap().opinions().len(), 0);
    }
}

#[tokio::test]
async fn a_changed_tip_waits_for_all_opinions_and_opinion_runs_have_no_bus() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,opinion-ask-changed"),
            investigator("inv-r1", "opinion-failure:HOLD"),
            investigator("inv-r2", "opinion-failure:HOLD"),
            investigator("inv-r3", "opinion-failure:HOLD"),
            investigator("inv-r4", "opinion-failure:HOLD"),
        ],
        1,
        true,
    )
    .await;
    rig.start().await;
    let (team_id, _) = rig
        .run(&["claude-1"], ONE, agent_core::state::MergeMode::Autopilot, None)
        .await;
    let state = rig
        .state_where("all opinion runs started", |state| {
            let team = state.team(&team_id).unwrap();
            team.items()[0].findings().is_some_and(|ledger| {
                ledger.opinions().len() == 2
                    && ledger.opinions().values().all(|opinion| {
                        let OpinionStage::Assigned { task, .. } = &opinion.stage else {
                            return false;
                        };
                        team.find(task).unwrap().1.current_run().is_some()
                    })
            })
        })
        .await;
    let team = state.team(&team_id).unwrap();
    let ledger = team.items()[0].findings().unwrap();
    for opinion in ledger.opinions().values() {
        let OpinionStage::Assigned { task, .. } = &opinion.stage else {
            panic!("unassigned");
        };
        let run = team.find(task).unwrap().1.current_run().unwrap();
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(rig.home.run_dir(task, run.id()).join("mcp.json")).unwrap()).unwrap();
        assert_eq!(config["mcpServers"].get("agentbus"), None);
    }
    assert_eq!(rig.github.reviews(1).len(), 2);
    assert!(matches!(team.items()[0].phase(), ItemPhase::Fixing { .. }));
    assert_ne!(
        ledger.opinions().values().next().unwrap().request.head,
        ledger.verification().unwrap().reviewed_head.clone().unwrap()
    );
    rig.stop().await.unwrap();
    assert_eq!(rig.github.reviews(1).len(), 2);
}
