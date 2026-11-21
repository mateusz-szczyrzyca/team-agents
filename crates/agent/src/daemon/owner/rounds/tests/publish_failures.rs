//! Tests for bounded publication failures, lead notifications, human questions, and split part conflict publishing.

use agent_core::protocol::{AnswerRequest, Client, Command};
use agent_core::state::{ItemPhase, MergeMode};
use agent_core::status::TaskStatus;

use super::fake_github::MergeAnswer;
use super::rig::{ONE, Rig, git, investigator, normal};
use crate::client::Connection;
use crate::daemon::owner::fixture::{answers, n};

const APPROVING: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

fn move_base(rig: &Rig) {
    std::fs::write(rig.repo.join("base-file.txt"), "moved\n").unwrap();
    git(&rig.repo, &["add", "base-file.txt"]);
    git(&rig.repo, &["commit", "-q", "-m", "Move the base"]);
    let origin = rig.origin.to_str().unwrap();
    git(&rig.repo, &["push", "-q", origin, "HEAD:refs/heads/main"]);
}

struct DeferChmod(std::path::PathBuf);
impl Drop for DeferChmod {
    fn drop(&mut self) {
        drop(
            std::process::Command::new("chmod")
                .args(["755", self.0.to_str().unwrap()])
                .status(),
        );
    }
}
fn defer_chmod(p: std::path::PathBuf) -> DeferChmod {
    DeferChmod(p)
}

fn split_approval(count: usize) -> String {
    let parts = (0..count)
        .map(|index| {
            serde_json::json!({
                "title": format!("Document {index}"),
                "delivers": format!("Independent document {index}"),
                "checks": format!("Read document {index}"),
                "depends_on": if index == 0 { vec![] } else { vec!["a"] }
            })
        })
        .collect::<Vec<_>>();
    let plan = serde_json::json!({"parent": "1.1", "parts": parts});
    format!("split\n```json\n{plan}\n```")
}

#[tokio::test]
async fn split_part_conflict_pass_publishes_gated_commit_to_part_pr_branch() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "ask-here,work,merge-base,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (team_id, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();

    // Lead asks to split into 2 parts:
    rig.status_is(&task, TaskStatus::Questions).await;
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: task.clone(),
                number: n(1),
                answers: answers(&[(1, &split_approval(2))]),
            }),
        )
        .await
        .unwrap();

    // Part 1.1a PR #1 is approved; refuse merge for conflict:
    rig.github.answer_merges(MergeAnswer::Refuse(405));

    // Wait until conflict pass starts:
    rig.state_where("part 1.1a is in conflict pass", |state| {
        state.team(&team_id).unwrap().item("1.1".parse().unwrap()).unwrap().merge_conflict().is_some()
    })
    .await;

    // Allow merge, move base, signal fake base ready:
    rig.github.answer_merges(MergeAnswer::Allow);
    move_base(&rig);
    std::fs::write(rig.worktree(&task).join(".fake-base-ready"), "").unwrap();

    // Wait until part 1.1a PR merges and advances:
    let state = rig
        .state_where("part 1.1a PR merged", |state| {
            state.team(&team_id).unwrap().rounds().len() >= 2
                && !state.team(&team_id).unwrap().rounds()[0].is_open()
        })
        .await;
    rig.stop().await.unwrap();

    assert!(
        state.team(&team_id).unwrap().splits().len() == 1,
        "split part PR should complete and merge"
    );
}

#[tokio::test]
async fn publish_with_checked_out_worktree_publishes_without_forcing() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;

    let (_, _tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;

    let state = rig
        .item_where("1.1", "in review or approved or done", |phase| {
            matches!(
                phase,
                ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. }
            )
        })
        .await;
    rig.stop().await.unwrap();

    let item = state.teams()[0].item("1.1".parse().unwrap()).unwrap();
    assert!(
        matches!(
            item.phase(),
            ItemPhase::InReview { .. } | ItemPhase::Approved { .. } | ItemPhase::Done { .. }
        ),
        "pull request must be published even when branch is checked out in worktree"
    );
}

#[tokio::test]
async fn publish_failure_retries_tells_lead_and_asks_human() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,work,work"),
        ],
        5,
        false,
    )
    .await;

    // Make origin refs directory read-only so git push fails with permission denied / rejected:
    let origin_refs = rig.origin.join("refs");
    let _restore = defer_chmod(origin_refs.clone());
    std::process::Command::new("chmod")
        .args(["555", origin_refs.to_str().unwrap()])
        .status()
        .unwrap();

    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();

    // After bounded retries and telling lead, human is asked via questions file:
    rig.status_is(&task, TaskStatus::Questions).await;

    let qfile = rig.home.questions_file(&task, n(1));
    assert!(qfile.exists(), "questions file should be written");
    let text = std::fs::read_to_string(&qfile).unwrap();
    assert!(text.contains("could not be published"));

    // Answering 'b' stops the task:
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: task.clone(),
                number: n(1),
                answers: answers(&[(1, "b")]),
            }),
        )
        .await
        .unwrap();

    rig.status_is(&task, TaskStatus::Stopped).await;
    rig.stop().await.unwrap();
}
