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
        writeln!(config, "[merge]\nconflict_passes = 1\ninvestigator_pass = false").unwrap();
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

#[tokio::test]
async fn a_conflict_pass_whose_lead_fails_asks_the_human_once_in_a_questions_file() {
    use agent_core::state::MergeConflict;
    use agent_core::status::TaskStatus;
    use std::num::NonZeroU32;

    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fail,ask-here"),
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
    rig.status_is(&task, TaskStatus::Questions).await;
    let state = rig.state();
    rig.stop().await.unwrap();

    assert!(rig.home.questions_file(&task, NonZeroU32::MIN).is_file());
    assert!(matches!(
        state.teams()[0].item("1.1".parse().unwrap()).unwrap().merge_conflict(),
        Some(MergeConflict::AskingHuman { .. })
    ));
}

#[tokio::test]
async fn a_pull_request_merged_by_the_human_while_a_conflict_question_is_open_completes_the_item() {
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
        writeln!(config, "[merge]\nconflict_passes = 1\ninvestigator_pass = false").unwrap();
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
    assert!(rig.home.questions_file(&task, NonZeroU32::MIN).is_file());

    rig.github.merged_by_a_human(1);
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let item = state.teams()[0].item("1.1".parse().unwrap()).unwrap();
    assert!(matches!(item.phase(), ItemPhase::Done { .. }));
    assert_eq!(item.merge_conflict(), None);
}

/// Runs the conflict-pass limit scenario where the human answers the conflict question and the lead's fourth run
/// (`fourth_action`) commits an unallowed file (`work-4.txt`), asserting that the commit is on the remote branch and
/// verified by a second review pass before the pull request merges.
async fn assert_answered_conflict_correction_is_pushed_and_verified(scenario: &'static str) {
    use agent_core::protocol::{AnswerInput, AnswerList, AnswerRequest, Client, Command, Reply};
    use agent_core::questions::Verdict;
    use agent_core::status::TaskStatus;
    use std::io::Write as _;
    use std::num::NonZeroU32;

    let mut rig = Rig::new(
        &[
            normal("claude-1", scenario),
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
        writeln!(config, "[merge]\nconflict_passes = 1\ninvestigator_pass = false").unwrap();
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
    rig.github.answer_merges(MergeAnswer::Allow);
    let answers = AnswerList::try_from(vec![AnswerInput {
        question: NonZeroU32::MIN,
        verdict: Verdict::from_text("ok").unwrap().unwrap(),
    }])
    .unwrap();
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: task.clone(),
                number: NonZeroU32::MIN,
                answers,
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "{reply:?}");

    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let item = state.teams()[0].item("1.1".parse().unwrap()).unwrap();
    assert!(matches!(item.phase(), ItemPhase::Done { .. }));
    assert_eq!(item.merge_conflict(), None);
    assert_eq!(
        git(&rig.origin, &["show", "refs/heads/main:work-4.txt"]),
        "run 4 of claude-1"
    );
    assert_eq!(
        rig.github
            .calls("POST", &format!("/repos/{SLUG}/pulls/1/reviews"))
            .len(),
        4,
        "the post-answer change to work-4.txt must be verified before merge"
    );
}

#[tokio::test]
async fn an_answered_conflict_question_whose_lead_changes_another_file_is_pushed_and_verified_before_merge() {
    assert_answered_conflict_correction_is_pushed_and_verified("work,merge-base,ask-here,work").await;
    assert_answered_conflict_correction_is_pushed_and_verified("work,merge-base,ask-here,work-pushed").await;
}

#[tokio::test]
async fn a_fix_pass_after_a_conflict_verification_review_attaches_the_verification_review_reports() {
    const REJECT_ON_VERIFY: &str = "verify:VERDICT: APPROVE\\nSUMMARY: initial approval,\
        verify:VERDICT: REQUEST_CHANGES\\nSUMMARY: verification rejection\\nFINDINGS\\n\
        1. [MAJOR] TDD-2 resolved.txt:1 — conflict resolution is untested → add coverage\\n   \
        CHECK: read resolved.txt,\
        verify:VERDICT: APPROVE\\nSUMMARY: final approval";

    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,merge-base-other,fix-work"),
            investigator("inv-r1", REJECT_ON_VERIFY),
            investigator("inv-r2", REJECT_ON_VERIFY),
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

    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let prompt = rig.kept_prompt(&task, 3);
    assert!(
        prompt.contains("SUMMARY: verification rejection"),
        "fix pass prompt after a conflict verification must attach the verification review reports:\n{prompt}"
    );
    assert!(!prompt.contains("SUMMARY: initial approval"), "{prompt}");
}

#[tokio::test]
async fn an_answered_conflict_question_on_a_legacy_ledger_without_verification_completes_the_item() {
    use agent_core::protocol::{AnswerInput, AnswerList, AnswerRequest, Client, Command, Reply};
    use agent_core::questions::Verdict;
    use agent_core::status::TaskStatus;
    use std::io::Write as _;
    use std::num::NonZeroU32;

    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,merge-base,ask-here,work-pushed"),
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
        writeln!(config, "[merge]\nconflict_passes = 1\ninvestigator_pass = false").unwrap();
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

    let mut raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(rig.home.state_file()).unwrap()).unwrap();
    for item in raw["teams"][0]["items"].as_array_mut().unwrap() {
        if let Some(findings) = item.get_mut("findings").and_then(serde_json::Value::as_object_mut) {
            findings.remove("verification");
        }
    }
    std::fs::write(rig.home.state_file(), serde_json::to_string_pretty(&raw).unwrap()).unwrap();

    rig.github.answer_merges(MergeAnswer::Allow);
    rig.start().await;

    let answers = AnswerList::try_from(vec![AnswerInput {
        question: NonZeroU32::MIN,
        verdict: Verdict::from_text("ok").unwrap().unwrap(),
    }])
    .unwrap();
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: task.clone(),
                number: NonZeroU32::MIN,
                answers,
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "{reply:?}");

    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let item = state.teams()[0].item("1.1".parse().unwrap()).unwrap();
    assert!(matches!(item.phase(), ItemPhase::Done { .. }));
    assert_eq!(item.merge_conflict(), None);
}

