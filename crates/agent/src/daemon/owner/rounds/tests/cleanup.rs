//! Merged rounds release clean, published worktrees while preserving branches and unfinished work.

use agent_core::config::Secrets;
use agent_core::ids::TaskId;
use agent_core::log::{LogEvent, LogLevel};
use agent_core::protocol::{Client, Command, ControlRequest, Reply, Target};
use agent_core::state::Completion;
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
    for member in team.reviewers() {
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
async fn closed_round_leaves_worktrees_without_target_including_kept_worktree() {
    let mut rig = rig().await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    rig.stop().await.unwrap();
    let reviewer = state.team(&team).unwrap().reviewers()[0].task().clone();
    let tree = rig.worktree(&reviewer);
    std::fs::write(tree.join("unpublished.txt"), "keep this work\n").unwrap();
    let target = tree.join("target");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("output.bin"), b"large build artifact\n").unwrap();

    rig.github.merged_by_a_human(1);
    rig.start().await;
    let removed: Vec<_> = tasks(&state).into_iter().filter(|task| task != &reviewer).collect();
    cleaned(&rig, &removed).await;
    rig.stop().await.unwrap();

    assert!(tree.join("unpublished.txt").exists(), "uncommitted work must be kept");
    assert!(
        !target.exists(),
        "target/ directory must be removed even when worktree is kept"
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

#[tokio::test]
async fn a_git_removal_failure_keeps_the_worktree_and_does_not_block_team_closure() {
    let mut rig = rig().await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    rig.stop().await.unwrap();
    let reviewer = state.team(&team).unwrap().reviewers()[0].task().clone();
    let tree = rig.worktree(&reviewer);
    git(&rig.repo, &["worktree", "lock", tree.to_str().unwrap()]);
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
    assert!(tree.exists());
    let state = rig.state();
    assert_eq!(
        state.team(&team).unwrap().reviewers()[0].status(),
        agent_core::status::TaskStatus::Done
    );
    let logs = Reader::open(&rig.home.history_file()).unwrap().daemon_logs().unwrap();
    assert!(
        logs.iter().any(|row| row.event() == LogEvent::GitCallFailed
            && row.task() == Some(&reviewer)
            && row.message() == "the merged worktree cannot be removed and is kept"),
        "{logs:?}"
    );
}

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use agent_core::status::TaskStatus;
use agent_runtime::agentbus::AgentBus;
use agent_runtime::harness_prompts::HarnessPrompts;
use agent_runtime::history::Writer;
use agent_runtime::process::Supervisor;
use agent_runtime::prompts::PromptLoader;
use agent_runtime::questions_prompts::QuestionsPrompts;
use agent_runtime::run::{EngineParts, RunEngine, RunTimings};
use agent_runtime::settings::Settings;
use agent_runtime::store::StateStore;
use agent_runtime::team_prompts::TeamPrompts;
use rand::{SeedableRng, rngs::StdRng};
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::daemon::owner::{MAX_RUNS, Owner, OwnerParts};

/// Holds channel consumers while the test drives the same owner methods as the daemon loop.
struct Stepped {
    /// The production owner, with real engine, store and history.
    owner: Owner,
    /// The history writer to drain before restarting.
    writer: Option<Writer>,
    /// Notices are retained until settlement completes.
    _notices: mpsc::Receiver<crate::daemon::notifier::Delivery>,
    /// Events are retained until settlement completes.
    _events: broadcast::Receiver<agent_core::protocol::Event>,
}

/// Open a production owner without its scheduling loop, so the crash boundary requires no timing race.
async fn stepped(rig: &Rig) -> Stepped {
    let store = Arc::new(StateStore::open(rig.home.clone()).await.unwrap());
    let settings = Arc::new(Settings::load(&rig.home).await.unwrap());
    let prompts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../prompts");
    let language = settings.config.language();
    let texts = Arc::new(HarnessPrompts::load(&prompts, language).await.unwrap());
    let opened = crate::daemon::history::open(&settings, &rig.home).await;
    let engine = RunEngine::new(EngineParts {
        store: Arc::clone(&store),
        texts: Arc::clone(&texts),
        config: Arc::new(settings.config.clone()),
        secrets: Arc::new(settings.secrets.clone()),
        supervisor: Supervisor::new(MAX_RUNS),
        clock: rig.history_test_clock(),
        local_offset: time::UtcOffset::UTC,
        timings: RunTimings {
            poll: Duration::from_millis(20),
            hang: Duration::from_secs(20),
            stop_grace: Duration::from_secs(5),
        },
        agent_binary_dir: "/opt/bin".parse().unwrap(),
    })
    .with_history(opened.history.clone());
    let (notices, deliveries) = mpsc::channel(64);
    let (events, kept) = broadcast::channel(64);
    let parts = OwnerParts {
        state: store.load().await.unwrap(),
        team_texts: Arc::new(TeamPrompts::load(&prompts, language).await.unwrap()),
        question_texts: Arc::new(QuestionsPrompts::load(&prompts, language).await.unwrap()),
        rules: Arc::new(PromptLoader::load(&prompts, language).await.unwrap()),
        bus: AgentBus::new(
            settings.config.binaries().agentbus.clone(),
            rig.home.absolute_root().clone(),
        ),
        opens: None,
        engine,
        clock: rig.history_test_clock(),
        rng: StdRng::seed_from_u64(88),
        notices,
        events,
        local_offset: time::UtcOffset::UTC,
        tick: Duration::from_millis(25),
        store,
        settings,
        texts,
    };
    Stepped {
        owner: Owner::new(parts, CancellationToken::new()).with_history(opened.history),
        writer: opened.writer,
        _notices: deliveries,
        _events: kept,
    }
}

#[tokio::test]
async fn merged_worktrees_are_cleaned_once_when_the_running_lead_finishes() {
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::prelude::*;

    let mut rig = rig().await;
    rig.start().await;
    let (team, lead_tasks) = rig.run(&["claude-1"], ONE, MergeMode::NoMerge, None).await;
    let state = rig
        .item_where("1.1", "approved", |phase| matches!(phase, ItemPhase::Approved { .. }))
        .await;
    rig.stop().await.unwrap();
    let all_tasks = tasks(&state);
    let mut step = stepped(&rig).await;
    let lead = &lead_tasks[0];
    let now = "2026-11-17T16:00:00Z".parse().unwrap();
    {
        let mut found = step.owner.parts.state.team_mut(&team).unwrap();
        let ItemPhase::Approved { pr, .. } = found.item("1.1".parse().unwrap()).unwrap().phase().clone() else {
            panic!("expected approved item")
        };
        found
            .advance_item(
                "1.1".parse().unwrap(),
                ItemPhase::Done {
                    completion: agent_core::state::Completion::Merged { pr, at: now },
                },
            )
            .unwrap();
        let (_, mut member) = found.find_mut(lead).unwrap();
        member.transition(TaskStatus::Queued).unwrap();
        member.transition(TaskStatus::Running).unwrap();
    }
    let layer = HistoryLayer::deferred(LogLevel::Info, rig.history_test_clock(), Secrets::default());
    layer.attach_history(step.owner.history.clone());
    let subscriber = tracing_subscriber::registry().with(layer);
    async {
        for _ in 0..2 {
            step.owner.look_at(&team);
            step.owner.team_life().await.unwrap();
        }
        assert!(
            all_tasks.iter().all(|task| rig.worktree(task).exists()),
            "a running team's worktrees must remain until closure"
        );
        {
            let mut found = step.owner.parts.state.team_mut(&team).unwrap();
            let (_, mut member) = found.find_mut(lead).unwrap();
            member.transition(TaskStatus::Waiting).unwrap();
        }
        for _ in 0..2 {
            step.owner.look_at(&team);
            step.owner.team_life().await.unwrap();
        }
        assert!(all_tasks.iter().all(|task| !rig.worktree(task).exists()));
    }
    .with_subscriber(subscriber)
    .await;
    drop(step.owner);
    step.writer.take().unwrap().close().unwrap();
    let logs = Reader::open(&rig.home.history_file()).unwrap().daemon_logs().unwrap();
    assert!(
        logs.iter().all(|row| row.event() != LogEvent::GitCallFailed),
        "{logs:?}"
    );
}

/// A list whose first item was ticked before the team started and whose second the team does.
const TICKED_THEN_OPEN: &str = "\
Build the thing.

1. First
[x] 1.1 The first item
    Do the first thing in the repository.
    Expected result:
    - the first thing works
    Checks:
    - run the first thing
[ ] 1.2 The second item
    Do the second thing in the repository.
    Expected result:
    - the second thing works
    Checks:
    - run the second thing
";

#[tokio::test]
async fn a_team_whose_other_items_were_ticked_in_the_list_removes_its_worktrees_after_its_merge() {
    let mut rig = rig().await;
    rig.start().await;
    rig.run(&["claude-1"], TICKED_THEN_OPEN, MergeMode::Autopilot, None)
        .await;
    let state = rig
        .item_where("1.2", "merged", |phase| {
            matches!(
                phase,
                ItemPhase::Done {
                    completion: Completion::Merged { .. }
                }
            )
        })
        .await;
    let tasks = tasks(&state);
    assert_eq!(tasks.len(), 3);
    cleaned(&rig, &tasks).await;
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn a_list_without_items_keeps_the_worktree_of_its_lead_when_the_lead_is_done() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "done"),
            investigator("inv-r1", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
            investigator("inv-r2", "done:VERDICT: APPROVE\\nSUMMARY: fine"),
        ],
        5,
        false,
    )
    .await;
    std::fs::write(rig.repo.join(".git/info/exclude"), ".fake-*\n").unwrap();
    rig.start().await;
    let (_, tasks) = rig
        .run(&["claude-1"], "Explain the layout.\n", MergeMode::Autopilot, None)
        .await;
    rig.status_is(&tasks[0], TaskStatus::Done).await;
    rig.stop().await.unwrap();
    assert!(rig.worktree(&tasks[0]).exists());
}
