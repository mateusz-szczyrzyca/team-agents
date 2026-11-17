//! Merged rounds release clean, published worktrees while preserving branches and unfinished work.

use agent_core::config::Secrets;
use agent_core::ids::TaskId;
use agent_core::log::{LogEvent, LogLevel};
use agent_core::protocol::{Client, Command, ControlRequest, Reply, Target};
use agent_core::state::{ItemPhase, MergeMode, State};
use agent_runtime::history::{HistoryLayer, Reader};

use super::rig::{CHAIN, ONE, Rig, git, investigator, normal};
use crate::client::Connection;

/// A published lead and two approving reviewers; fixture bookkeeping is ignored by Git.
async fn rig() -> Rig {
    let rig = Rig::new(
        &[
            normal("claude-1", "work"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    std::fs::write(rig.repo.join(".git/info/exclude"), ".fake-*\n").unwrap();
    rig
}

/// All tasks of the single team, including its reviewers.
fn tasks(state: &State) -> Vec<TaskId> {
    let team = &state.teams()[0];
    team.crew()
        .roster()
        .into_iter()
        .map(|(_, member)| member.task().clone())
        .chain(team.reviewers().iter().map(|member| member.task().clone()))
        .collect()
}

/// Waits for cleanup after the durable merge, which precedes filesystem side effects.
async fn cleaned(rig: &Rig, tasks: &[TaskId]) {
    rig.wait_for("merged worktrees to be removed", || {
        tasks.iter().all(|task| !rig.worktree(task).exists())
    })
    .await;
}

#[tokio::test]
async fn autopilot_merge_removes_lead_and_reviewers_but_keeps_all_branches() {
    let mut rig = rig().await;
    rig.start().await;
    rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "merged", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    let tasks = tasks(&state);
    assert_eq!(tasks.len(), 3);
    cleaned(&rig, &tasks).await;
    rig.stop().await.unwrap();
    let team = &state.teams()[0];
    let mut local = vec!["refs/heads/main".to_owned()];
    for member in team
        .crew()
        .roster()
        .into_iter()
        .map(|(_, member)| member)
        .chain(team.reviewers())
    {
        let agent = crate::daemon::team::agent_name(member.agent()).unwrap();
        let branch = crate::daemon::team::task_branch(&agent, member.task()).unwrap();
        local.push(format!("refs/heads/{branch}"));
    }
    let ItemPhase::Done {
        completion: agent_core::state::Completion::Merged { pr, .. },
    } = team.item("1.1".parse().unwrap()).unwrap().phase()
    else {
        panic!("expected merged item")
    };
    local.push(format!("refs/heads/{}", pr.branch));
    local.sort();
    assert_eq!(
        git(&rig.repo, &["for-each-ref", "--format=%(refname)", "refs/heads/"]),
        local.join("\n")
    );
    let mut remote = ["refs/heads/main".to_owned(), format!("refs/heads/{}", pr.branch)];
    remote.sort();
    assert_eq!(
        git(&rig.origin, &["for-each-ref", "--format=%(refname)", "refs/heads/"]),
        remote.join("\n")
    );
    let listed = git(&rig.repo, &["worktree", "list", "--porcelain"]);
    assert!(
        tasks
            .iter()
            .all(|task| !listed.contains(rig.worktree(task).to_str().unwrap()))
    );
}

#[tokio::test]
async fn no_merge_keeps_worktrees_until_the_humans_merge_is_seen() {
    let mut rig = rig().await;
    rig.start().await;
    rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    rig.stop().await.unwrap();
    let tasks = tasks(&state);
    assert!(tasks.iter().all(|task| rig.worktree(task).exists()));
    rig.github.merged_by_a_human(1);
    rig.start().await;
    cleaned(&rig, &tasks).await;
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn merged_round_keeps_a_dirty_reviewer_and_logs_why() {
    retained_reviewer(false).await;
}

#[tokio::test]
async fn merged_round_keeps_an_unpushed_reviewer_and_logs_why() {
    retained_reviewer(true).await;
}

/// Introduces either uncommitted or local-only work after approval, before the human merges.
async fn retained_reviewer(commit: bool) {
    let mut rig = rig().await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    rig.stop().await.unwrap();
    let reviewer = state.team(&team).unwrap().reviewers()[0].task().clone();
    let tree = rig.worktree(&reviewer);
    std::fs::write(tree.join("unpublished-λ.txt"), "keep this work\n").unwrap();
    if commit {
        git(&tree, &["add", "."]);
        git(&tree, &["commit", "-qm", "Local reviewer investigation"]);
    }
    rig.github.merged_by_a_human(1);
    rig.start_with_log_layer(HistoryLayer::deferred(
        LogLevel::Info,
        rig.history_test_clock(),
        Secrets::default(),
    ))
    .await;
    let removed: Vec<_> = tasks(&state).into_iter().filter(|task| task != &reviewer).collect();
    cleaned(&rig, &removed).await;
    rig.stop().await.unwrap();
    assert_eq!(
        std::fs::read_to_string(tree.join("unpublished-λ.txt")).unwrap(),
        "keep this work\n"
    );
    let logs = Reader::open(&rig.home.history_file()).unwrap().daemon_logs().unwrap();
    let reason = if commit {
        "commits not on origin"
    } else {
        "uncommitted changes"
    };
    assert!(
        logs.iter().any(|row| row.event() == LogEvent::GitCallFailed
            && row.message().contains(reason)
            && row.task() == Some(&reviewer)),
        "{logs:?}"
    );
}

#[tokio::test]
async fn stopping_a_round_keeps_every_worktree() {
    let mut rig = rig().await;
    rig.github.answer_merges(super::fake_github::MergeAnswer::Refuse(409));
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    let tasks = tasks(&state);
    let reply = Connection::connect(&rig.home.socket_file())
        .await
        .unwrap()
        .request(
            Client::Cli,
            Command::Stop(ControlRequest {
                target: Target::Team(team),
            }),
        )
        .await
        .unwrap();
    let Reply::Applied { .. } = reply else {
        panic!("{reply:?}")
    };
    rig.stop().await.unwrap();
    assert!(tasks.iter().all(|task| rig.worktree(task).exists()));
}

#[tokio::test]
async fn a_merged_item_does_not_remove_worktrees_of_the_next_open_round() {
    let mut rig = rig().await;
    rig.start().await;
    rig.run(&["claude-1"], CHAIN, MergeMode::NoMerge, None).await;
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    rig.stop().await.unwrap();
    rig.github.merged_by_a_human(1);
    rig.start().await;
    rig.item_where("1.2", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    rig.stop().await.unwrap();
    assert!(tasks(&state).iter().all(|task| rig.worktree(task).exists()));
}

#[tokio::test]
async fn a_deleted_origin_branch_cannot_authorize_removing_local_reviewer_work() {
    let mut rig = rig().await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    rig.stop().await.unwrap();
    let reviewer = state.team(&team).unwrap().reviewers()[0].task().clone();
    let tree = rig.worktree(&reviewer);
    std::fs::write(tree.join("investigation.txt"), "keep this commit\n").unwrap();
    git(&tree, &["add", "."]);
    git(&tree, &["commit", "-qm", "Investigate"]);
    git(&tree, &["push", "-q", "origin", "HEAD:investigation"]);
    git(&rig.origin, &["update-ref", "-d", "refs/heads/investigation"]);
    rig.github.merged_by_a_human(1);
    rig.start().await;
    let removed: Vec<_> = tasks(&state).into_iter().filter(|task| task != &reviewer).collect();
    cleaned(&rig, &removed).await;
    rig.stop().await.unwrap();
    assert!(
        tree.exists(),
        "a deleted remote branch must not authorize worktree removal"
    );
    assert_eq!(
        std::fs::read_to_string(tree.join("investigation.txt")).unwrap(),
        "keep this commit\n"
    );
}
