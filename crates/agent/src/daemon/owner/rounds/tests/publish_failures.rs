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
            investigator("inv-lead", "ask,integrate,integrate,merge-base"),
            normal("claude-1", "work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        true,
    )
    .await;
    rig.start().await;

    let (team_id, tasks) = rig
        .run(&["inv-lead", "claude-1"], ONE, MergeMode::Autopilot, None)
        .await;
    let coordinator = tasks[0].clone();
    let lead = tasks[1].clone();

    // Coordinator asks to split into 2 parts:
    rig.status_is(&coordinator, TaskStatus::Questions).await;
    rig.status_is(&lead, TaskStatus::Waiting).await;
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: coordinator.clone(),
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
        state
            .team(&team_id)
            .unwrap()
            .item("1.1".parse().unwrap())
            .unwrap()
            .merge_conflict()
            .is_some()
    })
    .await;

    // Allow merge, move base, signal fake base ready in coordinator's worktree:
    rig.github.answer_merges(MergeAnswer::Allow);
    move_base(&rig);
    std::fs::write(rig.worktree(&coordinator).join(".fake-base-ready"), "").unwrap();

    // Wait until part 1.1a PR merges and advances:
    let state = rig
        .state_where("part 1.1a PR merged", |state| {
            let team = state.team(&team_id).unwrap();
            team.rounds().len() >= 2 && !team.rounds()[0].is_open()
        })
        .await;
    rig.stop().await.unwrap();

    let team = state.team(&team_id).unwrap();
    assert_eq!(team.splits().len(), 1, "split part PR should complete and merge");
    let pr = team.rounds()[0].part_pull_request().unwrap();
    assert_eq!(pr.number.get().get(), 1);
    let remote_head = git(&rig.origin, &["rev-parse", pr.branch.as_str()]).trim().to_string();
    let gate: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(rig.home.gate_record_file(&coordinator)).unwrap()).unwrap();
    let gated_commit = gate["result"]["head"].as_str().unwrap();
    assert_eq!(
        remote_head, gated_commit,
        "remote PR branch must carry the exact gated commit hash"
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

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();

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
    let (ItemPhase::InReview { pr, .. }
    | ItemPhase::Approved { pr, .. }
    | ItemPhase::Done {
        completion: agent_core::state::Completion::Merged { pr, .. },
    }) = item.phase()
    else {
        panic!("unexpected phase");
    };
    let remote_head = git(&rig.origin, &["rev-parse", pr.branch.as_str()]).trim().to_string();
    let gate: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(rig.home.gate_record_file(&task)).unwrap()).unwrap();
    let gated_commit = gate["result"]["head"].as_str().unwrap();
    assert_eq!(
        remote_head, gated_commit,
        "remote PR branch must carry the exact gated commit hash"
    );
}

#[tokio::test]
async fn publish_with_detached_worktree_publishes_gated_commit_without_forcing() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:detach"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(
        scripts.join("detach.sh"),
        "set -eu\ncommit() { git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q \"$@\"; }\necho 'detached work' > detached.txt\ngit add detached.txt\ncommit -m 'Detached commit'\ngit checkout --detach HEAD\n",
    )
    .unwrap();
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();

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
    let (ItemPhase::InReview { pr, .. }
    | ItemPhase::Approved { pr, .. }
    | ItemPhase::Done {
        completion: agent_core::state::Completion::Merged { pr, .. },
    }) = item.phase()
    else {
        panic!("unexpected phase");
    };
    let remote_head = git(&rig.origin, &["rev-parse", pr.branch.as_str()]).trim().to_string();
    let gate: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(rig.home.gate_record_file(&task)).unwrap()).unwrap();
    let gated_commit = gate["result"]["head"].as_str().unwrap();
    assert_eq!(
        remote_head, gated_commit,
        "remote PR branch must carry the exact gated commit hash"
    );
    let worktree = rig.worktree(&task);
    assert_eq!(
        git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "HEAD",
        "worktree must remain detached on HEAD"
    );
}

#[tokio::test]
async fn publish_with_detach_before_commit_publishes_gated_commit_and_preserves_checkout() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:detach-first"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(
        scripts.join("detach-first.sh"),
        "set -eu\ncommit() { git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q \"$@\"; }\ngit checkout --detach HEAD\necho 'detached work' > detached.txt\ngit add detached.txt\ncommit -m 'Detached commit'\n",
    )
    .unwrap();
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();

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
    let (ItemPhase::InReview { pr, .. }
    | ItemPhase::Approved { pr, .. }
    | ItemPhase::Done {
        completion: agent_core::state::Completion::Merged { pr, .. },
    }) = item.phase()
    else {
        panic!("unexpected phase");
    };
    let remote_head = git(&rig.origin, &["rev-parse", pr.branch.as_str()]).trim().to_string();
    let gate: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(rig.home.gate_record_file(&task)).unwrap()).unwrap();
    let gated_commit = gate["result"]["head"].as_str().unwrap();
    assert_eq!(
        remote_head, gated_commit,
        "remote PR branch must carry the exact gated commit hash"
    );
    let worktree = rig.worktree(&task);
    assert_eq!(
        git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "HEAD",
        "worktree must remain detached on HEAD"
    );
}

#[tokio::test]
async fn publish_with_part_checkout_before_commit_publishes_gated_commit_and_preserves_checkout() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:part-first"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(
        scripts.join("part-first.sh"),
        "set -eu\ncommit() { git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q \"$@\"; }\ngit checkout -b part-work\necho 'part work' > part.txt\ngit add part.txt\ncommit -m 'Part commit'\n",
    )
    .unwrap();
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();

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
    let (ItemPhase::InReview { pr, .. }
    | ItemPhase::Approved { pr, .. }
    | ItemPhase::Done {
        completion: agent_core::state::Completion::Merged { pr, .. },
    }) = item.phase()
    else {
        panic!("unexpected phase");
    };
    let remote_head = git(&rig.origin, &["rev-parse", pr.branch.as_str()]).trim().to_string();
    let gate: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(rig.home.gate_record_file(&task)).unwrap()).unwrap();
    let gated_commit = gate["result"]["head"].as_str().unwrap();
    assert_eq!(
        remote_head, gated_commit,
        "remote PR branch must carry the exact gated commit hash"
    );
    let worktree = rig.worktree(&task);
    assert_eq!(
        git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "part-work",
        "worktree must remain on part-work branch"
    );
}
