//! The autopilot in a daemon: a team's branch is published as a pull request, investigators from outside the team
//! review it, and what they say leads to a merge, a fix pass or a question — against a fake GitHub, a bare `origin` and
//! stand-ins for the model CLIs.

use agent_core::protocol::{Client, Command, Reply, StatusRequest};
use agent_core::state::{ItemPhase, MergeMode, ReviewOutcome};
use agent_core::status::TaskStatus;

use super::fake_github::SLUG;
use super::rig::{ONE, Rig, git, investigator, normal};

/// The reviewers that never give a verdict: they end their turn and wait.
const SILENT: &str = "sleep";

/// Waits until the daemon has looked at its teams since now: its answer to a request comes between two turns of its
/// loop, and each turn starts with a look at the teams that something happened to.
async fn looked_again(rig: &Rig) {
    for _ in 0..3 {
        let reply = crate::client::Connection::connect(&rig.home.socket_file())
            .await
            .unwrap()
            .request(Client::Cli, Command::Status(StatusRequest { target: None }))
            .await
            .unwrap();
        assert!(matches!(reply, Reply::Teams { .. }), "{reply:?}");
    }
}

/// A rig with a solo agent that does its item and two reviewers who say nothing.
async fn solo_rig() -> Rig {
    Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", SILENT),
            investigator("inv-r2", SILENT),
        ],
        5,
        false,
    )
    .await
}

// ---- publishing ----------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_solo_agent_that_did_its_item_with_a_green_gate_has_its_branch_pushed_and_a_pull_request_opened() {
    let mut rig = solo_rig().await;
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "in review", |phase| matches!(phase, ItemPhase::InReview { .. }))
        .await;
    rig.stop().await.unwrap();

    let ItemPhase::InReview { pr, .. } = state.teams()[0].item("1.1".parse().unwrap()).unwrap().phase().clone() else {
        panic!("the item is not in review");
    };
    let branch = format!("agents/claude-1/{}-1.1", tasks[0]);
    assert_eq!((pr.number.get().get(), pr.branch.as_str()), (1, branch.as_str()));
    assert_eq!(rig.remote_tip(&branch), git(&rig.repo, &["rev-parse", &branch]));
    let created = rig.github.calls("POST", &format!("/repos/{SLUG}/pulls"));
    assert_eq!(created.len(), 1);
    let sent = created[0].json();
    assert_eq!(sent["head"], branch);
    assert_eq!(sent["base"], "main");
    assert_eq!(sent["draft"], true);
    assert_eq!(sent["title"], "Item 1.1: The first item");
    let body = sent["body"].as_str().unwrap();
    assert!(body.contains("Do the first thing in the repository."), "{body}");
    assert!(body.contains("- the first thing works"), "{body}");
    assert!(body.contains("passed at"), "{body}");
    assert_eq!(rig.github.calls("POST", "/graphql").len(), 1);
    assert!(!rig.github.pulls()[0].draft);
}

#[tokio::test]
async fn the_pull_request_of_an_item_is_asked_for_two_reviews_of_two_investigators_outside_the_team() {
    let mut rig = solo_rig().await;
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where(
            "1.1",
            "asked for reviews",
            |phase| matches!(phase, ItemPhase::InReview { reviews, .. } if reviews.len() == 2),
        )
        .await;
    rig.stop().await.unwrap();

    let found = state.team(&team).unwrap();
    let ItemPhase::InReview { reviews, .. } = found.item("1.1".parse().unwrap()).unwrap().phase().clone() else {
        panic!("the item is not in review");
    };
    let mut names: Vec<&str> = reviews.iter().map(|review| review.reviewer.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["inv-r1", "inv-r2"]);
    assert!(reviews.iter().all(|review| review.outcome == ReviewOutcome::Pending));
    assert_eq!(found.reviewers().len(), 2);
    for review in &reviews {
        let task = found.find(&review.task).map(|(_, member)| member.task().clone());
        assert_eq!(task.as_ref(), Some(&review.task));
        let worktree = rig.worktree(&review.task);
        assert_eq!(
            git(&worktree, &["rev-parse", "HEAD"]),
            rig.remote_tip(&format!("agents/claude-1/{}-1.1", found.crew().roster()[0].1.task()))
        );
    }
}

