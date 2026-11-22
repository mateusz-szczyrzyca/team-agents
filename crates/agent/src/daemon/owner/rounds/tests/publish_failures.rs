//! Tests for gated publication to pull request branches across worktree checkouts and split conflict passes.

use agent_core::protocol::{AnswerRequest, Client, Command};
use agent_core::state::{ItemPhase, MergeMode};
use agent_core::status::TaskStatus;

use super::fake_github::MergeAnswer;
use super::publish::{Attempt, OwnGate, Site};
use super::rig::{ONE, PATIENCE, Rig, git, investigator, normal};
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
async fn conflict_publication_failure_preserves_budget_and_recovers_after_restart_and_successful_merge() {
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

    // Part 1.1a PR #1 is approved; refuse merge for conflict and move base:
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    move_base(&rig);

    // Wait until conflict pass starts:
    let state = rig
        .state_where("part 1.1a is in conflict pass", |state| {
            let item = state.team(&team_id).unwrap().item("1.1".parse().unwrap()).unwrap();
            item.merge_conflict().is_some() && item.merge_passes() == 1
        })
        .await;
    let item = state.team(&team_id).unwrap().item("1.1".parse().unwrap()).unwrap();
    assert_eq!(item.merge_passes(), 1, "conflict pass initiation spends one merge pass");

    // Install pre-receive hook in origin that rejects git push during conflict publication:
    let hook_dir = rig.origin.join("hooks");
    std::fs::create_dir_all(&hook_dir).unwrap();
    let hook = hook_dir.join("pre-receive");
    std::fs::write(&hook, "#!/bin/sh\necho 'pre-receive: denied' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    // Signal fake base ready in coordinator's worktree:
    std::fs::write(rig.worktree(&coordinator).join(".fake-base-ready"), "").unwrap();

    // Wait until lead merges base and gate passes, but conflict publication fails and pass is refunded:
    let state = rig
        .state_where("conflict publication fails and pass is refunded", |state| {
            let item = state.team(&team_id).unwrap().item("1.1".parse().unwrap()).unwrap();
            item.publish_tries() > 0 && item.merge_passes() == 0
        })
        .await;
    let item = state.team(&team_id).unwrap().item("1.1".parse().unwrap()).unwrap();
    assert_eq!(
        item.merge_passes(),
        0,
        "failed publication must not consume merge conflict pass"
    );

    // Stop daemon (restart between lead completion and publication):
    rig.stop().await.unwrap();

    // Remove the blocking hook so push can succeed:
    std::fs::remove_file(&hook).unwrap();

    // Restart daemon:
    rig.start().await;

    // Allow merge:
    rig.github.answer_merges(MergeAnswer::Allow);

    // Wait until part 1.1a PR merges and advances:
    let state = rig
        .state_where(
            "part 1.1a PR merged after restart and successful publication",
            |state| {
                let team = state.team(&team_id).unwrap();
                team.rounds().len() >= 2 && !team.rounds()[0].is_open()
            },
        )
        .await;
    rig.stop().await.unwrap();

    let team = state.team(&team_id).unwrap();
    assert_eq!(team.splits().len(), 1, "split part PR should complete and merge");
    let pr = team.rounds()[0].part_pull_request().unwrap();
    assert_eq!(pr.number.get().get(), 1);
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

/// Creates the base commit used to exercise gate completion without publication.
fn initial_gate_commit(worktree: &std::path::Path) -> agent_core::state::CommitId {
    std::fs::create_dir_all(worktree).unwrap();
    git(worktree, &["init", "-q", "-b", "main"]);
    std::fs::write(worktree.join("work.txt"), "hello\n").unwrap();
    git(worktree, &["add", "work.txt"]);
    git(
        worktree,
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
    let tip_str = git(worktree, &["rev-parse", "HEAD"]);
    tip_str.trim().parse().unwrap()
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
    let tip = initial_gate_commit(&worktree);

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
    // While own_gate_state reports running, it returns Attempt::Nothing without loading the record.
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
    tx.send(Ok(agent_core::protocol::Reply::GateRecorded { record: record.clone() }))
        .unwrap();

    let attempt_completed = owner.try_publish(site, publication, now).await;
    assert!(
        matches!(
            attempt_completed,
            Ok(Attempt::SentBack {
                tip: actual,
                hold: agent_core::decide::publish::GateHold::NoCommit,
            }) if actual == tip
        ),
        "try_publish after gate completion must read updated gate record and return SentBack with NoCommit at tip"
    );
}

#[tokio::test]
async fn publish_failure_retries_tells_lead_and_asks_human() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;

    // Set [publish] tries = 1 so the first failure tells the lead and the second asks the human:
    let mut config_text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config_text.push_str("\n[publish]\ntries = 1\n");
    std::fs::write(rig.home.config_file(), config_text).unwrap();

    // Hook in origin that rejects git push:
    let hook_dir = rig.origin.join("hooks");
    std::fs::create_dir_all(&hook_dir).unwrap();
    let hook = hook_dir.join("pre-receive");
    std::fs::write(&hook, "#!/bin/sh\necho 'pre-receive: publish denied' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();

    // First publication failure tells lead, waking it for run 2:
    rig.wait_for("first run ended and lead woken for retry", || {
        rig.state().find_task(&task).is_some_and(|f| f.member.runs().len() >= 2)
    })
    .await;

    // Verify run 2 was given the publish failure prompt and that notice was delivered:
    let prompt2 = rig.prompt(&task, 2).await;
    assert!(
        prompt2.contains("# YOUR WORK IS NOT PUBLISHED: THE PUBLICATION FAILED"),
        "run 2 prompt must contain the publish failure lead notice, got: {prompt2}"
    );
    assert!(
        prompt2.contains("The harness could not publish your branch"),
        "run 2 prompt must contain the publication failure explanation"
    );

    // After bounded retries and telling lead, human is asked via questions file:
    rig.status_is(&task, TaskStatus::Questions).await;

    let qfile = rig.home.questions_file(&task, n(1));
    assert!(qfile.exists(), "questions file should be written");
    let text = std::fs::read_to_string(&qfile).unwrap();
    assert!(text.contains("could not be published"), "{text}");

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

#[tokio::test]
async fn publish_failure_restart_recovers_question_and_answer_retries_and_succeeds() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,work,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;

    // Set [publish] tries = 1 so the first failure tells the lead and the second asks the human:
    let mut config_text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config_text.push_str("\n[publish]\ntries = 1\n");
    std::fs::write(rig.home.config_file(), config_text).unwrap();

    // Hook in origin that rejects git push:
    let hook_dir = rig.origin.join("hooks");
    std::fs::create_dir_all(&hook_dir).unwrap();
    let hook = hook_dir.join("pre-receive");
    std::fs::write(&hook, "#!/bin/sh\necho 'pre-receive: publish denied' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();

    // First publication failure tells lead, waking it for run 2:
    rig.wait_for("first run ended and lead woken for retry", || {
        rig.state().find_task(&task).is_some_and(|f| f.member.runs().len() >= 2)
    })
    .await;

    // Verify run 2 was given the publish failure prompt and that notice was delivered:
    let prompt2 = rig.prompt(&task, 2).await;
    assert!(
        prompt2.contains("# YOUR WORK IS NOT PUBLISHED: THE PUBLICATION FAILED"),
        "run 2 prompt must contain the publish failure lead notice, got: {prompt2}"
    );
    assert!(
        prompt2.contains("The harness could not publish your branch"),
        "run 2 prompt must contain the publication failure explanation"
    );

    // After bounded retries and telling lead, human is asked via questions file:
    rig.status_is(&task, TaskStatus::Questions).await;

    let qfile = rig.home.questions_file(&task, n(1));
    assert!(qfile.exists(), "questions file should be written");

    // Restart daemon while the publication question is open:
    rig.stop().await.unwrap();
    rig.start().await;

    rig.status_is(&task, TaskStatus::Questions).await;

    // Remove the rejecting hook so that git push can succeed:
    std::fs::remove_file(&hook).unwrap();

    // Answering 'a' (retry) resumes the task and publication succeeds:
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: task.clone(),
                number: n(1),
                answers: answers(&[(1, "a")]),
            }),
        )
        .await
        .unwrap();

    rig.wait_for("publication succeeds and pull request is opened", || {
        !rig.github.pulls().is_empty()
    })
    .await;

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn publish_failure_question_creation_interrupted_before_commit_recovers_without_replacing_existing_file_or_answer()
 {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,work,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;

    // Set [publish] tries = 0 so publication immediately escalates to asking the human:
    let mut config_text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config_text.push_str("\n[publish]\ntries = 0\n");
    std::fs::write(rig.home.config_file(), config_text).unwrap();

    // Hook in origin that rejects git push:
    let hook_dir = rig.origin.join("hooks");
    std::fs::create_dir_all(&hook_dir).unwrap();
    let hook = hook_dir.join("pre-receive");
    std::fs::write(&hook, "#!/bin/sh\necho 'pre-receive: publish denied' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    rig.start().await;

    let (team_id, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();

    // Publication failure escalates to human question:
    rig.status_is(&task, TaskStatus::Questions).await;

    let qfile = rig.home.questions_file(&task, n(1));
    assert!(qfile.exists(), "questions file should be written");

    // Simulate crash before question commit: human writes answer to file, state still shows task queued/running without question:
    rig.stop().await.unwrap();
    let custom_answer_text = "# Questions\n\n## 1. Action?\n**Verdict:** a\n\n**ANSWERED**\n";
    std::fs::write(&qfile, custom_answer_text).unwrap();

    // Restore exact interrupted state before question commit in state.json:
    // Human has written the answer into the questions file on disk, but state.json still shows
    // the task waiting from its completed run, with no questions registered yet.
    let state_file = rig.home.state_file();
    let mut state_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state_file).unwrap()).unwrap();
    state_json["teams"][0]["items"][0]["publish_tries"] = serde_json::json!(0);
    state_json["teams"][0]["items"][0]["publish_question"] = serde_json::Value::Null;
    state_json["teams"][0]["items"][0]["publish_question_task"] = serde_json::Value::Null;
    state_json["teams"][0]["items"][0]["publish_question_number"] = serde_json::Value::Null;
    state_json["teams"][0]["crew"]["member"]["questions"] = serde_json::json!([]);
    state_json["teams"][0]["crew"]["member"]["status"] = serde_json::json!("waiting");
    std::fs::write(&state_file, serde_json::to_string_pretty(&state_json).unwrap()).unwrap();

    // Assert pre-commit state before restart:
    let pre_state = rig.state();
    let pre_team = pre_state.team(&team_id).unwrap();
    let pre_item = pre_team.item("1.1".parse().unwrap()).unwrap();
    let pre_member = &pre_state.find_task(&task).unwrap().member;
    assert_eq!(pre_member.status(), TaskStatus::Waiting);
    assert_eq!(pre_member.questions(), []);
    assert!(pre_item.publish_question().is_none());
    assert!(pre_item.publish_question_task().is_none());
    assert!(pre_item.publish_question_number().is_none());
    assert_eq!(pre_item.publish_tries(), 0);

    std::fs::remove_file(&hook).unwrap();
    rig.start().await;

    // After re-evaluating publication failure, the daemon must NOT overwrite existing human answer:
    rig.wait_for("saved publication answer adopted", || {
        rig.state()
            .find_task(&task)
            .unwrap()
            .member
            .questions()
            .last()
            .is_some_and(|q| !q.is_open())
    })
    .await;

    let file_content_after_restart = std::fs::read_to_string(&qfile).unwrap();
    assert!(
        file_content_after_restart.contains("**Verdict:** a"),
        "restart must NOT replace existing question file or erase human answer, got: {file_content_after_restart}"
    );

    // Active question identity in state must match question 1:
    let active_q = rig
        .state()
        .find_task(&task)
        .unwrap()
        .member
        .questions()
        .last()
        .unwrap()
        .clone();
    assert_eq!(active_q.number(), n(1));
    assert_eq!(active_q.file().as_path(), qfile.as_path());

    // Duplicate questions-2.md must not be created:
    assert!(
        !rig.home.questions_file(&task, n(2)).exists(),
        "duplicate questions-2.md must not be created"
    );

    rig.wait_for("publication succeeds and pull request is opened", || {
        !rig.github.pulls().is_empty()
    })
    .await;

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn publish_failure_interrupted_lead_release_recovers_after_restart_and_escalates_to_human() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;

    let hook_dir = rig.origin.join("hooks");
    std::fs::create_dir_all(&hook_dir).unwrap();
    let hook = hook_dir.join("pre-receive");
    std::fs::write(&hook, "#!/bin/sh\necho 'pre-receive: publish denied' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    // Start with default tries=3 so first failure enters durable retry-only backoff:
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();

    // Synchronize on durable first publication failure entering retry-only backoff:
    rig.state_where(
        "first publication rejection persists retry-only state with publish_tries=1",
        |state| {
            state
                .teams()
                .iter()
                .any(|t| t.items().iter().any(|item| item.publish_tries() == 1))
        },
    )
    .await;

    // Stop daemon safely while idle in backoff:
    rig.stop().await.unwrap();

    // While stopped:
    // 1. Configure tries = 1 so the retry attempt becomes the lead-exhaustion attempt:
    let mut config_text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config_text.push_str("\n[publish]\ntries = 1\n");
    std::fs::write(rig.home.config_file(), config_text).unwrap();

    // 2. Establish exact interrupted queued fixture in state.json:
    // Lead was released for corrective run, publish_tries is 1, and no questions have been created yet:
    let state_file = rig.home.state_file();
    let mut state_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state_file).unwrap()).unwrap();
    state_json["teams"][0]["items"][0]["publish_tries"] = serde_json::json!(1);
    state_json["teams"][0]["items"][0]["publish_question"] = serde_json::Value::Null;
    state_json["teams"][0]["items"][0]["publish_question_task"] = serde_json::Value::Null;
    state_json["teams"][0]["items"][0]["publish_question_number"] = serde_json::Value::Null;
    state_json["teams"][0]["crew"]["member"]["status"] = serde_json::json!("queued");
    state_json["teams"][0]["crew"]["member"]["questions"] = serde_json::json!([]);
    if let Some(runs) = state_json["teams"][0]["crew"]["member"]["runs"].as_array_mut() {
        runs.truncate(1);
    }
    std::fs::write(&state_file, serde_json::to_string_pretty(&state_json).unwrap()).unwrap();

    let pre_state = rig.state();

    // Restart daemon: the queued corrective run executes:
    rig.start().await;
    rig.wait_for("lead executes run 2 retry", || {
        rig.state().find_task(&task).is_some_and(|f| f.member.runs().len() >= 2)
    })
    .await;

    // After run 2 completes and publication fails again (tries=2 > limit=1),
    // it escalates to asking the human with question 1:
    rig.status_is(&task, TaskStatus::Questions).await;
    assert_interrupted_lead_recovery_state(&pre_state, &task, &rig.state());

    // Remove rejecting hook and answer 'a' to retry publication:
    std::fs::remove_file(&hook).unwrap();
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: task.clone(),
                number: n(1),
                answers: answers(&[(1, "a")]),
            }),
        )
        .await
        .unwrap();

    rig.wait_for("publication succeeds and pull request is opened", || {
        !rig.github.pulls().is_empty()
    })
    .await;

    rig.stop().await.unwrap();
}

