//! Fix and verification responses with reasons complete without correction runs.
use super::rig::{ONE, Rig, investigator, normal};
use agent_core::state::{FindingStatus, ItemPhase, MergeMode};

#[tokio::test]
async fn reasoned_fixes_with_minor_lines_and_reasoned_verdicts_are_accepted_on_first_try() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-reasons-minor"),
            investigator("inv-r1", "review-reasons"),
            investigator("inv-r2", "review-reasons"),
            investigator("inv-r3", "review-reasons"),
            investigator("inv-r4", "review-reasons"),
        ],
        1,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "reasoned answers accepted", |phase| {
        matches!(phase, ItemPhase::Done { .. })
    })
    .await;
    rig.stop().await.unwrap();

    let state = rig.state();
    let team = state.team(&team).unwrap();
    let item = &team.items()[0];
    assert_eq!(item.fix_passes().len(), 1);
    assert_eq!(rig.kept_runs(&tasks[0]), 2, "one implementation run and one fix run");
    assert_eq!(rig.github.reviews(1).len(), 4);
    let entries = item.findings().unwrap().entries();
    assert_eq!(entries.len(), 4);
    assert!(entries.iter().all(|entry| entry.status == FindingStatus::Resolved));
    let mut verification_count = 0;
    for member in team.reviewers() {
        assert_eq!(rig.kept_runs(member.task()), 1, "no reviewer correction run");
        let prompt = rig.kept_prompt(member.task(), 1);
        if prompt.contains("# OPEN FINDINGS TO VERIFY") {
            verification_count += 1;
            assert!(prompt.contains("FIXED — café regression added"), "{prompt}");
        }
    }
    assert_eq!(verification_count, 2);
    let posted = format!("{:?}", rig.github.requests());
    assert!(posted.contains("RESOLVED — café regression checked"), "{posted}");
}

#[tokio::test]
async fn refused_lead_response_quotes_unknown_number_in_correction_and_its_own_notice() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-work:99"),
            investigator("inv-r1", "review-reasons"),
            investigator("inv-r2", "review-reasons"),
        ],
        1,
        false,
    )
    .await;
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let notice = rig.notification_with("The lead's fix response was refused").await;
    rig.stop().await.unwrap();

    let correction = rig.kept_prompt(&tasks[0], 3);
    for text in [&notice, &correction] {
        assert!(text.contains("FINDING 99: FIXED"), "{text}");
        assert!(text.contains("unknown number"), "{text}");
        assert!(text.contains("FINDING 1: FIXED or EXPLAINED"), "{text}");
        assert!(text.contains("FINDING 3: FIXED or EXPLAINED"), "{text}");
        assert!(!text.contains("could not be published"), "{text}");
    }
    assert!(notice.contains("3 times"), "{notice}");
    assert!(notice.contains(&format!("agent resume {}", tasks[0])), "{notice}");
    assert_eq!(rig.kept_runs(&tasks[0]), 4);
}

#[tokio::test]
async fn refused_report_with_verdict_quotes_the_answer_instead_of_claiming_no_verdict() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nFINDING 99: RESOLVED"),
            investigator("inv-r2", "done:VERDICT: APPROVE"),
        ],
        1,
        false,
    )
    .await;
    rig.start().await;
    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let notice = rig.notification_with("The reviewer's report was refused").await;
    rig.stop().await.unwrap();

    assert!(notice.contains("FINDING 99: RESOLVED"), "{notice}");
    assert!(notice.contains("unknown number"), "{notice}");
    assert!(notice.contains("3 times"), "{notice}");
    assert!(!notice.contains("gave no verdict"), "{notice}");
    assert!(!notice.contains("could not be published"), "{notice}");
}