#[tokio::test]
async fn the_prompt_of_a_reviewer_holds_the_pull_request_and_the_items_title_description_and_expected_result() {
    let mut rig = solo_rig().await;
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where(
            "1.1",
            "asked for reviews",
            |phase| matches!(phase, ItemPhase::InReview { reviews, .. } if reviews.len() == 2),
        )
        .await;
    let reviewer = state.team(&team).unwrap().reviewers()[0].task().clone();
    let prompt = rig.prompt(&reviewer, 1).await;
    rig.stop().await.unwrap();

    assert!(prompt.contains("pull request #1"), "{prompt}");
    assert!(prompt.contains("1.1 — The first item"), "{prompt}");
    assert!(prompt.contains("Do the first thing in the repository."), "{prompt}");
    assert!(prompt.contains("- the first thing works"), "{prompt}");
    assert!(prompt.contains("- run the first thing"), "{prompt}");
    assert!(prompt.contains("VERDICT: APPROVE"), "{prompt}");
    assert!(prompt.contains("Your role: reviewer"), "{prompt}");
}

#[tokio::test]
async fn nothing_is_published_while_the_gate_has_not_passed_at_the_tip_of_the_branch() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "done"),
            investigator("inv-r1", SILENT),
            investigator("inv-r2", SILENT),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    looked_again(&rig).await;
    rig.stop().await.unwrap();

    assert_eq!(rig.github.requests(), []);
    assert_eq!(rig.state().teams()[0].reviewers().len(), 0);
}

// ---- reviews and the merge -----------------------------------------------------------------------------------

use agent_core::state::Completion;

use super::rig::CHAIN;

/// The reviewers that approve whatever they are asked to review.
const APPROVING: &str = "done:VERDICT: APPROVE\\nSUMMARY: fine";

/// A rig with a solo agent that does two items one after the other and two reviewers that approve both.
async fn approving_rig() -> Rig {
    Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await
}

/// The phase of item `id` of the only team.
fn phase_of(state: &agent_core::state::State, id: &str) -> ItemPhase {
    state.teams()[0].item(id.parse().unwrap()).unwrap().phase().clone()
}

#[tokio::test]
async fn two_approving_reviews_merge_the_pull_request_through_the_api_at_the_head_they_reviewed() {
    let mut rig = approving_rig().await;
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let ItemPhase::Done { completion } = phase_of(&state, "1.1") else {
        panic!("item 1.1 is not done");
    };
    let Completion::Merged { pr, .. } = completion else {
        panic!("item 1.1 was not merged");
    };
    assert_eq!(pr.number.get().get(), 1);
    let merges = rig.github.calls("PUT", &format!("/repos/{SLUG}/pulls/1/merge"));
    assert_eq!(merges.len(), 1);
    let sent = merges[0].json();
    assert_eq!(sent["merge_method"], "merge");
    let branch = format!("agents/claude-1/{}-1.1", tasks[0]);
    assert_eq!(sent["sha"], git(&rig.origin, &["rev-parse", &branch]));
    let reviewers: Vec<_> = state
        .team(&team)
        .unwrap()
        .reviewers()
        .iter()
        .map(|member| member.task().clone())
        .collect();
    assert_eq!(reviewers.len(), 2);
    assert_eq!(rig.github.reviews(1).len(), 2);
}

#[tokio::test]
async fn the_review_of_a_reviewer_is_posted_on_the_pull_request_as_a_comment_with_its_report_and_kept() {
    let mut rig = approving_rig().await;
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let branch = format!("agents/claude-1/{}-1.1", tasks[0]);
    let head = rig.remote_tip("main");
    assert_ne!(head, "");
    let posted = rig.github.reviews(1);
    assert_eq!(posted.len(), 2);
    for review in &posted {
        assert_eq!(review["event"], "COMMENT");
        assert_eq!(review["commit_id"], git(&rig.origin, &["rev-parse", &branch]));
        let body = review["body"].as_str().unwrap();
        assert!(body.contains("VERDICT: APPROVE"), "{body}");
        assert!(body.contains("SUMMARY: fine"), "{body}");
        assert!(!body.contains("STATUS: DONE"), "{body}");
        assert!(body.contains("inv-r"), "{body}");
    }
    for member in state.team(&team).unwrap().reviewers() {
        let kept = std::fs::read_to_string(rig.home.review_report_file(
            member.task(),
            agent_core::state::PrNumber::new(std::num::NonZeroU32::MIN),
            0,
        ))
        .unwrap();
        assert!(kept.starts_with("VERDICT: APPROVE"), "{kept}");
    }
}

