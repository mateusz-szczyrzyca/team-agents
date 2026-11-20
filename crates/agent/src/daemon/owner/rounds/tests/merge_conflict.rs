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

#[tokio::test]
async fn a_conflict_pass_that_changes_another_file_is_reviewed_again_before_the_merge() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,merge-base-other"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    rig.state_where("the lead is sent to merge the base", in_conflict_pass)
        .await;

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
    // The change to `resolved.txt` is reviewed: the reviewers approve the pull request a second time.
    let reviews = rig
        .github
        .calls("POST", &format!("/repos/{SLUG}/pulls/1/reviews"))
        .len();
    assert!(reviews > 2, "reviews asked again after the conflict pass: {reviews}");
    assert_eq!(
        rig.github.calls("PUT", &format!("/repos/{SLUG}/pulls/1/merge")).len(),
        2
    );
}

#[tokio::test]
async fn when_the_conflict_pass_limit_is_reached_the_human_is_asked_once_in_a_questions_file() {
    use agent_core::status::TaskStatus;
    use std::io::Write as _;
    use std::num::NonZeroU32;

    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,merge-base,ask-here"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    {
        let mut config = std::fs::OpenOptions::new()
            .append(true)
            .open(rig.home.config_file())
            .unwrap();
        writeln!(config, "[merge]\nconflict_passes = 1").unwrap();
    }
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    rig.state_where("the lead is sent to merge the base", in_conflict_pass)
        .await;

    move_base(&rig);
    std::fs::write(rig.worktree(&task).join(".fake-base-ready"), "").unwrap();

    rig.status_is(&task, TaskStatus::Questions).await;
    rig.stop().await.unwrap();

    assert!(rig.home.questions_file(&task, NonZeroU32::MIN).is_file());
    assert!(!rig.home.questions_file(&task, NonZeroU32::new(2).unwrap()).exists());
    let prompt = std::fs::read_to_string(rig.worktree(&task).join(".fake-prompt-3")).unwrap();
    assert!(
        prompt.contains("## Merge conflict on pull request #1 needs the human"),
        "{prompt}"
    );
    assert!(prompt.contains("(1 allowed)"), "{prompt}");
    let told: Vec<_> = rig
        .notifications()
        .into_iter()
        .filter(|text| text.contains("could not be merged"))
        .collect();
    assert!(told.is_empty(), "{told:?}");
}

#[tokio::test]
async fn a_restart_while_the_lead_merges_the_base_and_after_the_push_completes_the_merge() {
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
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    rig.state_where("the lead is sent to merge the base", in_conflict_pass)
        .await;
    rig.wait_for("the lead's conflict pass run to start", || {
        std::fs::read_to_string(rig.worktree(&task).join(".fake-runs")).is_ok_and(|runs| runs.trim() == "2")
    })
    .await;

    // Restart while the lead is at work in the conflict pass.
    rig.stop().await.unwrap();
    rig.github.answer_merges(MergeAnswer::Refuse(500));
    rig.start().await;

    move_base(&rig);
    std::fs::write(rig.worktree(&task).join(".fake-base-ready"), "").unwrap();

    rig.state_where("the conflict pass is pushed and ready to merge", |state| {
        state.teams()[0].item("1.1".parse().unwrap()).is_some_and(|item| {
            item.merge_passes() == 1
                && item.merge_conflict().is_none()
                && matches!(item.phase(), ItemPhase::Approved { .. })
        })
    })
    .await;

    // Restart after the push and before the merge succeeds.
    rig.stop().await.unwrap();
    rig.github.answer_merges(MergeAnswer::Allow);
    rig.start().await;

    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    assert!(matches!(
        state.teams()[0].item("1.1".parse().unwrap()).unwrap().phase(),
        ItemPhase::Done { .. }
    ));
}

#[test]
fn an_old_state_with_a_round_waiting_after_a_refused_merge_loads() {
    let mut raw: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../agent-core/tests/fixtures/state-before-delivery.json"
    ))
    .unwrap();
    let item = &mut raw["teams"][0]["items"][2];
    assert!(item.get("merge_passes").is_none());
    assert!(item.get("merge_conflict").is_none());
    item["phase"] = serde_json::json!({
        "phase": "approved",
        "pr": {
            "number": 7,
            "branch": "agents/codex-1/item-1-3",
            "opened_at": "2026-11-18T19:20:00Z"
        },
        "reviews": [
            {
                "reviewer": "inv-codex",
                "task": "task-20261111-193007-k3x9",
                "requested_at": "2026-11-18T19:25:00Z",
                "outcome": {
                    "status": "submitted",
                    "verdict": "APPROVE",
                    "at": "2026-11-18T19:28:00Z"
                }
            },
            {
                "reviewer": "inv-gemini",
                "task": "task-20261111-193008-k3x9",
                "requested_at": "2026-11-18T19:25:00Z",
                "outcome": {
                    "status": "submitted",
                    "verdict": "APPROVE",
                    "at": "2026-11-18T19:28:30Z"
                }
            }
        ]
    });

    let loaded = State::from_json(&serde_json::to_string(&raw).unwrap()).unwrap();
    let progress = loaded.teams()[0].item("1.3".parse().unwrap()).unwrap();

    assert!(matches!(progress.phase(), ItemPhase::Approved { .. }));
    assert_eq!(progress.merge_passes(), 0);
    assert_eq!(progress.merge_conflict(), None);
}

#[tokio::test]
async fn a_verification_review_after_a_conflict_pass_is_recorded_and_posted_as_pass_two() {
    use agent_core::state::PrNumber;
    use std::num::NonZeroU32;

    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,merge-base-other"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    rig.state_where("the lead is sent to merge the base", in_conflict_pass)
        .await;

    rig.github.answer_merges(MergeAnswer::Allow);
    move_base(&rig);
    std::fs::write(rig.worktree(&task).join(".fake-base-ready"), "").unwrap();

    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let posted = rig.github.calls("POST", &format!("/repos/{SLUG}/pulls/1/reviews"));
    assert_eq!(posted.len(), 4);
    assert!(posted[0].body.contains("Review pass 1"), "{}", posted[0].body);
    assert!(posted[1].body.contains("Review pass 1"), "{}", posted[1].body);
    assert!(posted[2].body.contains("Review pass 2"), "{}", posted[2].body);
    assert!(posted[3].body.contains("Review pass 2"), "{}", posted[3].body);
    let pr = PrNumber::new(NonZeroU32::MIN);
    for reviewer in state.teams()[0].reviewers() {
        assert!(rig.home.review_report_file(reviewer.task(), pr, 0).is_file());
        assert!(rig.home.review_report_file(reviewer.task(), pr, 1).is_file());
    }
}
