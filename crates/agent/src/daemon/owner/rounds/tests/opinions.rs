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
                investigator("inv-r5", probe),
                investigator("inv-r6", probe),
                investigator("inv-r7", probe),
                investigator("inv-r8", probe),
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
        if probe.ends_with("HOLD") {
            let config = std::fs::read_to_string(rig.home.config_file()).unwrap();
            std::fs::write(
                rig.home.config_file(),
                config.replace("opinion-failure:HOLD", "opinion-failure:FAILED"),
            )
            .unwrap();
        }
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
            let attempts = item.findings().unwrap().opinion_attempts(*number);
            assert_eq!(attempts.len(), 3);
            assert!(attempts.iter().all(|attempt| attempt.failure.is_some()));
            let notice = rig
                .notifications()
                .into_iter()
                .find(|n| n.contains("Second opinion needs attention") && n.contains(&format!("FINDING {number}")))
                .unwrap();
            for attempt in attempts {
                assert!(notice.contains(attempt.investigator.as_str()), "{notice}");
            }
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
        let path = rig.home.run_dir(task, run.id()).join("mcp.json");
        rig.wait_for("opinion MCP configuration written", || path.is_file())
            .await;
        let config: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
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

#[tokio::test]
async fn confirmed_continuation_lists_only_the_blocker_still_needing_a_fix() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,opinion-mixed,fix-work:2"),
            investigator("inv-r1", "opinion-probe:CONFIRMED"),
            investigator("inv-r2", "opinion-probe:CONFIRMED"),
            investigator("inv-r3", "opinion-probe:CONFIRMED"),
            investigator("inv-r4", "opinion-probe:CONFIRMED"),
        ],
        1,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(&["claude-1"], ONE, agent_core::state::MergeMode::Autopilot, None)
        .await;
    rig.item_where("1.1", "mixed answers verified", |p| matches!(p, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();
    let continuation = rig.kept_prompt(&tasks[0], 3);
    assert!(
        !continuation.lines().any(|line| line.starts_with("1. [")),
        "{continuation}"
    );
    assert!(
        continuation.lines().any(|line| line.starts_with("2. [BLOCKER]")),
        "{continuation}"
    );
    assert_eq!(rig.kept_runs(&tasks[0]), 3);
    let state = rig.state();
    let item = &state.team(&team).unwrap().items()[0];
    assert_eq!(item.fix_passes().len(), 1);
    assert_eq!(
        item.findings().unwrap().fixed_answers(std::num::NonZeroU32::MIN),
        [std::num::NonZeroU32::MIN, std::num::NonZeroU32::new(2).unwrap()].into()
    );
}

#[tokio::test]
async fn opinion_history_records_the_request_outcome_and_run_reason() {
    use agent_core::history::{HistoryEvent, RunWhy};
    use agent_runtime::history::Reader;
    for (confirmed, lead, probe) in [
        (false, "work,opinion-ask", "opinion-probe:NOT CONFIRMED"),
        (true, "work,opinion-ask,fix-work-pushed", "opinion-probe:CONFIRMED"),
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
        let (team, _) = rig
            .run(&["claude-1"], ONE, agent_core::state::MergeMode::Autopilot, None)
            .await;
        rig.item_where("1.1", "opinions recorded and settled", |p| {
            matches!(p, ItemPhase::Done { .. })
        })
        .await;
        rig.stop().await.unwrap();
        let state = rig.state();
        let item = &state.team(&team).unwrap().items()[0];
        let reader = Reader::open(&rig.home.history_file()).unwrap();
        let events = opinion_events(&reader, &team);
        let runs = reader.runs_of_team(&team).unwrap();
        assert_eq!(events.len(), 4);
        for (number, opinion) in item.findings().unwrap().opinions() {
            let OpinionStage::Reported {
                investigator,
                task,
                report,
                ..
            } = &opinion.stage
            else {
                panic!("missing report")
            };
            let ask = HistoryEvent::SecondOpinionAsked {
                pr: agent_core::state::PrNumber::new(std::num::NonZeroU32::MIN),
                number: *number,
                pass: 1,
                by: "claude-1".parse().unwrap(),
                why: opinion.request.reason.as_str().to_owned(),
            };
            let by = investigator.as_str().parse().unwrap();
            let why = report.evidence.as_str().to_owned();
            let outcome = if confirmed {
                HistoryEvent::FindingConfirmed {
                    pr: agent_core::state::PrNumber::new(std::num::NonZeroU32::MIN),
                    number: *number,
                    pass: 1,
                    by,
                    why,
                }
            } else {
                HistoryEvent::FindingNotConfirmed {
                    pr: agent_core::state::PrNumber::new(std::num::NonZeroU32::MIN),
                    number: *number,
                    pass: 1,
                    by,
                    why,
                }
            };
            for expected in [ask, outcome] {
                let recorded: Vec<_> = events.iter().filter(|e| e.event == expected).collect();
                assert_eq!(recorded.len(), 1, "{expected:?}");
                assert_eq!(
                    recorded[0].rounds,
                    [state.team(&team).unwrap().rounds()[0].id().clone()]
                );
            }
            let recorded: Vec<_> = runs.iter().filter(|run| &run.start.value.task == task).collect();
            assert_eq!(
                recorded
                    .iter()
                    .filter(|run| matches!(run.start.value.why, RunWhy::SecondOpinion { .. }))
                    .count(),
                1
            );
            assert_eq!(
                recorded[0].start.value.why,
                RunWhy::SecondOpinion {
                    pr: agent_core::state::PrNumber::new(std::num::NonZeroU32::MIN),
                    pass: 1
                }
            );
        }
    }
}

#[tokio::test]
async fn a_confirmed_blocker_refuses_a_second_request_in_the_same_fix_pass() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,opinion-ask,opinion-ask,opinion-fix-pushed"),
            investigator("inv-r1", "opinion-probe:CONFIRMED"),
            investigator("inv-r2", "opinion-probe:CONFIRMED"),
            investigator("inv-r3", "opinion-probe:CONFIRMED"),
            investigator("inv-r4", "opinion-probe:CONFIRMED"),
        ],
        1,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig
        .run(&["claude-1"], ONE, agent_core::state::MergeMode::Autopilot, None)
        .await;
    rig.item_where("1.1", "second request corrected and code verified", |p| {
        matches!(p, ItemPhase::Done { .. })
    })
    .await;
    rig.stop().await.unwrap();
    let correction = rig.kept_prompt(&tasks[0], 4);
    assert!(
        correction.contains("FIXED (explanation and second opinion are forbidden)"),
        "{correction}"
    );
    assert!(correction.contains("FINDING 1"), "{correction}");
    assert_eq!(rig.kept_runs(&tasks[0]), 4);
    let state = rig.state();
    let item = &state.team(&team).unwrap().items()[0];
    assert_eq!(item.fix_passes().len(), 1);
    assert_eq!(item.findings().unwrap().opinions().len(), 2);
}