#[tokio::test]
async fn when_fix_passes_are_exhausted_after_a_conflict_verification_review_the_human_question_prompt_attaches_the_verification_reports()
 {
    use agent_core::status::TaskStatus;

    const REJECT_ON_VERIFY: &str = "verify:VERDICT: APPROVE\\nSUMMARY: initial approval,\
        verify:VERDICT: REQUEST_CHANGES\\nSUMMARY: verification rejection\\nFINDINGS\\n\
        1. [MAJOR] TDD-2 resolved.txt:1 — conflict resolution is untested → add coverage\\n   \
        CHECK: read resolved.txt";

    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,merge-base-other,ask-here"),
            investigator("inv-r1", REJECT_ON_VERIFY),
            investigator("inv-r2", REJECT_ON_VERIFY),
        ],
        0,
        false,
    )
    .await;
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

    let prompt = std::fs::read_to_string(rig.worktree(&task).join(".fake-prompt-3")).unwrap();
    assert!(
        prompt.contains("SUMMARY: verification rejection"),
        "escalation prompt after a conflict verification must attach the verification review reports:\n{prompt}"
    );
    assert!(!prompt.contains("SUMMARY: initial approval"), "{prompt}");
}

#[tokio::test]
async fn an_answered_conflict_question_after_a_conflict_verification_review_runs_pass_three_and_keeps_all_three_report_slots()
 {
    use agent_core::protocol::{AnswerInput, AnswerList, AnswerRequest, Client, Command, Reply};
    use agent_core::questions::Verdict;
    use agent_core::state::PrNumber;
    use agent_core::status::TaskStatus;
    use std::io::Write as _;
    use std::num::NonZeroU32;

    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,merge-base-other,ask-here,work"),
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
        writeln!(config, "[merge]\nconflict_passes = 1\ninvestigator_pass = false").unwrap();
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
    rig.github.answer_merges(MergeAnswer::Allow);
    let answers = AnswerList::try_from(vec![AnswerInput {
        question: NonZeroU32::MIN,
        verdict: Verdict::from_text("ok").unwrap().unwrap(),
    }])
    .unwrap();
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: task.clone(),
                number: NonZeroU32::MIN,
                answers,
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "{reply:?}");

    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let posted = rig.github.calls("POST", &format!("/repos/{SLUG}/pulls/1/reviews"));
    assert_eq!(posted.len(), 6);
    assert!(posted[0].body.contains("Review pass 1"), "{}", posted[0].body);
    assert!(posted[1].body.contains("Review pass 1"), "{}", posted[1].body);
    assert!(posted[2].body.contains("Review pass 2"), "{}", posted[2].body);
    assert!(posted[3].body.contains("Review pass 2"), "{}", posted[3].body);
    assert!(posted[4].body.contains("Review pass 3"), "{}", posted[4].body);
    assert!(posted[5].body.contains("Review pass 3"), "{}", posted[5].body);
    let pr = PrNumber::new(NonZeroU32::MIN);
    for reviewer in state.teams()[0].reviewers() {
        assert!(rig.home.review_report_file(reviewer.task(), pr, 0).is_file());
        assert!(rig.home.review_report_file(reviewer.task(), pr, 1).is_file());
        assert!(rig.home.review_report_file(reviewer.task(), pr, 2).is_file());
    }
}