#[tokio::test]
async fn the_next_item_starts_from_what_was_merged_and_is_published_in_its_turn() {
    let mut rig = approving_rig().await;
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.2", "in review", |phase| matches!(phase, ItemPhase::InReview { .. }))
        .await;
    rig.stop().await.unwrap();

    let ItemPhase::InReview { pr, .. } = phase_of(&state, "1.2") else {
        panic!("item 1.2 is not in review");
    };
    let second = format!("agents/claude-1/{}-1.2", tasks[0]);
    assert_eq!((pr.number.get().get(), pr.branch.as_str()), (2, second.as_str()));
    // The first pull request was merged into the remote's main; the second item was started from it.
    let merged = rig.remote_tip("main");
    let started_from_it = std::process::Command::new("git")
        .args(["merge-base", "--is-ancestor", &merged, &second])
        .current_dir(&rig.repo)
        .status()
        .unwrap();
    assert!(started_from_it.success(), "{merged} is not part of {second}");
}

#[tokio::test]
async fn a_reviewer_that_gave_its_review_waits_and_reviews_the_next_pull_request_in_the_same_task() {
    let mut rig = approving_rig().await;
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where(
            "1.2",
            "asked for reviews",
            |phase| matches!(phase, ItemPhase::InReview { reviews, .. } if reviews.len() == 2),
        )
        .await;
    rig.stop().await.unwrap();

    let found = state.team(&team).unwrap();
    assert_eq!(found.reviewers().len(), 2);
    let ItemPhase::InReview { reviews, .. } = phase_of(&state, "1.2") else {
        panic!("item 1.2 is not in review");
    };
    let mut asked: Vec<_> = reviews.iter().map(|review| review.task.clone()).collect();
    let mut known: Vec<_> = found.reviewers().iter().map(|member| member.task().clone()).collect();
    asked.sort();
    known.sort();
    assert_eq!(asked, known);
}

#[tokio::test]
async fn when_every_item_is_merged_the_team_ends_its_reviewers_are_closed_and_the_human_is_told() {
    let mut rig = approving_rig().await;
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::Autopilot, None).await;
    let notice = rig.notification_with("is finished").await;
    let state = rig
        .state_where("every task to be done", |state| {
            state.team(&team).is_some_and(|found| {
                found
                    .reviewers()
                    .iter()
                    .all(|member| member.status() == TaskStatus::Done)
                    && found
                        .crew()
                        .roster()
                        .iter()
                        .all(|(_, member)| member.status() == TaskStatus::Done)
            })
        })
        .await;
    rig.stop().await.unwrap();

    assert!(notice.starts_with(&format!("Team {team} is finished\n")), "{notice}");
    let found = state.team(&team).unwrap();
    for id in ["1.1", "1.2"] {
        assert!(
            matches!(
                found.item(id.parse().unwrap()).unwrap().phase(),
                ItemPhase::Done {
                    completion: Completion::Merged { .. }
                }
            ),
            "item {id}"
        );
    }
    assert_eq!(
        rig.github.calls("PUT", &format!("/repos/{SLUG}/pulls/2/merge")).len(),
        1
    );
    // Both items are in the remote's main.
    let second = format!("agents/claude-1/{}-1.2", tasks[0]);
    assert_eq!(rig.remote_tip("main"), git(&rig.repo, &["rev-parse", &second]));
}

#[tokio::test]
async fn a_report_without_a_verdict_is_asked_for_again_and_then_counts() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator(
                "inv-r1",
                "done:I read it and it looks fine.,done:VERDICT: APPROVE\\nSUMMARY: fine",
            ),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let silent = state
        .team(&team)
        .unwrap()
        .reviewers()
        .iter()
        .find(|member| member.agent().name() == "inv-r1")
        .map(|member| member.task().clone())
        .unwrap();
    assert_eq!(rig.runs(&silent), 2);
    let second = rig.prompt(&silent, 2).await;
    assert!(second.contains("Your report has no verdict"), "{second}");
}

/// A reviewer that always asks for changes.
const CHANGES: &str = "done:VERDICT: REQUEST_CHANGES\\nBLOCKERS\\n1. [TDD-2] src/lib.rs:7 — the case is untested";

#[tokio::test]
async fn a_request_for_changes_sends_the_item_back_to_its_lead_for_a_fix_pass_that_records_the_reviews() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", CHANGES),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .state_where("a fix pass", |state| {
            state
                .team(&team)
                .and_then(|found| found.item("1.1".parse().unwrap()))
                .is_some_and(|progress| !progress.fix_passes().is_empty())
        })
        .await;
    rig.stop().await.unwrap();

    let progress = state.team(&team).unwrap().item("1.1".parse().unwrap()).unwrap();
    let pass = &progress.fix_passes()[0];
    assert!(
        pass.requested_by.iter().any(|review| matches!(
            review.outcome,
            ReviewOutcome::Submitted {
                verdict: agent_core::status::ReviewVerdict::RequestChanges,
                ..
            }
        )),
        "{pass:?}"
    );
    let branch = format!("agents/claude-1/{}-1.1", tasks[0]);
    // Nothing is merged while a reviewer asks for changes.
    assert_eq!(rig.github.calls("PUT", &format!("/repos/{SLUG}/pulls/1/merge")), []);
    assert_ne!(rig.remote_tip("main"), rig.remote_tip(&branch));
}

