//! Tests for bounded publication failures, lead notifications, human questions, and split part conflict publishing.

use agent_core::protocol::{AnswerRequest, Client, Command};
use agent_core::state::{ItemPhase, MergeMode};
use agent_core::status::TaskStatus;

use super::fake_github::MergeAnswer;
use super::publish::{Attempt, OwnGate, Site};
use super::rig::{ONE, Rig, git, investigator, normal};
use crate::client::Connection;
use crate::daemon::owner::Owner;
use crate::daemon::owner::fixture::{Daemon, Sinks, answers, n};
use crate::daemon::owner::rounds::Pulls;
use crate::daemon::pr::plan_publications;
use std::sync::Arc;

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

#[tokio::test]
async fn publish_with_stale_gate_record_and_new_worktree_commit_gates_and_publishes_new_commit() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "ask,done"),
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
    let worktree = rig.worktree(&task);

    // Wait until claude-1 asks questions and waits:
    rig.status_is(&task, TaskStatus::Questions).await;

    // Create First commit in worktree and record passing gate for it:
    std::fs::write(worktree.join("first.txt"), "first\n").unwrap();
    git(&worktree, &["add", "first.txt"]);
    git(&worktree, &["commit", "-q", "-m", "First commit"]);
    let first_commit = git(&worktree, &["rev-parse", "HEAD"]).trim().to_string();

    let gate_file = rig.home.gate_record_file(&task);
    std::fs::create_dir_all(gate_file.parent().unwrap()).unwrap();
    std::fs::write(
        &gate_file,
        format!(
            "{{\"tree\":\"clean\",\"result\":{{\"head\":\"{first_commit}\",\"finished_at\":\"2026-11-21T10:00:00Z\",\"outcome\":{{\"kind\":\"passed\"}},\"log\":\"/tmp/gate.log\"}}}}"
        ),
    )
    .unwrap();

    // Create Second commit in worktree (worktree HEAD moves ahead of gate record):
    let exclude = git(&worktree, &["rev-parse", "--git-path", "info/exclude"])
        .trim()
        .to_string();
    std::fs::create_dir_all(std::path::Path::new(&exclude).parent().unwrap()).unwrap();
    std::fs::write(&exclude, ".fake-*\n").unwrap();
    std::fs::write(worktree.join("second.txt"), "second\n").unwrap();
    git(&worktree, &["add", "second.txt"]);
    git(&worktree, &["commit", "-q", "-m", "Second commit"]);
    let second_commit = git(&worktree, &["rev-parse", "HEAD"]).trim().to_string();
    assert_ne!(first_commit, second_commit);

    // Answer questions so claude-1 finishes with done:
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: task.clone(),
                number: n(1),
                answers: answers(&[(1, "proceed")]),
            }),
        )
        .await
        .unwrap();

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
    assert_eq!(
        remote_head, second_commit,
        "remote PR branch must carry the new worktree commit, not the earlier pass"
    );
}

#[tokio::test]
async fn split_part_conflict_pass_with_lead_on_pr_part_branch_publishes_and_preserves_checkout() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate,integrate,script:conflict-part-branch"),
            normal("claude-1", "work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        true,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(
        scripts.join("conflict-part-branch.sh"),
        "set -eu\nfor _ in $(seq 300); do [[ -e .fake-base-ready ]] && break; sleep 0.1; done\nrm -f .fake-base-ready\npart_branch=$(git for-each-ref --format='%(refname:short)' 'refs/heads/agents/*/*-1.1a' | head -n 1)\ngit checkout \"$part_branch\"\ngit fetch -q origin main\ngit -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false merge -q --no-ff --no-edit origin/main\n",
    )
    .unwrap();
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
    let worktree = rig.worktree(&coordinator);
    assert_eq!(
        git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        pr.branch.as_str(),
        "worktree checkout must remain on the actual PR part branch"
    );
}