#[tokio::test]
async fn reopening_review_after_allowed_only_conflict_passes_and_an_answered_question_advances_each_pass_identity() {
    use agent_core::protocol::{AnswerInput, AnswerList, AnswerRequest, Client, Command, Reply};
    use agent_core::questions::Verdict;
    use agent_core::state::{MergeConflict, PrNumber};
    use agent_core::status::TaskStatus;
    use std::num::NonZeroU32;

    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,merge-base,merge-base,merge-base-other,ask-here,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    std::fs::write(
        rig.home.config_file(),
        format!(
            "{}\n[merge]\ninvestigator_pass = false\n",
            std::fs::read_to_string(rig.home.config_file()).unwrap()
        ),
    )
    .unwrap();
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    for pass in 1..=3 {
        rig.state_where(&format!("conflict pass {pass} to begin"), |state| {
            state.teams()[0].item("1.1".parse().unwrap()).is_some_and(|item| {
                item.merge_passes() == pass && matches!(item.merge_conflict(), Some(MergeConflict::MergingBase { .. }))
            })
        })
        .await;
        std::fs::write(rig.repo.join("base-file.txt"), format!("moved {pass}\n")).unwrap();
        git(&rig.repo, &["add", "base-file.txt"]);
        git(&rig.repo, &["commit", "-q", "-m", &format!("Move the base {pass}")]);
        let origin = rig.origin.to_str().unwrap();
        git(&rig.repo, &["push", "-q", origin, "HEAD:refs/heads/main"]);
        std::fs::write(rig.worktree(&task).join(".fake-base-ready"), "").unwrap();
    }

    rig.status_is(&task, TaskStatus::Questions).await;
    rig.github.answer_merges(MergeAnswer::Allow);
    let answers = AnswerList::try_from(vec![AnswerInput {
        question: NonZeroU32::MIN,
        verdict: Verdict::from_text("ok").unwrap().unwrap(),
    }])
    .unwrap();
    let reply = crate::client::Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: task.clone(),
                number: NonZeroU32::MIN,
                answers,
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "{reply:?}");

    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let posted = rig.github.calls("POST", &format!("/repos/{SLUG}/pulls/1/reviews"));
    assert_eq!(posted.len(), 6);
    assert!(posted[0].body.contains("Review pass 1"), "{}", posted[0].body);
    assert!(posted[1].body.contains("Review pass 1"), "{}", posted[1].body);
    assert!(posted[2].body.contains("Review pass 2"), "{}", posted[2].body);
    assert!(posted[3].body.contains("Review pass 2"), "{}", posted[3].body);
    assert!(posted[4].body.contains("Review pass 3"), "{}", posted[4].body);
    assert!(posted[5].body.contains("Review pass 3"), "{}", posted[5].body);
    let pr = PrNumber::new(NonZeroU32::MIN);
    for reviewer in state.teams()[0].reviewers() {
        assert!(rig.home.review_report_file(reviewer.task(), pr, 0).is_file());
        assert!(rig.home.review_report_file(reviewer.task(), pr, 1).is_file());
        assert!(rig.home.review_report_file(reviewer.task(), pr, 2).is_file());
    }
}

