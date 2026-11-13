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