fn assert_interrupted_lead_recovery_state(
    pre_state: &agent_core::state::State,
    task: &agent_core::ids::TaskId,
    post_state: &agent_core::state::State,
) {
    let member = &pre_state.find_task(task).unwrap().member;
    assert_eq!(member.status(), TaskStatus::Queued);
    assert_eq!(member.runs().len(), 1);
    assert_eq!(member.questions(), []);
    assert_eq!(pre_state.teams()[0].items()[0].publish_tries(), 1);
    assert_eq!(pre_state.teams()[0].items()[0].publish_question(), None);

    let post_member = &post_state.find_task(task).unwrap().member;
    assert_eq!(post_member.runs().len(), 2, "exactly one retry run should execute");
    let q = post_member.open_question().unwrap().clone();
    assert_eq!(q.number(), n(1));
    assert_eq!(post_state.teams()[0].items()[0].publish_tries(), 2);
}

fn assert_publish_write_error(
    stop_err: crate::daemon::DaemonError,
    expected_task: &agent_core::ids::TaskId,
    expected_path: &std::path::Path,
) {
    let crate::daemon::DaemonError::Owner(crate::daemon::owner::OwnerError::PublishQuestion(
        crate::daemon::owner::rounds::PublishQuestionError::Write {
            task: err_task,
            path: err_path,
            ..
        },
    )) = stop_err
    else {
        panic!("unexpected daemon error: {stop_err:?}");
    };
    assert_eq!(&err_task, expected_task);
    assert_eq!(&err_path, expected_path);
}