/// Runs three real conflict passes, leaving the automatic investigation (or old human fallback) observable.
async fn spent_conflict_passes(
    review_and_answer: &'static str,
    lead: &'static str,
    enabled: bool,
) -> (Rig, agent_core::ids::TaskId) {
    use std::io::Write as _;
    let mut rig = Rig::new(
        &[
            normal("claude-1", lead),
            investigator("inv-r1", review_and_answer),
            investigator("inv-r2", review_and_answer),
        ],
        5,
        true,
    )
    .await;
    // The key is written only for the disabled compatibility case, so the default is exercised too.
    if !enabled {
        writeln!(
            std::fs::OpenOptions::new()
                .append(true)
                .open(rig.home.config_file())
                .unwrap(),
            "[merge]\ninvestigator_pass = false"
        )
        .unwrap();
    }
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    for pass in 1..=3 {
        rig.wait_for("next conflict run", || rig.runs(&task) > pass).await;
        std::fs::write(rig.repo.join("base-file.txt"), format!("base pass {pass}\n")).unwrap();
        git(&rig.repo, &["add", "base-file.txt"]);
        git(&rig.repo, &["commit", "-q", "-m", "Move the base for the next pass"]);
        git(
            &rig.repo,
            &["push", "-q", rig.origin.to_str().unwrap(), "HEAD:refs/heads/main"],
        );
        std::fs::write(rig.worktree(&task).join(".fake-base-ready"), "").unwrap();
    }
    rig.state_where("investigation or human fallback after spent passes", |state| {
        let item = state.teams()[0].item("1.1".parse().unwrap()).unwrap();
        let value = serde_json::to_value(item).unwrap();
        value.get("conflict_investigation").is_some()
            || matches!(
                item.merge_conflict(),
                Some(agent_core::state::MergeConflict::AskingHuman { .. })
            )
    })
    .await;
    (rig, task)
}

/// Requires a durable request before checking any dependent investigation lifecycle behaviour.
fn assert_investigation_filed(rig: &Rig) {
    let state = rig.state();
    let item = serde_json::to_value(state.teams()[0].item("1.1".parse().unwrap()).unwrap()).unwrap();
    assert_eq!(item["conflict_investigation"]["id"], 1);
}

/// The stored request record, read through the bus's persisted public format.
fn conflict_requests(rig: &Rig) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(rig.home.root().join("bus/state.json")).unwrap()).unwrap()
}

#[tokio::test]
async fn a_conflict_after_three_lead_passes_goes_to_an_investigator_before_the_human() {
    let (mut rig, task) = spent_conflict_passes(
        "conflict-investigator:hold",
        "work,merge-base,merge-base,merge-base,hold,ask-here",
        true,
    )
    .await;
    let state = rig.state();
    rig.stop().await.unwrap();
    let item = serde_json::to_value(state.teams()[0].item("1.1".parse().unwrap()).unwrap()).unwrap();
    assert_eq!(item["conflict_investigation"]["id"], 1);
    assert_eq!(rig.runs(&task), 4);
    let requests = conflict_requests(&rig);
    assert_eq!(requests["investigations"]["1"]["item"], "1.1");
    assert_eq!(requests["investigations"]["1"]["from_task"], task.as_str());
    assert_eq!(
        requests["investigations"]["1"]["question"],
        "Pull request #1 conflicts with main after 3 passes of the lead. Resolve it locally as your instructions say."
    );
}

