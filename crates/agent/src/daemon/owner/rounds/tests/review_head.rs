//! The head of a posted review, commit linking, and review format.

use agent_core::state::{ItemPhase, MergeMode};

use super::fake_github::SLUG;
use super::rig::{Rig, git, investigator, normal};

const APPROVING: &str = "done:VERDICT: APPROVE\\nSUMMARY: fine";
const R1_SCENARIO: &str = "done:VERDICT: REQUEST_CHANGES\\nBLOCKERS\\n1. [TDD-2] see `HEAD` for issue,done:VERDICT: APPROVE\\nSUMMARY: fixed in `HEAD`";

#[tokio::test]
async fn review_head_and_commit_links_in_reviews_pr_description_and_update_comment() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,work-pushed"),
            investigator("inv-r1", R1_SCENARIO),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (_, tasks) = rig
        .run(&["claude-1"], super::rig::ONE, MergeMode::Autopilot, None)
        .await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let branch = format!("agents/claude-1/{}-1.1", tasks[0]);
    let initial_head = git(&rig.origin, &["rev-parse", &format!("refs/heads/{branch}~1")]);
    let final_head = rig.remote_tip(&branch);

    // 1. PR description links its commit ID
    let pulls = rig.github.pulls();
    assert_eq!(pulls.len(), 1);
    let pr_body = &pulls[0].body;
    let short_initial = initial_head.get(..7).unwrap();
    let expected_pr_link = format!("[{short_initial}](https://github.com/{SLUG}/commit/{initial_head})");
    assert!(
        pr_body.contains(&expected_pr_link),
        "PR body missing commit link: {pr_body}"
    );

    // 2. Updated branch comment links its commit ID
    let comments = rig.github.comments(1);
    assert_eq!(comments.len(), 1);
    let expected_update_link =
        format!("The branch was updated to [`{final_head}`](https://github.com/{SLUG}/commit/{final_head})");
    assert!(
        comments[0].contains(&expected_update_link),
        "Update comment missing commit link: {}",
        comments[0]
    );

    // 3. Reviews: 4 reviews total (2 on pass 1, 2 on pass 2)
    let reviews = rig.github.reviews(1);
    assert_eq!(reviews.len(), 4);

    // Pass 1: inv-r1 requested changes
    let r1_pass1 = reviews
        .iter()
        .find(|r| {
            r["body"].as_str().unwrap().contains("Review by `inv-r1`") && r["body"].as_str().unwrap().contains("pass 1")
        })
        .unwrap();
    let body1 = r1_pass1["body"].as_str().unwrap();
    assert!(body1.starts_with("### Review by `inv-r1` (claude)\n## ❌ $\\color{#d1242f}{\\textbf{REQUEST CHANGES}}$\nReview pass 1 of commit [`"));
    assert!(body1.contains(&format!("https://github.com/{SLUG}/commit/{initial_head}")));
    assert!(body1.contains("\n---\n\nBLOCKERS\n1. [TDD-2] see"));
    assert!(!body1.contains("VERDICT: REQUEST_CHANGES"));
    // Commit in backticks in report is linked
    assert!(body1.contains(&format!(
        "[`{initial_head}`](https://github.com/{SLUG}/commit/{initial_head})"
    )));

    // Pass 1: inv-r2 approved
    let r2_pass1 = reviews
        .iter()
        .find(|r| {
            r["body"].as_str().unwrap().contains("Review by `inv-r2`") && r["body"].as_str().unwrap().contains("pass 1")
        })
        .unwrap();
    let body2 = r2_pass1["body"].as_str().unwrap();
    assert!(body2.starts_with(
        "### Review by `inv-r2` (claude)\n## ✅✅ $\\color{#1a7f37}{\\textbf{APPROVED}}$\nReview pass 1 of commit [`"
    ));
    assert!(body2.contains("\n---\n\nSUMMARY: fine"));
    assert!(!body2.contains("VERDICT: APPROVE"));

    // Pass 2: inv-r1 approved on second pass
    let r1_pass2 = reviews
        .iter()
        .find(|r| {
            r["body"].as_str().unwrap().contains("Review by `inv-r1`") && r["body"].as_str().unwrap().contains("pass 2")
        })
        .unwrap();
    let body3 = r1_pass2["body"].as_str().unwrap();
    assert!(body3.starts_with(
        "### Review by `inv-r1` (claude)\n## ✅✅ $\\color{#1a7f37}{\\textbf{APPROVED}}$\nReview pass 2 of commit [`"
    ));
    assert!(body3.contains(&format!("https://github.com/{SLUG}/commit/{final_head}")));
    assert!(body3.contains("\n---\n\nSUMMARY: fixed in"));
    assert!(!body3.contains("VERDICT: APPROVE"));
}