// ---- fix passes ----------------------------------------------------------------------------------------------

/// A reviewer that asks for changes the first time and approves afterwards.
const CHANGES_ONCE: &str = "done:VERDICT: REQUEST_CHANGES\\nBLOCKERS\\n1. [TDD-2] src/lib.rs:7 — the case is untested,done:VERDICT: APPROVE\\nSUMMARY: fixed";

/// A rig with a solo agent that works twice on its item (the second time is the fix pass), a reviewer that asks for
/// changes once and one that approves.
async fn fixing_rig(fix_passes: u32) -> Rig {
    Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        fix_passes,
        false,
    )
    .await
}

#[tokio::test]
async fn the_lead_is_given_the_fix_pass_with_the_findings_of_the_reviewers() {
    let mut rig = fixing_rig(5).await;
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let prompt = rig.prompt(&tasks[0], 2).await;
    rig.stop().await.unwrap();

    assert!(prompt.contains("THE REVIEWERS ASKED FOR CHANGES"), "{prompt}");
    assert!(prompt.contains("pull request #1, fix pass 1 of 5"), "{prompt}");
    assert!(
        prompt.contains("1. [TDD-2] src/lib.rs:7 — the case is untested"),
        "{prompt}"
    );
    assert!(prompt.contains("inv-r1"), "{prompt}");
    assert!(
        prompt.contains(&format!("agents/claude-1/{}-1.1", tasks[0])),
        "{prompt}"
    );
    assert!(prompt.contains("agent gate"), "{prompt}");
}

#[tokio::test]
async fn a_fixed_branch_updates_the_pull_request_and_is_reviewed_again_until_it_is_merged() {
    let mut rig = fixing_rig(5).await;
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    // One pull request, updated: the second push of the branch is a comment on it.
    assert_eq!(rig.github.calls("POST", &format!("/repos/{SLUG}/pulls")).len(), 1);
    let comments = rig.github.comments(1);
    assert_eq!(comments.len(), 1, "{comments:?}");
    assert!(comments[0].contains("The branch was updated to"), "{}", comments[0]);
    let branch = format!("agents/claude-1/{}-1.1", tasks[0]);
    assert!(comments[0].contains(&rig.remote_tip(&branch)), "{}", comments[0]);
    // Four reviews were posted: two on the first head, two on the fixed one.
    assert_eq!(rig.github.reviews(1).len(), 4);
    let merges = rig.github.calls("PUT", &format!("/repos/{SLUG}/pulls/1/merge"));
    assert_eq!(merges.len(), 1);
    assert_eq!(merges[0].json()["sha"], rig.remote_tip(&branch));
    let progress = state.team(&team).unwrap().item("1.1".parse().unwrap()).unwrap();
    assert_eq!(progress.fix_passes().len(), 1);
    // The reports of the second round are kept apart from those of the first.
    for member in state.team(&team).unwrap().reviewers() {
        let number = agent_core::state::PrNumber::new(std::num::NonZeroU32::MIN);
        assert!(rig.home.review_report_file(member.task(), number, 0).exists());
        assert!(rig.home.review_report_file(member.task(), number, 1).exists());
    }
}