#[tokio::test]
async fn the_lead_gets_one_more_pass_with_the_investigators_answer() {
    let (mut rig, task) = spent_conflict_passes(
        "conflict-investigator:inv-done:git merge origin/main — résolution",
        "work,merge-base,merge-base,merge-base,hold,ask-here",
        true,
    )
    .await;
    assert_investigation_filed(&rig);
    rig.wait_for("the extra lead run", || rig.runs(&task) >= 5).await;
    rig.wait_for("the extra prompt to be written", || {
        rig.worktree(&task).join(".fake-prompt-5").is_file()
    })
    .await;
    let prompt = std::fs::read_to_string(rig.worktree(&task).join(".fake-prompt-5")).unwrap();
    rig.stop().await.unwrap();
    assert!(prompt.contains("pass 4 of 4"), "{prompt}");
    assert!(prompt.contains("An investigator worked out this resolution on the tip of your pull request. Apply it, check it, then go on from step 4:"), "{prompt}");
    assert!(prompt.contains("git merge origin/main — résolution"), "{prompt}");
}

#[tokio::test]
async fn the_human_is_asked_when_the_pass_after_the_answer_fails() {
    let (mut rig, task) = spent_conflict_passes(
        "conflict-investigator:inv-done:Resolve the conflict",
        "work,merge-base,merge-base,merge-base,fail-tool,ask-here",
        true,
    )
    .await;
    assert_investigation_filed(&rig);
    rig.status_is(&task, agent_core::status::TaskStatus::Questions).await;
    let prompt = std::fs::read_to_string(rig.worktree(&task).join(".fake-prompt-6")).unwrap();
    rig.stop().await.unwrap();
    assert!(
        prompt.contains("Merge conflict on pull request #1 needs the human"),
        "{prompt}"
    );
    assert_eq!(conflict_requests(&rig)["investigations"].as_object().unwrap().len(), 1);
}

#[tokio::test]
async fn the_human_is_asked_when_the_conflict_request_fails() {
    let (mut rig, task) = spent_conflict_passes(
        "conflict-investigator:inv-fail",
        "work,merge-base,merge-base,merge-base,ask-here",
        true,
    )
    .await;
    assert_investigation_filed(&rig);
    rig.status_is(&task, agent_core::status::TaskStatus::Questions).await;
    rig.stop().await.unwrap();
    assert_eq!(conflict_requests(&rig)["investigations"]["1"]["status"], "failed");
    let prompt = std::fs::read_to_string(rig.worktree(&task).join(".fake-prompt-5")).unwrap();
    assert!(
        prompt.contains("Merge conflict on pull request #1 needs the human"),
        "{prompt}"
    );
}

#[tokio::test]
async fn investigator_pass_false_asks_the_human_after_three_passes_as_before() {
    let (mut rig, task) = spent_conflict_passes(
        "conflict-investigator:hold",
        "work,merge-base,merge-base,merge-base,ask-here",
        false,
    )
    .await;
    rig.status_is(&task, agent_core::status::TaskStatus::Questions).await;
    rig.stop().await.unwrap();
    let requests = conflict_requests(&rig);
    assert!(
        requests["investigations"]
            .as_object()
            .is_none_or(serde_json::Map::is_empty)
    );
    assert_eq!(rig.runs(&task), 5);
}

#[tokio::test]
async fn a_conflict_request_is_filed_once_per_item_and_head() {
    let (mut rig, _) = spent_conflict_passes(
        "conflict-investigator:hold",
        "work,merge-base,merge-base,merge-base,hold,ask-here",
        true,
    )
    .await;
    assert_investigation_filed(&rig);
    let before = conflict_requests(&rig);
    assert_eq!(before["investigations"].as_object().unwrap().len(), 1);
    rig.stop().await.unwrap();
    rig.start().await;
    rig.state_where("restored investigation", |state| {
        serde_json::to_value(state)
            .unwrap()
            .to_string()
            .contains("conflict_investigation")
    })
    .await;
    rig.stop().await.unwrap();
    assert_eq!(conflict_requests(&rig)["investigations"].as_object().unwrap().len(), 1);
}

