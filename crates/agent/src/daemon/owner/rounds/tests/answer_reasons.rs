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