#[tokio::test]
async fn a_fix_pass_whose_gate_has_not_passed_does_not_update_the_pull_request() {
    let mut rig = Rig::new(
        &[
            // The second run, the fix pass, commits nothing and records no gate.
            normal("claude-1", "work,done"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.prompt(&tasks[0], 2).await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    looked_again(&rig).await;
    rig.stop().await.unwrap();

    assert_eq!(rig.github.comments(1), Vec::<String>::new());
    assert_eq!(rig.github.reviews(1).len(), 2);
    assert!(matches!(phase_of(&rig.state(), "1.1"), ItemPhase::Fixing { .. }));
}

// ---- the human ------------------------------------------------------------------------------------------------

use agent_core::protocol::{AnswerInput, AnswerList, AnswerRequest};
use agent_core::questions::Verdict;

/// Answers round `round` of `task` with `ok`, as the human does.
async fn accept(rig: &Rig, task: &agent_core::ids::TaskId, round: u32) {
    let answers = AnswerList::try_from(vec![AnswerInput {
        question: std::num::NonZeroU32::MIN,
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
                round: std::num::NonZeroU32::new(round).unwrap(),
                answers,
            }),
        )
        .await
        .unwrap();
    assert!(matches!(reply, Reply::Applied { .. }), "{reply:?}");
}

/// A rig in which the only reviewer that matters always asks for changes and one pass is allowed, with a solo agent that
/// works on the item, fixes it once, and then does what `third` says.
async fn exhausting_rig(solo: &'static str) -> Rig {
    Rig::new(
        &[
            normal("claude-1", solo),
            investigator("inv-r1", CHANGES),
            investigator("inv-r2", APPROVING),
        ],
        1,
        false,
    )
    .await
}

#[tokio::test]
async fn when_the_fix_passes_are_used_up_the_lead_is_asked_to_put_the_question_to_the_human() {
    let mut rig = exhausting_rig("work,work,ask").await;
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig.status_is(&tasks[0], TaskStatus::Questions).await;
    let prompt = rig.prompt(&tasks[0], 3).await;
    let notice = rig.notification_with("has questions").await;
    rig.stop().await.unwrap();

    assert!(
        prompt.contains("THE REVIEWERS STILL ASK FOR CHANGES (pull request #1)"),
        "{prompt}"
    );
    assert!(prompt.contains("The team used its 1 fix passes"), "{prompt}");
    assert!(
        prompt.contains("1. [TDD-2] src/lib.rs:7 — the case is untested"),
        "{prompt}"
    );
    assert!(prompt.contains("questions-1.md"), "{prompt}");
    let found = &state.teams()[0];
    assert!(matches!(
        found.item("1.1".parse().unwrap()).unwrap().phase(),
        ItemPhase::InReview { .. }
    ));
    assert_eq!(found.item("1.1".parse().unwrap()).unwrap().fix_passes().len(), 1);
    assert!(found.crew().roster()[0].1.open_question().is_some());
    assert!(notice.contains("questions-1.md"), "{notice}");
    assert_eq!(rig.github.calls("PUT", &format!("/repos/{SLUG}/pulls/1/merge")), []);
}

#[tokio::test]
async fn the_humans_answer_starts_one_more_fix_pass_with_the_findings_and_the_answer() {
    let mut rig = exhausting_rig("work,work,ask,work").await;
    rig.start().await;

    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.status_is(&tasks[0], TaskStatus::Questions).await;
    accept(&rig, &tasks[0], 1).await;
    let prompt = rig.prompt(&tasks[0], 4).await;
    let state = rig
        .state_where("a second fix pass", |state| {
            state
                .team(&team)
                .and_then(|found| found.item("1.1".parse().unwrap()))
                .is_some_and(|progress| progress.fix_passes().len() >= 2)
        })
        .await;
    rig.stop().await.unwrap();

    assert!(prompt.contains("THE REVIEWERS ASKED FOR CHANGES"), "{prompt}");
    assert!(prompt.contains("fix pass 2 of 1"), "{prompt}");
    assert!(
        prompt.contains("1. [TDD-2] src/lib.rs:7 — the case is untested"),
        "{prompt}"
    );
    assert!(
        prompt.contains("What should the team do about the pull request?"),
        "{prompt}"
    );
    let progress = state.team(&team).unwrap().item("1.1".parse().unwrap()).unwrap();
    assert_eq!(progress.fix_passes().len(), 2);
}

#[tokio::test]
async fn a_lead_that_ends_without_the_question_leaves_the_human_told_directly_once() {
    let mut rig = exhausting_rig("work,work,done").await;
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.prompt(&tasks[0], 3).await;
    let notice = rig.notification_with("needs your decision").await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    looked_again(&rig).await;
    rig.stop().await.unwrap();

    assert!(notice.contains("pull request #1"), "{notice}");
    let told: Vec<_> = rig
        .notifications()
        .into_iter()
        .filter(|text| text.contains("needs your decision"))
        .collect();
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(rig.runs(&tasks[0]), 3);
}

// ---- --no-merge and a merge that does not happen --------------------------------------------------------------

use super::fake_github::MergeAnswer;

#[tokio::test]
async fn with_no_merge_nothing_is_merged_and_the_human_is_told_that_the_pull_request_is_approved() {
    let mut rig = approving_rig().await;
    rig.start().await;

    rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let notice = rig.notification_with("is approved").await;
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    looked_again(&rig).await;
    rig.stop().await.unwrap();

    assert!(notice.starts_with("Pull request 1 of "), "{notice}");
    assert!(notice.contains("pull/1"), "{notice}");
    assert!(notice.contains("--no-merge"), "{notice}");
    assert_eq!(rig.github.calls("PUT", &format!("/repos/{SLUG}/pulls/1/merge")), []);
    assert!(matches!(phase_of(&state, "1.1"), ItemPhase::Approved { .. }));
    let told: Vec<_> = rig
        .notifications()
        .into_iter()
        .filter(|text| text.contains("is approved"))
        .collect();
    assert_eq!(told.len(), 1, "{told:?}");
}

#[tokio::test]
async fn with_no_merge_the_next_item_starts_when_the_first_is_approved() {
    let mut rig = approving_rig().await;
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], CHAIN, MergeMode::NoMerge, None).await;
    let state = rig
        .item_where("1.2", "in review", |phase| matches!(phase, ItemPhase::InReview { .. }))
        .await;
    rig.stop().await.unwrap();

    assert!(matches!(phase_of(&state, "1.1"), ItemPhase::Approved { .. }));
    let ItemPhase::InReview { pr, .. } = phase_of(&state, "1.2") else {
        panic!("item 1.2 is not in review");
    };
    assert_eq!(pr.branch.as_str(), format!("agents/claude-1/{}-1.2", tasks[0]));
    assert_eq!(rig.github.calls("PUT", &format!("/repos/{SLUG}/pulls/1/merge")), []);
}

#[tokio::test]
async fn the_item_is_done_and_the_team_ends_when_the_human_merges_the_approved_pull_request() {
    let mut rig = approving_rig().await;
    rig.start().await;

    rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    rig.notification_with("is approved").await;
    rig.github.merged_by_a_human(1);
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    let finished = rig.notification_with("is finished").await;
    rig.stop().await.unwrap();

    let ItemPhase::Done { completion } = phase_of(&state, "1.1") else {
        panic!("item 1.1 is not done");
    };
    assert!(matches!(completion, Completion::Merged { .. }));
    assert!(finished.contains("is finished"), "{finished}");
}

#[tokio::test]
async fn a_merge_that_github_refuses_is_told_to_the_human_once_and_is_not_tried_again() {
    let mut rig = approving_rig().await;
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    rig.start().await;

    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let notice = rig.notification_with("could not be merged").await;
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    looked_again(&rig).await;
    rig.stop().await.unwrap();

    assert!(notice.starts_with("Pull request 1 of "), "{notice}");
    assert!(notice.contains("Pull Request is not mergeable"), "{notice}");
    assert!(matches!(phase_of(&state, "1.1"), ItemPhase::Approved { .. }));
    assert_eq!(
        rig.github.calls("PUT", &format!("/repos/{SLUG}/pulls/1/merge")).len(),
        1
    );
    let told: Vec<_> = rig
        .notifications()
        .into_iter()
        .filter(|text| text.contains("could not be merged"))
        .collect();
    assert_eq!(told.len(), 1, "{told:?}");
}

#[tokio::test]
async fn an_approved_pull_request_that_the_human_merged_after_a_refusal_completes_the_item() {
    let mut rig = approving_rig().await;
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    rig.start().await;

    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.notification_with("could not be merged").await;
    rig.github.merged_by_a_human(1);
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    assert_eq!(
        rig.github.calls("PUT", &format!("/repos/{SLUG}/pulls/1/merge")).len(),
        1
    );
}

#[tokio::test]
async fn a_pull_request_the_human_closed_without_merging_is_told_once_and_the_item_waits() {
    let mut rig = approving_rig().await;
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    rig.start().await;

    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.notification_with("could not be merged").await;
    rig.github.closed_by_a_human(1);
    let notice = rig.notification_with("was closed").await;
    looked_again(&rig).await;
    rig.stop().await.unwrap();

    assert!(
        notice.contains("pull request #1") || notice.contains("Pull request 1"),
        "{notice}"
    );
    assert!(matches!(phase_of(&rig.state(), "1.1"), ItemPhase::Approved { .. }));
}

// ---- a team with a coordinator ---------------------------------------------------------------------------------

/// A rig with a coordinator that sleeps until its member is done and then integrates (and, when it is asked for a fix
/// pass, works on its own branch), a member that does its item, and two reviewers.
async fn team_rig(reviewer: &'static str) -> Rig {
    Rig::new(
        &[
            investigator("inv-lead", "sleep,integrate,work"),
            normal("claude-1", "work"),
            investigator("inv-r1", reviewer),
            investigator("inv-r2", APPROVING),
        ],
        5,
        true,
    )
    .await
}

#[tokio::test]
async fn the_branch_of_the_coordinator_that_integrated_the_members_work_is_published_reviewed_and_merged() {
    let mut rig = team_rig(APPROVING).await;
    rig.start().await;

    let (team, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.notification_with("is finished").await;
    rig.stop().await.unwrap();

    let ItemPhase::Done {
        completion: Completion::Merged { pr, .. },
    } = phase_of(&state, "1.1")
    else {
        panic!("item 1.1 was not merged");
    };
    let coordinator_branch = format!("agents/inv-lead/{}", tasks[0]);
    assert_eq!(pr.branch.as_str(), coordinator_branch);
    let created = rig.github.calls("POST", &format!("/repos/{SLUG}/pulls"));
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].json()["head"], coordinator_branch);
    // What the member did is in the main branch of the remote, through the coordinator's merge.
    let member_branch = format!("agents/claude-1/{}-1.1", tasks[1]);
    let part = std::process::Command::new("git")
        .args(["merge-base", "--is-ancestor", &member_branch, &rig.remote_tip("main")])
        .current_dir(&rig.repo)
        .status()
        .unwrap();
    assert!(part.success());
    let reviewers: Vec<_> = state
        .team(&team)
        .unwrap()
        .reviewers()
        .iter()
        .map(|member| member.agent().name().to_owned())
        .collect();
    assert_eq!(reviewers.len(), 2);
    assert!(reviewers.iter().all(|name| name.starts_with("inv-r")), "{reviewers:?}");
}