#[tokio::test]
async fn split_part_conflict_pass_with_lead_detached_publishes_and_preserves_checkout() {
    let mut rig = Rig::new(
        &[
            investigator("inv-lead", "ask,integrate,integrate,script:conflict-detached"),
            normal("claude-1", "work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        true,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(
        scripts.join("conflict-detached.sh"),
        "set -eu\nfor _ in $(seq 300); do [[ -e .fake-base-ready ]] && break; sleep 0.1; done\nrm -f .fake-base-ready\ngit checkout --detach HEAD\ngit fetch -q origin main\ngit -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false merge -q --no-ff --no-edit origin/main\n",
    )
    .unwrap();
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
    let worktree = rig.worktree(&coordinator);
    assert_eq!(
        git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "HEAD",
        "worktree checkout must remain detached on HEAD"
    );
}

#[tokio::test]
async fn publish_with_equal_remote_state_is_up_to_date_without_forcing() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work-pushed"),
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
        "remote PR branch must match the gated commit on equal remote outcome"
    );
}

#[tokio::test]
async fn publish_with_diverged_remote_branch_refuses_push_and_leaves_remote_unchanged() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "ask,work"),
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

    // Wait until claude-1 asks questions so task branch exists:
    rig.status_is(&task, TaskStatus::Questions).await;

    // Push an independent diverged commit directly to origin on the task PR branch:
    std::fs::write(rig.repo.join("diverged.txt"), "diverged remote work\n").unwrap();
    git(&rig.repo, &["add", "diverged.txt"]);
    git(&rig.repo, &["commit", "-q", "-m", "Remote diverged commit"]);
    let diverged_head = git(&rig.repo, &["rev-parse", "HEAD"]).trim().to_string();
    let target_branch = format!("agents/claude-1/{task}-1.1");
    let origin_str = rig.origin.to_str().unwrap();
    git(
        &rig.repo,
        &["push", "-q", origin_str, &format!("HEAD:refs/heads/{target_branch}")],
    );

    // Answer questions so claude-1 commits independent work and requests publication:
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: task.clone(),
                number: n(1),
                answers: answers(&[(1, "proceed")]),
            }),
        )
        .await
        .unwrap();

    // claude-1 finishes work with STATUS: DONE, but because initial publication is refused by origin
    // (carried is None, non-fast-forward push fails), the item remains InProgress.
    // In solo_waits_for_its_items, Settled::Status(TaskStatus::Done) is mapped to TaskStatus::Waiting.
    // The task never transitions to TaskStatus::Done; it settles into TaskStatus::Waiting.
    rig.status_is(&task, TaskStatus::Waiting).await;
    let note = rig.notification_with("origin refused the push of").await;
    assert!(
        note.contains("origin refused the push of"),
        "publication must be refused with origin refused notification: {note}"
    );
    rig.stop().await.unwrap();

    // The remote branch on origin must remain unchanged at diverged_head (not overwritten):
    let current_origin_head = git(&rig.origin, &["rev-parse", target_branch.as_str()])
        .trim()
        .to_string();
    assert_eq!(
        current_origin_head, diverged_head,
        "remote PR branch must remain unchanged at diverged head on publication refusal"
    );
}