#[tokio::test]
async fn a_new_head_may_get_a_new_conflict_request() {
    let (mut rig, task) = spent_conflict_passes(
        "conflict-investigator:hold",
        "work,merge-base,merge-base,merge-base,hold",
        true,
    )
    .await;
    assert_investigation_filed(&rig);
    let worktree = rig.worktree(&task);
    std::fs::write(worktree.join("result.txt"), "another head\n").unwrap();
    git(&worktree, &["add", "result.txt"]);
    git(&worktree, &["commit", "-q", "-m", "Publish another head"]);
    git(&worktree, &["push", "-q", "origin", "HEAD"]);
    rig.state_where("a request for the new head", |state| {
        serde_json::to_value(state.teams()[0].item("1.1".parse().unwrap()).unwrap()).unwrap()["conflict_investigation"]
            ["id"]
            == 2
    })
    .await;
    rig.stop().await.unwrap();
    assert_eq!(conflict_requests(&rig)["investigations"].as_object().unwrap().len(), 2);
}

#[tokio::test]
async fn merge_conflict_investigated_is_logged_with_item_and_id() {
    use agent_core::config::Secrets;
    use agent_core::log::LogLevel;
    use agent_runtime::history::{HistoryLayer, Reader};
    use std::io::Write as _;
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,merge-base,ask-here"),
            investigator("inv-r1", "conflict-investigator:hold"),
            investigator("inv-r2", "conflict-investigator:hold"),
        ],
        5,
        true,
    )
    .await;
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(rig.home.config_file())
            .unwrap(),
        "[merge]\nconflict_passes = 1"
    )
    .unwrap();
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    let layer = HistoryLayer::deferred(LogLevel::Info, rig.history_test_clock(), Secrets::default());
    rig.start_with_log_layer(layer).await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    rig.wait_for("the conflict pass", || rig.runs(&task) >= 2).await;
    move_base(&rig);
    std::fs::write(rig.worktree(&task).join(".fake-base-ready"), "").unwrap();
    rig.state_where("investigation or human", |state| {
        let Some(item) = state.teams().first().and_then(|team| team.item("1.1".parse().unwrap())) else {
            return false;
        };
        serde_json::to_value(item)
            .unwrap()
            .get("conflict_investigation")
            .is_some()
            || matches!(
                item.merge_conflict(),
                Some(agent_core::state::MergeConflict::AskingHuman { .. })
            )
    })
    .await;
    rig.stop().await.unwrap();
    let logs = Reader::open(&rig.home.history_file()).unwrap().daemon_logs().unwrap();
    let entries: Vec<_> = logs
        .iter()
        .filter(|row| row.event().as_str() == "merge_conflict_investigated")
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].fields().get("item"), Some(&serde_json::json!("1.1")));
    assert_eq!(entries[0].fields().get("id"), Some(&serde_json::json!(1)));
}

#[tokio::test]
async fn the_human_is_asked_when_the_extra_published_tip_still_conflicts() {
    let (mut rig, task) = spent_conflict_passes(
        "conflict-investigator:inv-done:Resolve the conflict",
        "work,merge-base,merge-base,merge-base,merge-base,ask-here",
        true,
    )
    .await;
    assert_investigation_filed(&rig);
    rig.wait_for("the extra conflict run", || rig.runs(&task) >= 5).await;
    move_base(&rig);
    std::fs::write(rig.worktree(&task).join(".fake-base-ready"), "").unwrap();
    let state = rig
        .state_where("human fallback or an unwanted second investigation", |state| {
            let item = state.teams()[0].item("1.1".parse().unwrap()).unwrap();
            matches!(
                item.merge_conflict(),
                Some(agent_core::state::MergeConflict::AskingHuman { .. })
            ) || serde_json::to_value(item).unwrap()["conflict_investigation"]["id"] == 2
        })
        .await;
    rig.stop().await.unwrap();
    let item = state.teams()[0].item("1.1".parse().unwrap()).unwrap();
    assert!(
        matches!(
            item.merge_conflict(),
            Some(agent_core::state::MergeConflict::AskingHuman { .. })
        ),
        "{item:?}"
    );
    assert_eq!(conflict_requests(&rig)["investigations"].as_object().unwrap().len(), 1);
}