#[tokio::test]
async fn publish_failure_question_write_failure_recovers_when_storage_restored() {
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

    // Hook in origin that rejects git push:
    let hook_dir = rig.origin.join("hooks");
    std::fs::create_dir_all(&hook_dir).unwrap();
    let hook = hook_dir.join("pre-receive");
    std::fs::write(&hook, "#!/bin/sh\necho 'pre-receive: publish denied' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    // Start with default tries=3: first publication rejection enters a retry-only backoff state:
    rig.start().await;

    let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();

    // Synchronize on durable first publication failure entering retry-only backoff state:
    rig.state_where(
        "first publication rejection persists retry-only state with publish_tries=1",
        |state| {
            state
                .teams()
                .iter()
                .any(|t| t.items().iter().any(|item| item.publish_tries() == 1))
        },
    )
    .await;

    // Stop daemon safely while idle in backoff (no live race condition):
    rig.stop().await.unwrap();

    // Now establish failure conditions before restarting:
    // 1. Set [publish] tries = 0 so next attempt immediately escalates to asking the human:
    let mut config_text = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config_text.push_str("\n[publish]\ntries = 0\n");
    std::fs::write(rig.home.config_file(), config_text).unwrap();

    // 2. Simulate storage failure: make task directory read-only so question file cannot be written:
    let task_dir = rig.home.task_dir(&task);
    std::fs::set_permissions(
        &task_dir,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o555),
    )
    .unwrap();

    // Restart daemon with failure conditions established:
    rig.start().await;

    // Wait deterministically for daemon to attempt question creation, fail with write error, and stop:
    let deadline = std::time::Instant::now() + PATIENCE;
    loop {
        if Connection::connect(&rig.home.socket_file()).await.is_err() {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for daemon to stop on write failure"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    // Assert exact error and context from owner:
    let stop_err = rig
        .stop()
        .await
        .expect_err("owner should record write failure when directory is unwritable");
    assert_publish_write_error(stop_err, &task, &rig.home.questions_file(&task, n(1)));

    // State before recovery must remain in Waiting without orphaned question:
    let pre_state = rig.state();
    let member = &pre_state.find_task(&task).unwrap().member;
    assert_eq!(member.status(), TaskStatus::Waiting);
    assert_eq!(member.open_question(), None);

    // Restore permissions so writing can succeed:
    std::fs::set_permissions(
        &task_dir,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o755),
    )
    .unwrap();

    // Restart daemon: pending escalation recovers cleanly, creates question file and transitions task to Questions:
    rig.start().await;
    rig.status_is(&task, TaskStatus::Questions).await;
    let q = rig
        .state()
        .find_task(&task)
        .unwrap()
        .member
        .open_question()
        .unwrap()
        .clone();
    assert_eq!(q.number(), n(1));

    // Remove rejecting hook and answer 'a' to complete publication:
    std::fs::remove_file(&hook).unwrap();
    Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Answer(AnswerRequest {
                task: task.clone(),
                number: n(1),
                answers: answers(&[(1, "a")]),
            }),
        )
        .await
        .unwrap();

    rig.wait_for("publication succeeds and pull request is opened", || {
        !rig.github.pulls().is_empty()
    })
    .await;

    rig.stop().await.unwrap();
}