#[tokio::test]
async fn an_opinion_waits_for_a_free_eligible_investigator_then_uses_it() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,opinion-ask"),
            investigator("inv-r1", "opinion-release"),
            investigator("inv-r2", "opinion-release"),
            investigator("inv-r3", "opinion-release"),
        ],
        1,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig
        .run(&["claude-1"], ONE, agent_core::state::MergeMode::Autopilot, None)
        .await;
    let waiting = rig
        .state_where(
            "one assigned and one waiting for the only eligible investigator",
            |state| {
                let Some(ledger) = state.team(&team).unwrap().items()[0].findings() else {
                    return false;
                };
                ledger.opinions().len() == 2
                    && ledger
                        .opinions()
                        .values()
                        .filter(|o| matches!(o.stage, OpinionStage::Assigned { .. }))
                        .count()
                        == 1
                    && ledger
                        .opinions()
                        .values()
                        .filter(|o| matches!(o.stage, OpinionStage::Waiting))
                        .count()
                        == 1
            },
        )
        .await;
    assert!(matches!(
        waiting.team(&team).unwrap().items()[0].phase(),
        ItemPhase::Fixing { .. }
    ));
    assert_eq!(rig.github.reviews(1).len(), 2);
    std::fs::write(rig.home.root().join("opinion-release"), "ready").unwrap();
    rig.item_where("1.1", "both opinions settled after capacity released", |p| {
        matches!(p, ItemPhase::Done { .. })
    })
    .await;
    rig.stop().await.unwrap();
    let state = rig.state();
    let item = &state.team(&team).unwrap().items()[0];
    let investigators: std::collections::BTreeSet<_> = item
        .findings()
        .unwrap()
        .opinions()
        .values()
        .map(|opinion| {
            let OpinionStage::Reported { investigator, .. } = &opinion.stage else {
                panic!("missing report")
            };
            investigator.clone()
        })
        .collect();
    assert_eq!(investigators.len(), 1);
    assert!(
        !item.fix_passes()[0]
            .requested_by
            .iter()
            .any(|review| investigators.contains(&review.reviewer))
    );
    assert_eq!(rig.github.reviews(1).len(), 2);
}