#[tokio::test]
async fn a_fix_pass_of_a_team_goes_to_its_coordinator_which_has_the_integrated_branch() {
    let mut rig = team_rig(CHANGES_ONCE).await;
    rig.start().await;

    let (_, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, Some("inv-lead"))
        .await;
    let prompt = rig.prompt(&tasks[0], 3).await;
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    assert!(prompt.contains("THE REVIEWERS ASKED FOR CHANGES"), "{prompt}");
    assert!(prompt.contains(&format!("agents/inv-lead/{}", tasks[0])), "{prompt}");
    assert!(
        prompt.contains("1. [TDD-2] src/lib.rs:7 — the case is untested"),
        "{prompt}"
    );
    assert_eq!(rig.github.calls("POST", &format!("/repos/{SLUG}/pulls")).len(), 1);
    assert_eq!(rig.github.comments(1).len(), 1);
    let progress = state.teams()[0].item("1.1".parse().unwrap()).unwrap();
    assert_eq!(progress.fix_passes().len(), 1);
}

// ---- what goes wrong -------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_review_that_cannot_be_posted_still_counts_and_its_report_is_kept() {
    let mut rig = approving_rig().await;
    rig.github.refuse_reviews(422);
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    assert_eq!(rig.github.reviews(1), Vec::<serde_json::Value>::new());
    assert_eq!(
        rig.github.calls("PUT", &format!("/repos/{SLUG}/pulls/1/merge")).len(),
        1
    );
    for member in state.team(&team).unwrap().reviewers() {
        let number = agent_core::state::PrNumber::new(std::num::NonZeroU32::MIN);
        assert!(rig.home.review_report_file(member.task(), number, 0).exists());
    }
}