#[tokio::test]
async fn existing_pr_equal_head_publication_takes_uptodate_and_skips_push() {
    const R1_CHANGES: &str = "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — see `HEAD` for issue → fix it\\n   CHECK: read `HEAD`,verify:VERDICT: APPROVE\\nSUMMARY: fixed in `HEAD`";

    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-script:held-fix-pushed"),
            investigator("inv-r1", R1_CHANGES),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    std::fs::write(
        scripts.join("held-fix-pushed.sh"),
        "set -eu\necho started > .fake-started-fix\nfor _ in $(seq 1 600); do\n  if [ -f .fake-release-fix ]; then\n    break\n  fi\n  sleep 0.05\ndone\nif [ ! -f .fake-release-fix ]; then\n  echo 'test never released fix' >&2\n  exit 65\nfi\ncommit() { git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false commit -q \"$@\"; }\necho 'fixed work' > fixed.txt\ngit add fixed.txt\ncommit -m 'Fix finding 1'\ngit push -q origin HEAD\n",
    )
    .unwrap();
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    let worktree = rig.worktree(&task);

    // Wait until round 1 is published and in review:
    rig.item_where("1.1", "in review", |phase| matches!(phase, ItemPhase::InReview { .. }))
        .await;

    // Synchronize deterministically before starting the fix:
    // Wait until claude-1 starts the fix run and holds before committing or pushing:
    let started_fix = worktree.join(".fake-started-fix");
    rig.wait_for("fix pass started", || started_fix.exists()).await;

    // Configure a client-facing receivepack wrapper on the worktree's origin remote.
    // Git invokes the remote.origin.receivepack executable for every client push command,
    // even when the remote ref is already equal to the pushed commit (which server-side pre-receive skips).
    let push_log = rig.origin.join("push-log");
    let wrapper_path = rig.origin.join("receivepack-wrapper.sh");
    std::fs::write(
        &wrapper_path,
        format!(
            "#!/bin/sh\nprintf 'push\\n' >> \"{}\"\nexec git-receive-pack \"$@\"\n",
            push_log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper_path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    git(
        &worktree,
        &["config", "remote.origin.receivepack", wrapper_path.to_str().unwrap()],
    );

    // Release claude-1 to commit the fix and push it directly to origin:
    std::fs::write(worktree.join(".fake-release-fix"), "release\n").unwrap();

    // When the daemon publishes the update, it observes existing PR carried == gated tip (Equal),
    // takes PublishAction::UpToDate, calls adopt_pushed, and skips push_commit.
    let state = rig
        .item_where("1.1", "approved or done", |phase| {
            matches!(phase, ItemPhase::Approved { .. } | ItemPhase::Done { .. })
        })
        .await;
    rig.stop().await.unwrap();

    // Exactly one client push reached origin: the push from held-fix-pushed in the fake-cli, and zero from daemon:
    let log_contents = std::fs::read_to_string(&push_log).expect("push-log must exist and be readable");
    assert_eq!(
        log_contents.lines().count(),
        1,
        "origin must receive exactly one client git push (from the agent CLI), proving daemon took UpToDate and skipped push: {log_contents}"
    );
    let item = state.teams()[0].item("1.1".parse().unwrap()).unwrap();
    assert!(matches!(
        item.phase(),
        ItemPhase::Approved { .. } | ItemPhase::Done { .. }
    ));
}

/// An owner of `state` with autopilot texts loaded for testing direct publish attempts.
async fn owner_of(state: &agent_core::state::State) -> (Daemon, Owner, Sinks, Pulls) {
    let daemon = Daemon::new(state, None);
    let secrets = daemon.home.root().join("secrets.toml");
    std::fs::write(&secrets, "github_token = \"ghp_test_token_0123\"\n").unwrap();
    std::fs::set_permissions(&secrets, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
    let (owner, sinks) = daemon.bare_owner().await;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let language = owner.parts.settings.config.language();
    let pulls = Pulls::new(
        Arc::new(
            agent_runtime::autopilot_prompts::AutopilotPrompts::load(&root, language)
                .await
                .unwrap(),
        ),
        Arc::new(
            agent_runtime::checks_prompts::ChecksPrompts::load(&root, language)
                .await
                .unwrap(),
        ),
        &owner.parts.settings,
    )
    .unwrap()
    .unwrap();
    (daemon, owner.with_pulls(pulls.clone()), sinks, pulls)
}

#[tokio::test]
async fn try_publish_while_own_gate_running_does_not_load_stale_or_invalid_gate_record() {
    use std::str::FromStr;

    let mut state = crate::fixtures::solo_state();
    let team_id = crate::fixtures::team_id(0);
    let item: agent_core::ids::ItemId = "1.1".parse().unwrap();
    let round: agent_core::ids::RoundId = "round-20261119-090000-wait".parse().unwrap();
    let branch: agent_core::ids::BranchName = "agents/claude-1/task-1-1.1".parse().unwrap();
    {
        let mut team = state.team_mut(&team_id).unwrap();
        team.open_round(round, Some(item), crate::fixtures::at(10)).unwrap();
        team.advance_item(
            item,
            ItemPhase::InProgress {
                assignee: "claude-1".parse().unwrap(),
                branch: branch.clone(),
            },
        )
        .unwrap();
        team.begin_run(
            &crate::fixtures::task(1),
            agent_core::state::NewRun {
                id: crate::fixtures::run(1),
                at: crate::fixtures::at(20),
                session: None,
                rounds: Vec::new(),
            },
        )
        .unwrap();
        let (_, mut lead) = team.find_mut(&crate::fixtures::task(1)).unwrap();
        lead.transition(TaskStatus::Running).unwrap();
        lead.end_run(
            crate::fixtures::at(30),
            agent_core::state::RunEnd::Reported {
                outcome: agent_core::status::RunOutcome::Done,
            },
        )
        .unwrap();
        lead.transition(TaskStatus::Waiting).unwrap();
    }
    let (daemon, mut owner, _sinks, pulls) = owner_of(&state).await;
    let repo = crate::fixtures::repo();
    let task = crate::fixtures::task(1);
    let worktree = daemon.home.worktree_dir(&repo, &task);
    std::fs::create_dir_all(&worktree).unwrap();
    git(&worktree, &["init", "-q", "-b", "main"]);
    std::fs::write(worktree.join("work.txt"), "hello\n").unwrap();
    git(&worktree, &["add", "work.txt"]);
    git(
        &worktree,
        &[
            "-c",
            "user.name=Fake",
            "-c",
            "user.email=fake@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "Initial commit",
        ],
    );
    let tip_str = git(&worktree, &["rev-parse", "HEAD"]);
    let tip: agent_core::state::CommitId = tip_str.trim().parse().unwrap();

    let lead = owner.lead_of(&team_id).unwrap();
    let site = Site {
        pulls: &pulls,
        team: &team_id,
        lead: &lead,
    };
    let plan = plan_publications(owner.parts.state.team(&team_id).unwrap()).unwrap();
    assert!(!plan.is_empty(), "plan must have at least one publication");
    let publication = &plan[0];
    let now = agent_core::state::Timestamp::now(&owner.parts.clock).unwrap();

    // 1. In-flight own gate: answer channel is open and has not answered yet.
    let (tx, rx) = tokio::sync::oneshot::channel();
    owner.watch.own_gates.insert(
        lead.task.clone(),
        OwnGate::Running {
            tip: tip.clone(),
            retry: false,
            answer: rx,
        },
    );

    // Write an invalid / in-flight record to gate.json.
    // If try_publish reads gate_record before checking own_gate_state (or while OwnGate::Running),
    // it will attempt to parse this invalid record and fail with StoreError::ParseGateRecord.
    // When read after own_gate_state reports completion, it returns Attempt::Nothing without loading the record.
    let gate_file = daemon.home.gate_record_file(&lead.task);
    std::fs::create_dir_all(gate_file.parent().unwrap()).unwrap();
    std::fs::write(&gate_file, "in-flight-invalid-json\n").unwrap();

    let attempt = owner.try_publish(site, publication, now).await;
    assert!(
        matches!(attempt, Ok(Attempt::Nothing)),
        "try_publish must not load gate record while own gate is running"
    );

    // 2. Complete the gate: write the valid passing record and send Ok on the channel.
    let record = agent_core::state::GateRecord {
        tree: agent_core::state::GateTree::Clean,
        result: agent_core::state::GateResult {
            head: tip.clone(),
            finished_at: now,
            outcome: agent_core::state::GateOutcome::Passed,
            log: agent_core::config::AbsolutePath::from_str("/tmp/gate.log").unwrap(),
        },
    };
    std::fs::write(&gate_file, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    tx.send(Ok(agent_core::protocol::Reply::GateRecorded {
        record: record.clone(),
    }))
    .unwrap();

    let attempt_completed = owner.try_publish(site, publication, now).await;
    assert!(
        !matches!(attempt_completed, Ok(Attempt::Nothing)),
        "try_publish after gate completion must read the updated gate record and proceed"
    );
}