/// The three second-opinion event kinds read back from the daemon's history.
fn opinion_events(
    reader: &agent_runtime::history::Reader,
    team: &agent_core::ids::TeamId,
) -> Vec<agent_core::history::EventRecord> {
    use agent_core::history::{HistoryEvent, Record};
    reader
        .team_records(team)
        .unwrap()
        .into_iter()
        .filter_map(|row| match row.record {
            Record::Event(event)
                if matches!(
                    event.event,
                    HistoryEvent::SecondOpinionAsked { .. }
                        | HistoryEvent::FindingConfirmed { .. }
                        | HistoryEvent::FindingNotConfirmed { .. }
                ) =>
            {
                Some(*event)
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn failed_silent_reports_and_exhausted_corrections_are_given_to_another_investigator() {
    for probe in ["opinion-retry:FAILED", "opinion-retry:EMPTY", "opinion-retry:INVALID"] {
        let mut rig = Rig::new(
            &[
                normal("claude-1", "work,opinion-ask"),
                investigator("inv-r1", probe),
                investigator("inv-r2", probe),
                investigator("inv-r3", probe),
                investigator("inv-r4", probe),
                investigator("inv-r5", probe),
                investigator("inv-r6", probe),
            ],
            1,
            false,
        )
        .await;
        rig.start().await;
        let (team, tasks) = rig
            .run(&["claude-1"], ONE, agent_core::state::MergeMode::Autopilot, None)
            .await;
        rig.state_where("opinion outcome or exhausted hold", |state| {
            let ledger = state.team(&team).unwrap().items()[0].findings();
            ledger.is_some_and(|ledger| {
                ledger.opinions().len() == 2
                    && ledger
                        .opinions()
                        .values()
                        .all(|opinion| matches!(opinion.stage, OpinionStage::Reported { .. }))
            }) || rig
                .notifications()
                .iter()
                .any(|n| n.contains("Second opinion needs attention"))
        })
        .await;
        let state = rig.state();
        let ledger = state.team(&team).unwrap().items()[0].findings().unwrap();
        assert!(
            ledger
                .opinions()
                .keys()
                .all(|number| ledger.opinion_attempts(*number).len() >= 2)
        );
        rig.item_where("1.1", "replacement opinions counted", |phase| {
            matches!(phase, ItemPhase::Done { .. })
        })
        .await;
        rig.stop().await.unwrap();
        let state = rig.state();
        let item = &state.team(&team).unwrap().items()[0];
        let ledger = item.findings().unwrap();
        for number in ledger.opinions().keys() {
            let attempts = ledger.opinion_attempts(*number);
            assert_eq!(attempts.len(), 2);
            assert_ne!(attempts[0].investigator, attempts[1].investigator);
            assert!(attempts[0].failure.is_some());
            assert_eq!(attempts[1].failure, None);
            assert!(
                !item.fix_passes()[0]
                    .requested_by
                    .iter()
                    .any(|review| attempts.iter().any(|attempt| attempt.investigator == review.reviewer))
            );
            for attempt in attempts {
                assert!(
                    rig.kept_prompt(&attempt.task, 1)
                        .contains(ledger.opinions()[number].request.head.as_str())
                );
            }
        }
        assert_eq!(item.fix_passes().len(), 1);
        assert_eq!(rig.kept_runs(&tasks[0]), 2);
        assert_eq!(
            rig.notifications()
                .iter()
                .filter(|n| n.contains("Second opinion needs attention"))
                .count(),
            0
        );
    }
}

#[tokio::test]
async fn restart_resumes_the_same_unanswered_opinions_once_and_keeps_answers() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,opinion-ask"),
            investigator("inv-r1", "opinion-release"),
            investigator("inv-r2", "opinion-release"),
            investigator("inv-r3", "opinion-release"),
            investigator("inv-r4", "opinion-release"),
        ],
        1,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig
        .run(&["claude-1"], ONE, agent_core::state::MergeMode::Autopilot, None)
        .await;
    let before = rig
        .state_where("opinion processes running", |state| {
            let team = state.team(&team).unwrap();
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
    let ledger = before.team(&team).unwrap().items()[0].findings().unwrap();
    let assignments: Vec<_> = ledger
        .opinions()
        .values()
        .map(|opinion| {
            let OpinionStage::Assigned { investigator, task } = &opinion.stage else {
                panic!("assigned")
            };
            (investigator.clone(), task.clone())
        })
        .collect();
    rig.stop().await.unwrap();
    std::fs::write(rig.home.root().join("opinion-release"), "ready").unwrap();
    rig.start().await;
    rig.state_where("recovered answers or erroneous hold", |state| {
        state.team(&team).unwrap().items()[0]
            .findings()
            .unwrap()
            .opinions()
            .values()
            .all(|opinion| matches!(opinion.stage, OpinionStage::Reported { .. }))
            || rig
                .notifications()
                .iter()
                .any(|n| n.contains("Second opinion needs attention"))
    })
    .await;
    let state = rig.state();
    let ledger = state.team(&team).unwrap().items()[0].findings().unwrap();
    for (opinion, (who, original)) in ledger.opinions().values().zip(&assignments) {
        assert!(
            matches!(&opinion.stage, OpinionStage::Reported { investigator, task, .. } if investigator == who && task == original)
        );
    }
    rig.item_where("1.1", "done after recovered opinions", |phase| {
        matches!(phase, ItemPhase::Done { .. })
    })
    .await;
    rig.stop().await.unwrap();
    rig.start().await;
    rig.stop().await.unwrap();
    for (_, task) in assignments {
        assert_eq!(rig.kept_runs(&task), 2);
    }
    assert_eq!(rig.state().team(&team).unwrap().items()[0].fix_passes().len(), 1);
    assert_eq!(
        rig.notifications()
            .iter()
            .filter(|n| n.contains("Second opinion needs attention"))
            .count(),
        0
    );
}

#[tokio::test]
async fn restart_replaces_opinions_whose_worktrees_have_gone() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,opinion-ask"),
            investigator("inv-r1", "opinion-release"),
            investigator("inv-r2", "opinion-release"),
            investigator("inv-r3", "opinion-release"),
            investigator("inv-r4", "opinion-release"),
            investigator("inv-r5", "opinion-release"),
            investigator("inv-r6", "opinion-release"),
        ],
        1,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig
        .run(&["claude-1"], ONE, agent_core::state::MergeMode::Autopilot, None)
        .await;
    let before = rig
        .state_where("assigned opinion processes", |state| {
            let team = state.team(&team).unwrap();
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
    let tasks: Vec<_> = before.team(&team).unwrap().items()[0]
        .findings()
        .unwrap()
        .opinions()
        .values()
        .map(|opinion| {
            let OpinionStage::Assigned { task, .. } = &opinion.stage else {
                panic!("assigned")
            };
            task.clone()
        })
        .collect();
    rig.stop().await.unwrap();
    for task in &tasks {
        std::fs::remove_dir_all(rig.worktree(task)).unwrap();
    }
    std::fs::write(rig.home.root().join("opinion-release"), "ready").unwrap();
    rig.start().await;
    let state = rig
        .state_where("redrawn assignments or failed missing worktree", |state| {
            let team = state.team(&team).unwrap();
            let ledger = team.items()[0].findings().unwrap();
            ledger.opinions().keys().all(|n| ledger.opinion_attempts(*n).len() == 2)
                || tasks
                    .iter()
                    .any(|task| team.find(task).unwrap().1.status() == agent_core::status::TaskStatus::Failed)
        })
        .await;
    let ledger = state.team(&team).unwrap().items()[0].findings().unwrap();
    for number in ledger.opinions().keys() {
        assert_eq!(ledger.opinion_attempts(*number).len(), 2);
        assert_eq!(
            ledger.opinion_attempts(*number)[0].failure,
            Some(agent_core::state::OpinionFailure::Interrupted)
        );
        assert!(!tasks.contains(&ledger.opinion_attempts(*number)[1].task));
    }
    rig.item_where("1.1", "done after redrawing missing tasks", |phase| {
        matches!(phase, ItemPhase::Done { .. })
    })
    .await;
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn replacement_history_keeps_each_failed_investigator_and_its_reason() {
    use agent_core::history::HistoryEvent;
    use agent_runtime::history::Reader;
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,opinion-ask"),
            investigator("inv-r1", "opinion-failure:FAILED"),
            investigator("inv-r2", "opinion-failure:FAILED"),
            investigator("inv-r3", "opinion-failure:FAILED"),
            investigator("inv-r4", "opinion-failure:FAILED"),
            investigator("inv-r5", "opinion-failure:FAILED"),
            investigator("inv-r6", "opinion-failure:FAILED"),
            investigator("inv-r7", "opinion-failure:FAILED"),
            investigator("inv-r8", "opinion-failure:FAILED"),
        ],
        1,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig
        .run(&["claude-1"], ONE, agent_core::state::MergeMode::Autopilot, None)
        .await;
    rig.wait_for("both exhausted notices", || {
        rig.notifications()
            .iter()
            .filter(|n| n.contains("Second opinion needs attention"))
            .count()
            == 2
    })
    .await;
    rig.stop().await.unwrap();
    let state = rig.state();
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let events = opinion_events(&reader, &team);
    let ledger = state.team(&team).unwrap().items()[0].findings().unwrap();
    for number in ledger.opinions().keys() {
        for attempt in ledger.opinion_attempts(*number) {
            assert!(
                events.iter().any(
                    |event| matches!(&event.event, HistoryEvent::SecondOpinionAsked { number: found, why, .. }
                if found == number && why.contains(attempt.investigator.as_str()) && why.contains("run failed"))
                ),
                "missing failure of {:?}: {events:?}",
                attempt.investigator
            );
        }
    }
}