#[tokio::test]
async fn a_branch_that_cannot_be_pushed_is_told_once_and_published_when_the_remote_is_back_after_a_restart() {
    let mut rig = approving_rig().await;
    let hidden = rig.origin.with_file_name("hidden.git");
    std::fs::rename(&rig.origin, &hidden).unwrap();
    rig.start().await;

    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let notice = rig.notification_with("could not be published").await;
    rig.stop().await.unwrap();
    std::fs::rename(&hidden, &rig.origin).unwrap();
    rig.start().await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    assert!(notice.contains("item 1.1"), "{notice}");
    let told: Vec<_> = rig
        .notifications()
        .into_iter()
        .filter(|text| text.contains("could not be published"))
        .collect();
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(rig.github.calls("POST", &format!("/repos/{SLUG}/pulls")).len(), 1);
    assert!(rig.state().team(&team).is_some());
}

// ---- what the subscribers see ----------------------------------------------------------------------------------

use agent_core::protocol::{EventKind, Frame, Request, RequestId, SubscribeRequest};

/// The events of a subscription to every team, read until the ones a test waits for are in.
struct Subscription {
    reader: tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>,
    _writer: tokio::net::unix::OwnedWriteHalf,
}

impl Subscription {
    async fn to(rig: &Rig) -> Self {
        let (read, mut writer) = tokio::net::UnixStream::connect(rig.home.socket_file())
            .await
            .unwrap()
            .into_split();
        let mut reader = tokio::io::BufReader::new(read);
        let line = Request::new(
            RequestId(1),
            Client::Cli,
            Command::Subscribe(SubscribeRequest { team: None }),
        )
        .to_line()
        .unwrap();
        crate::lines::write_line(&mut writer, &line).await.unwrap();
        let first = crate::lines::read_line(&mut reader, 1 << 24).await.unwrap().unwrap();
        assert!(
            matches!(Frame::from_line(&first).unwrap(), Frame::Response(_)),
            "{first}"
        );
        Self {
            reader,
            _writer: writer,
        }
    }

