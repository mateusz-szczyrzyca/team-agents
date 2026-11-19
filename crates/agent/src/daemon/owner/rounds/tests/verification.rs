//! End-to-end rotating verification with persisted commit and fix context.

use agent_core::state::{FindingStatus, ItemPhase, MergeMode};

use super::rig::{ONE, Rig, git, investigator, normal};

#[tokio::test]
async fn fixes_are_verified_by_three_distinct_pairs_with_incremental_context_and_new_findings() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,work-pushed,work-pushed"),
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
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();
    let state = rig.state();
    let team = state.team(&team).unwrap();
    let progress = team.item("1.1".parse().unwrap()).unwrap();
    let mut asked: Vec<_> = progress
        .fix_passes()
        .iter()
        .flat_map(|pass| &pass.requested_by)
        .map(|review| review.task.clone())
        .collect();
    let final_reviews = rig.github.reviews(1);
    assert_eq!(final_reviews.len(), 6);
    let branch = format!("agents/claude-1/{}-1.1", tasks[0]);
    let previous = git(&rig.origin, &["rev-parse", &format!("refs/heads/{branch}~1")]);
    let first = git(&rig.origin, &["rev-parse", &format!("refs/heads/{branch}~2")]);
    let ledger = progress.findings().unwrap();
    assert_eq!(ledger.entries().len(), 4);
    assert!(
        ledger
            .entries()
            .iter()
            .all(|entry| entry.status == FindingStatus::Resolved)
    );
    for member in team.reviewers() {
        if rig.kept_runs(member.task()) > 0 {
            asked.push(member.task().clone());
        }
    }
    asked.sort();
    asked.dedup();
    assert_eq!(asked.len(), 6, "every investigator should have one pass");
    let mut passes = [0, 0, 0];
    for task in asked {
        assert_eq!(rig.kept_runs(&task), 1);
        let prompt = rig.kept_prompt(&task, 1);
        if prompt.contains("verification pass 2") {
            passes[1] += 1;
            assert!(prompt.contains(&format!("git diff {first}..HEAD")), "{prompt}");
            assert!(prompt.contains("Finished the item and pushed it."), "{prompt}");
            assert!(prompt.contains("1. [MAJOR] TDD-2 src/lib.rs:7"), "{prompt}");
            assert!(prompt.contains("CHECK: read src/lib.rs:7"), "{prompt}");
        } else if prompt.contains("verification pass 3") {
            passes[2] += 1;
            assert!(prompt.contains(&format!("git diff {previous}..HEAD")), "{prompt}");
            assert!(prompt.contains("3. [MAJOR] TDD-2 src/lib.rs:7"), "{prompt}");
            assert!(!prompt.contains("1. [MAJOR] TDD-2 src/lib.rs:7"), "{prompt}");
        } else {
            passes[0] += 1;
            assert!(prompt.contains("A PULL REQUEST TO REVIEW"));
        }
    }
    assert_eq!(passes, [2, 2, 2]);
}

#[tokio::test]
async fn verification_diff_uses_the_reviewed_head_or_explains_rewritten_history() {
    use agent_core::config::{AbsolutePath, Language};
    use agent_runtime::autopilot_prompts::AutopilotPrompts;
    use agent_runtime::git::{CommitIdentity, Git};
    use std::path::Path;

    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-b", "main"]);
    git(dir.path(), &["config", "user.name", "Test Agent"]);
    git(dir.path(), &["config", "user.email", "agent@example.com"]);
    git(dir.path(), &["commit", "--allow-empty", "-m", "Reviewed head"]);
    let previous = git(dir.path(), &["rev-parse", "HEAD"]).parse().unwrap();
    let repository = Git::new(
        dir.path().to_str().unwrap().parse::<AbsolutePath>().unwrap(),
        CommitIdentity::new("Test Agent".into(), "agent@example.com".into(), None),
    );
    let texts = AutopilotPrompts::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts"),
        &Language::default(),
    )
    .await
    .unwrap();
    let changes = super::prompts::verification_changes(&repository, &texts, Some(&previous), "main")
        .await
        .unwrap();
    assert!(changes.contains(&format!("git diff {previous}..HEAD")), "{changes}");

    git(dir.path(), &["checkout", "--orphan", "rewritten"]);
    git(dir.path(), &["commit", "--allow-empty", "-m", "Rewritten history"]);
    let changes = super::prompts::verification_changes(&repository, &texts, Some(&previous), "main")
        .await
        .unwrap();
    assert!(changes.contains("git diff main...HEAD"), "{changes}");
    assert!(changes.contains("no longer an ancestor"), "{changes}");
    assert!(changes.contains("rewrote"), "{changes}");
}