#[tokio::test]
async fn a_verified_conflict_change_gets_an_investigation_and_the_extra_pass() {
    use std::io::Write as _;
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,merge-base-other,hold,ask-here"),
            investigator("inv-r1", "conflict-investigator:inv-done:Resolve the verified conflict"),
            investigator("inv-r2", "conflict-investigator:inv-done:Resolve the verified conflict"),
        ],
        5,
        true,
    )
    .await;
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(rig.home.config_file())
            .unwrap(),
        "[merge]\nconflict_passes = 1"
    )
    .unwrap();
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    rig.start().await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    rig.wait_for("the first conflict run", || rig.runs(&task) >= 2).await;
    move_base(&rig);
    std::fs::write(rig.worktree(&task).join(".fake-base-ready"), "").unwrap();
    rig.state_where("investigation or repeated refused merges after verification", |state| {
        let Some(item) = state.teams().first().and_then(|team| team.item("1.1".parse().unwrap())) else {
            return false;
        };
        serde_json::to_value(item)
            .unwrap()
            .get("conflict_investigation")
            .is_some()
            || rig.github.calls("PUT", &format!("/repos/{SLUG}/pulls/1/merge")).len() >= 3
    })
    .await;
    assert_investigation_filed(&rig);
    rig.wait_for("the extra prompt after verification", || {
        rig.worktree(&task).join(".fake-prompt-3").is_file()
    })
    .await;
    let prompt = std::fs::read_to_string(rig.worktree(&task).join(".fake-prompt-3")).unwrap();
    rig.stop().await.unwrap();
    assert!(prompt.contains("pass 2 of 2"), "{prompt}");
    assert!(prompt.contains("Resolve the verified conflict"), "{prompt}");
}

/// Collects the new event from its real daemon cause for the exhaustive log test.
pub(super) async fn record_merge_conflict_investigated(
    observed: &mut std::collections::BTreeSet<agent_core::log::LogEvent>,
) {
    use agent_core::config::Secrets;
    use agent_core::log::LogLevel;
    use agent_runtime::history::{HistoryLayer, Reader};
    use std::io::Write as _;
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,merge-base,ask-here"),
            investigator("inv-r1", "conflict-investigator:hold"),
            investigator("inv-r2", "conflict-investigator:hold"),
        ],
        5,
        true,
    )
    .await;
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(rig.home.config_file())
            .unwrap(),
        "[merge]\nconflict_passes = 1"
    )
    .unwrap();
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    let layer = HistoryLayer::deferred(LogLevel::Info, rig.history_test_clock(), Secrets::default());
    rig.start_with_log_layer(layer).await;
    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    rig.wait_for("the conflict pass", || rig.runs(&task) >= 2).await;
    move_base(&rig);
    std::fs::write(rig.worktree(&task).join(".fake-base-ready"), "").unwrap();
    let state = rig
        .state_where("investigation or human", |state| {
            let Some(item) = state.teams().first().and_then(|team| team.item("1.1".parse().unwrap())) else {
                return false;
            };
            serde_json::to_value(item)
                .unwrap()
                .get("conflict_investigation")
                .is_some()
                || matches!(
                    item.merge_conflict(),
                    Some(agent_core::state::MergeConflict::AskingHuman { .. })
                )
        })
        .await;
    rig.stop().await.unwrap();
    let logs = Reader::open(&rig.home.history_file()).unwrap().daemon_logs().unwrap();
    let entries: Vec<_> = logs
        .iter()
        .filter(|row| row.event().as_str() == "merge_conflict_investigated")
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].fields().get("item"), Some(&serde_json::json!("1.1")));
    assert_eq!(entries[0].fields().get("id"), Some(&serde_json::json!(1)));
    let row = entries[0];
    assert_eq!(row.level(), agent_core::log::LogLevel::Info);
    assert_eq!(row.team(), Some(state.teams()[0].id()));
    assert!(!row.rounds().is_empty());
    assert_eq!(row.task(), None);
    assert_eq!(row.run(), None);
    observed.insert(agent_core::log::LogEvent::MergeConflictInvestigated);
}