    /// Reads events until `wanted` of them satisfy `pick`, and returns what it picked.
    async fn collect<T>(&mut self, wanted: usize, pick: impl Fn(&EventKind) -> Option<T>) -> Vec<T> {
        let mut found = Vec::new();
        while found.len() < wanted {
            let line = tokio::time::timeout(super::rig::PATIENCE, crate::lines::read_line(&mut self.reader, 1 << 24))
                .await
                .expect("timed out waiting for events")
                .unwrap()
                .expect("the connection ended while waiting for events");
            if let Frame::Event(event) = Frame::from_line(&line).unwrap()
                && let Some(picked) = pick(&event.event)
            {
                found.push(picked);
            }
        }
        found
    }
}

#[tokio::test]
async fn the_subscribers_see_the_pull_request_opened_the_reviews_come_in_and_the_item_merged() {
    let mut rig = approving_rig().await;
    rig.start().await;
    let mut events = Subscription::to(&rig).await;

    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let opened = events
        .collect(1, |kind| match kind {
            EventKind::PullRequestOpened { item, pr, .. } => Some((item.to_string(), pr.get().get())),
            _ => None,
        })
        .await;
    let verdicts = events
        .collect(2, |kind| match kind {
            EventKind::ReviewSubmitted { reviewer, verdict, .. } => Some((reviewer.as_str().to_owned(), *verdict)),
            _ => None,
        })
        .await;
    let statuses = events
        .collect(1, |kind| match kind {
            EventKind::ItemStatus { status, .. } if *status == agent_core::status::ItemStatus::Done => Some(*status),
            _ => None,
        })
        .await;
    rig.stop().await.unwrap();

    assert_eq!(opened, [("1.1".to_owned(), 1)]);
    let mut names: Vec<_> = verdicts.iter().map(|(name, _)| name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["inv-r1", "inv-r2"]);
    assert!(
        verdicts
            .iter()
            .all(|(_, verdict)| *verdict == agent_core::status::ReviewVerdict::Approve)
    );
    assert_eq!(statuses.len(), 1);
}

// ---- the reviewers are few ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_pull_request_waits_for_two_free_investigators_while_other_reviews_keep_them_busy() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            normal("claude-2", "work"),
            investigator("inv-r1", SILENT),
            investigator("inv-r2", SILENT),
            investigator("inv-r3", SILENT),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (first, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.state_where("the first pull request to have its reviews asked", |state| {
        state
            .team(&first)
            .and_then(|found| found.item("1.1".parse().unwrap()))
            .is_some_and(
                |progress| matches!(progress.phase(), ItemPhase::InReview { reviews, .. } if reviews.len() == 2),
            )
    })
    .await;
    let (second, _) = rig.run(&["claude-2"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .state_where("the second pull request to be open", |state| {
            state
                .team(&second)
                .and_then(|found| found.item("1.1".parse().unwrap()))
                .is_some_and(|progress| matches!(progress.phase(), ItemPhase::InReview { .. }))
        })
        .await;
    looked_again(&rig).await;
    let state_later = rig.state();
    rig.stop().await.unwrap();

    for state in [&state, &state_later] {
        let found = state.team(&second).unwrap();
        let ItemPhase::InReview { reviews, .. } = found.item("1.1".parse().unwrap()).unwrap().phase().clone() else {
            panic!("the item of the second team is not in review");
        };
        assert_eq!(reviews, []);
        assert_eq!(found.reviewers().len(), 0);
    }
    assert_eq!(rig.github.calls("POST", &format!("/repos/{SLUG}/pulls")).len(), 2);
}

#[tokio::test]
async fn a_branch_that_moved_after_the_gate_passed_is_not_published_with_that_gate() {
    let mut rig = Rig::new(
        &[
            // The second run, the fix pass, commits and records no gate: the record is of the commit before.
            normal("claude-1", "work,commit"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.prompt(&tasks[0], 2).await;
    rig.status_is(&tasks[0], TaskStatus::Waiting).await;
    let branch = format!("agents/claude-1/{}-1.1", tasks[0]);
    rig.wait_for("the fix to be committed", || {
        git(&rig.repo, &["rev-parse", &branch]) != rig.remote_tip(&branch)
    })
    .await;
    looked_again(&rig).await;
    rig.stop().await.unwrap();

    assert_eq!(rig.github.comments(1), Vec::<String>::new());
    assert!(matches!(phase_of(&rig.state(), "1.1"), ItemPhase::Fixing { .. }));
    assert_ne!(git(&rig.repo, &["rev-parse", &branch]), rig.remote_tip(&branch));
}