#[tokio::test]
async fn publication_default_three_attempts_back_off_and_never_interrupt_a_running_lead() {
    for conflict in [false, true] {
        let scenario = if conflict {
            "work,merge-base,script:hold-publication"
        } else {
            "work,script:hold-publication"
        };
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
        let scripts = rig.home.root().join("fake-scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        std::fs::write(
            scripts.join("hold-publication.sh"),
            "set -eu\ntouch .fake-publication-running\nfor attempt in $(seq 300); do if [[ -e .fake-publication-release ]]; then exit 0; fi; sleep 0.1; done\nexit 1\n",
        )
        .unwrap();
        let hook = rig.origin.join("hooks/pre-receive");
        if conflict {
            rig.github.answer_merges(MergeAnswer::Refuse(405));
        } else {
            std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
            std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        }
        let layer = agent_runtime::history::HistoryLayer::deferred(
            agent_core::log::LogLevel::Info,
            rig.history_test_clock(),
            agent_core::config::Secrets::default(),
        );
        rig.start_with_log_layer(layer).await;
        let (_, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
        let task = tasks[0].clone();
        if conflict {
            rig.state_where("merge conflict pass", |state| {
                state.teams()[0].items()[0].merge_conflict().is_some()
            })
            .await;
            move_base(&rig);
            std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
            std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
            std::fs::write(rig.worktree(&task).join(".fake-base-ready"), "").unwrap();
        }
        rig.wait_for("corrective publication run is held", || {
            rig.worktree(&task).join(".fake-publication-running").exists()
        })
        .await;
        let state = rig.state();
        assert_eq!(state.teams()[0].items()[0].publish_tries(), 3);
        assert_eq!(state.find_task(&task).unwrap().member.status(), TaskStatus::Running);
        assert_eq!(state.find_task(&task).unwrap().member.open_question(), None);
        let reader = agent_runtime::history::Reader::open(&rig.home.history_file()).unwrap();
        let logs = reader.daemon_logs().unwrap();
        let attempts: Vec<_> = logs
            .iter()
            .filter(|entry| entry.event() == agent_core::log::LogEvent::GithubCallFailed)
            .filter(|entry| entry.fields().contains_key("tries"))
            .collect();
        assert_eq!(attempts.len(), 2, "only attempts below the default limit are retried");
        assert_eq!(attempts[0].fields().get("tries"), Some(&serde_json::json!(1)));
        assert_eq!(attempts[1].fields().get("tries"), Some(&serde_json::json!(2)));
        let gap = attempts[1].first_at().as_utc() - attempts[0].first_at().as_utc();
        assert!(
            gap.whole_seconds() >= 50,
            "conflict={conflict}: retry must honor a minute's backoff: {gap}"
        );
        rig.advance_history_days(1);
        rig.state_where("running lead remains undisturbed after backoff", |state| {
            state.find_task(&task).unwrap().member.status() == TaskStatus::Running
        })
        .await;
        assert_eq!(rig.state().teams()[0].items()[0].publish_tries(), 3);
        assert!(!rig.home.questions_file(&task, n(1)).exists());
        std::fs::write(rig.worktree(&task).join(".fake-publication-release"), "").unwrap();
        rig.status_is(&task, TaskStatus::Questions).await;
        assert_eq!(rig.state().teams()[0].items()[0].publish_tries(), 4);
        assert!(!rig.home.questions_file(&task, n(2)).exists());
        rig.stop().await.unwrap();
    }
}

#[tokio::test]
async fn conflict_publication_adopts_an_answer_saved_before_question_registration_after_restart() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,merge-base,work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let config = rig.home.config_file();
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str("\n[publish]\ntries = 0\n");
    std::fs::write(config, text).unwrap();
    rig.github.answer_merges(MergeAnswer::Refuse(405));
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let task = tasks[0].clone();
    rig.state_where("conflict pass requested", |state| {
        state.team(&team).unwrap().items()[0].merge_conflict().is_some()
    })
    .await;
    move_base(&rig);
    let hook = rig.origin.join("hooks/pre-receive");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    std::fs::write(rig.worktree(&task).join(".fake-base-ready"), "").unwrap();
    rig.status_is(&task, TaskStatus::Questions).await;
    rig.stop().await.unwrap();
    let file = rig.home.questions_file(&task, n(1));
    let answer = "# Questions\n\n## 1. Retry?\n**Verdict:** a\n\n**ANSWERED**\n";
    std::fs::write(&file, answer).unwrap();
    let mut state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(rig.home.state_file()).unwrap()).unwrap();
    state["teams"][0]["items"][0]["publish_question"] = serde_json::Value::Null;
    state["teams"][0]["items"][0]["publish_tries"] = serde_json::json!(0);
    state["teams"][0]["crew"]["member"]["questions"] = serde_json::json!([]);
    state["teams"][0]["crew"]["member"]["status"] = serde_json::json!("waiting");
    std::fs::write(rig.home.state_file(), serde_json::to_string_pretty(&state).unwrap()).unwrap();
    std::fs::remove_file(hook).unwrap();
    rig.github.answer_merges(MergeAnswer::Allow);
    rig.start().await;
    rig.state_where("saved conflict answer adopted", |state| {
        state
            .find_task(&task)
            .unwrap()
            .member
            .questions()
            .last()
            .is_some_and(|q| !q.is_open())
    })
    .await;
    let state = rig.state();
    let question = state.find_task(&task).unwrap().member.questions().last().unwrap();
    assert_eq!(question.number(), n(1));
    assert_eq!(question.file().as_path(), file.as_path());
    assert_eq!(std::fs::read_to_string(file).unwrap(), answer);
    assert!(!rig.home.questions_file(&task, n(2)).exists());
    rig.item_where(
        "1.1",
        "conflict publication merges after automatic answer adoption",
        |phase| matches!(phase, ItemPhase::Done { .. }),
    )
    .await;
    rig.stop().await.unwrap();
}
