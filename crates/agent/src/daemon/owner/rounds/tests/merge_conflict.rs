//! The merge conflict pass in a daemon with a fake GitHub, a bare `origin` and a stand-in for the lead: a refusal of an
//! approved merge for a conflict sends the lead to merge the base, and the merge is tried again once the lead's change is
//! pushed and nothing but the allowed files changed.

use agent_core::state::{ItemPhase, MergeMode, State};

use super::fake_github::{MergeAnswer, SLUG};
use super::rig::{ONE, Rig, git, investigator, normal};

const APPROVING: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// Whether the item 1.1 of the only team is in a merge conflict pass.
fn in_conflict_pass(state: &State) -> bool {
    state.teams()[0]
        .item("1.1".parse().unwrap())
        .is_some_and(|item| item.merge_conflict().is_some())
}

/// Moves the base on `origin`: a file the task never touches, so that the lead's merge of it makes a commit.
fn move_base(rig: &Rig) {
    std::fs::write(rig.repo.join("base-file.txt"), "moved\n").unwrap();
    git(&rig.repo, &["add", "base-file.txt"]);
    git(&rig.repo, &["commit", "-q", "-m", "Move the base"]);
    let origin = rig.origin.to_str().unwrap();
    git(&rig.repo, &["push", "-q", origin, "HEAD:refs/heads/main"]);
}

#[tokio::test]
async fn a_conflict_refusal_sends_the_lead_to_merge_the_base_and_the_merge_is_tried_again() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,merge-base"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    // "Pull Request is not mergeable" is a conflict with the base.
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    rig.state_where("the lead is sent to merge the base", in_conflict_pass)
        .await;

    // GitHub merges from now on; the base moves, and the lead merges it.
    rig.github.answer_merges(MergeAnswer::Allow);
    move_base(&rig);
    std::fs::write(rig.worktree(&task).join(".fake-base-ready"), "").unwrap();

    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    assert!(matches!(
        state.teams()[0].item("1.1".parse().unwrap()).unwrap().phase(),
        ItemPhase::Done { .. }
    ));
    assert_eq!(
        rig.github.calls("PUT", &format!("/repos/{SLUG}/pulls/1/merge")).len(),
        2,
        "the refused merge and the merge tried again"
    );
    let told: Vec<_> = rig
        .notifications()
        .into_iter()
        .filter(|text| text.contains("could not be merged"))
        .collect();
    assert!(told.is_empty(), "{told:?}");
}
