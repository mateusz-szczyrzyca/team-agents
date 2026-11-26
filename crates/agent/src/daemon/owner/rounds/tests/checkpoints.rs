//! The checkpoints of a round through a daemon in a temporary home: a stand-in CLI that commits as a script says, a
//! scratch repository with a bare `origin`, a fake GitHub and the stand-ins of the reviewers. The checkpoints are read
//! back from the history after the daemon stopped, and their snapshots from the repository.

use std::collections::BTreeSet;
use std::path::Path;

use agent_core::history::{CheckpointCause, HistoryEvent, Record};
use agent_core::ids::TeamId;
use agent_core::log::LogEvent;
use agent_core::state::{CommitId, ItemPhase, MergeMode};
use agent_runtime::history::Reader;

use super::rig::{ONE, Rig, git, investigator, normal};

/// A reviewer that approves.
const APPROVING: &str = "verify:VERDICT: APPROVE\\nSUMMARY: fine";

/// A reviewer that asks for changes the first time and approves afterwards.
const CHANGES_ONCE: &str = "verify:VERDICT: REQUEST_CHANGES\\nFINDINGS\\n1. [MAJOR] TDD-2 src/lib.rs:7 — the case is untested → add coverage\\n   CHECK: read src/lib.rs:7,verify:VERDICT: APPROVE\\nSUMMARY: fixed";

/// What a checkpoint says that the tests look at.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Taken {
    /// The step it follows.
    cause: CheckpointCause,
    /// The tip of the worktree's branch.
    head: Option<CommitId>,
    /// The reference of its snapshot.
    snapshot: Option<String>,
    /// The review pass of the round then.
    review_pass: Option<u32>,
    /// The fix pass of the round then.
    fix_pass: Option<u32>,
    /// Whether the pull request was a draft.
    draft: bool,
}

/// The agents of a solo round whose work is the script `commits` and whose reviewers are never asked.
fn committing() -> [super::rig::Agent; 3] {
    [
        normal("claude-1", "script:commits"),
        investigator("inv-r1", APPROVING),
        investigator("inv-r2", APPROVING),
    ]
}

/// The checkpoints of `team` in the order they were written.
fn checkpoints(rig: &Rig, team: &TeamId) -> Vec<Taken> {
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let mut taken = Vec::new();
    for stored in reader.team_records(team).unwrap() {
        if let Record::Event(event) = stored.record
            && let HistoryEvent::Checkpoint {
                cause,
                head,
                snapshot,
                review_pass,
                fix_pass,
                draft,
                ..
            } = event.event
        {
            taken.push(Taken {
                cause,
                head,
                snapshot,
                review_pass,
                fix_pass,
                draft,
            });
        }
    }
    taken
}

/// The heads of the checkpoints of `cause`, oldest first.
fn heads_of(rig: &Rig, team: &TeamId, cause: CheckpointCause) -> Vec<CommitId> {
    checkpoints(rig, team)
        .into_iter()
        .filter(|taken| taken.cause == cause)
        .filter_map(|taken| taken.head)
        .collect()
}

/// The commits reachable from the head of `repository`, newest first.
fn reachable(repository: &Path) -> Vec<String> {
    git(repository, &["rev-list", "HEAD"])
        .lines()
        .map(str::to_owned)
        .collect()
}

/// Writes the script `name` of the stand-in, which commits as the test says: it commits twice, then waits for the file
/// `third`, commits again, and waits for `finish` before it ends. Returns the directory of the files it waits for and writes.
fn commit_script(rig: &Rig, name: &str) -> std::path::PathBuf {
    let signals = rig.home.root().join("signals");
    std::fs::create_dir_all(&signals).unwrap();
    let scripts = rig.home.root().join("fake-scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let text = format!(
        "set -eu\n\
         sig='{}'\n\
         g() {{ git -c user.name=Fake -c user.email=fake@example.com -c commit.gpgsign=false \"$@\"; }}\n\
         g commit -q --allow-empty -m one\n\
         g commit -q --allow-empty -m two\n\
         touch \"$sig/two\"\n\
         while [ ! -e \"$sig/third\" ]; do sleep 0.05; done\n\
         g commit -q --allow-empty -m three\n\
         touch \"$sig/three\"\n\
         while [ ! -e \"$sig/finish\" ]; do sleep 0.05; done\n",
        signals.display()
    );
    std::fs::write(scripts.join(format!("{name}.sh")), text).unwrap();
    signals
}

#[tokio::test]
async fn a_local_commit_is_a_checkpoint_pushed_or_not_and_its_snapshot_outlives_the_worktree() {
    let mut rig = Rig::new(&committing(), 5, false).await;
    let signals = commit_script(&rig, "commits");
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);

    rig.wait_for("two commits", || signals.join("two").exists()).await;
    let two = git(&worktree, &["rev-parse", "HEAD"]);
    rig.wait_for("the checkpoint of the second commit", || {
        heads_of(&rig, &team, CheckpointCause::Commit)
            .iter()
            .any(|head| head.as_str() == two)
    })
    .await;
    std::fs::write(signals.join("third"), "").unwrap();
    rig.wait_for("the third commit", || signals.join("three").exists())
        .await;
    let three = git(&worktree, &["rev-parse", "HEAD"]);
    rig.wait_for("the checkpoint of the third commit", || {
        heads_of(&rig, &team, CheckpointCause::Commit)
            .iter()
            .any(|head| head.as_str() == three)
    })
    .await;

    let commits = reachable(&worktree);
    let heads = heads_of(&rig, &team, CheckpointCause::Commit);
    assert!(
        heads.iter().all(|head| commits.contains(&head.to_string())),
        "{heads:?}"
    );
    let mut distinct = heads.clone();
    distinct.dedup();
    assert_eq!(distinct, heads, "two checkpoints share a head");
    assert_eq!(heads.last().map(CommitId::as_str), Some(three.as_str()));
    // Nothing was pushed: the work is local, and its checkpoints are its only record.
    assert_eq!(rig.github.pulls().len(), 0);
    assert_eq!(
        git(&rig.origin, &["for-each-ref", "--format=%(refname)", "refs/heads"]),
        "refs/heads/main"
    );
    let snapshot = checkpoints(&rig, &team)
        .into_iter()
        .rfind(|taken| taken.cause == CheckpointCause::Commit)
        .and_then(|taken| taken.snapshot)
        .unwrap();
    let branch = git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]);
    git(
        &rig.repo,
        &["worktree", "remove", "--force", worktree.to_str().unwrap()],
    );
    git(&rig.repo, &["branch", "-D", &branch]);
    git(&rig.repo, &["merge-base", "--is-ancestor", &three, &snapshot]);
    std::fs::write(signals.join("finish"), "").unwrap();
    rig.stop().await.unwrap();
}

#[tokio::test]
async fn two_checkpoints_of_the_same_worktree_share_one_snapshot_reference() {
    let mut rig = Rig::new(&committing(), 5, false).await;
    let signals = commit_script(&rig, "commits");
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);

    rig.wait_for("two commits", || signals.join("two").exists()).await;
    let two = git(&worktree, &["rev-parse", "HEAD"]);
    rig.wait_for("the checkpoint of the second commit", || {
        heads_of(&rig, &team, CheckpointCause::Commit)
            .iter()
            .any(|head| head.as_str() == two)
    })
    .await;
    // A second step in the same tree: nothing in the worktree changed between the two.
    rig.wait_for("a second checkpoint of the same tree", || {
        checkpoints(&rig, &team)
            .iter()
            .filter(|taken| taken.head.as_ref().is_some_and(|head| head.as_str() == two))
            .count()
            >= 2
    })
    .await;
    std::fs::write(signals.join("third"), "").unwrap();
    std::fs::write(signals.join("finish"), "").unwrap();
    rig.stop().await.unwrap();

    let same: Vec<Taken> = checkpoints(&rig, &team)
        .into_iter()
        .filter(|taken| taken.head.as_ref().is_some_and(|head| head.as_str() == two))
        .collect();
    let first = same.first().unwrap();
    assert!(first.snapshot.is_some());
    assert!(same.iter().all(|taken| taken.snapshot == first.snapshot), "{same:?}");
}

#[tokio::test]
async fn a_changed_uncommitted_file_makes_a_new_snapshot_reference() {
    let mut rig = Rig::new(&committing(), 5, false).await;
    let signals = commit_script(&rig, "commits");
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);

    rig.wait_for("two commits", || signals.join("two").exists()).await;
    let two = git(&worktree, &["rev-parse", "HEAD"]);
    rig.wait_for("the checkpoint of the second commit", || {
        heads_of(&rig, &team, CheckpointCause::Commit)
            .iter()
            .any(|head| head.as_str() == two)
    })
    .await;
    let before = checkpoints(&rig, &team)
        .into_iter()
        .rfind(|taken| taken.cause == CheckpointCause::Commit)
        .and_then(|taken| taken.snapshot)
        .unwrap();
    std::fs::write(worktree.join("notes.txt"), "not committed\n").unwrap();
    std::fs::write(signals.join("third"), "").unwrap();
    rig.wait_for("the third commit", || signals.join("three").exists())
        .await;
    let three = git(&worktree, &["rev-parse", "HEAD"]);
    rig.wait_for("the checkpoint of the third commit", || {
        heads_of(&rig, &team, CheckpointCause::Commit)
            .iter()
            .any(|head| head.as_str() == three)
    })
    .await;
    std::fs::write(signals.join("finish"), "").unwrap();
    rig.stop().await.unwrap();

    let after = checkpoints(&rig, &team)
        .into_iter()
        .rfind(|taken| taken.cause == CheckpointCause::Commit)
        .and_then(|taken| taken.snapshot)
        .unwrap();
    assert_ne!(after, before);
    // The uncommitted file is in the new snapshot's tree.
    git(&rig.repo, &["cat-file", "-e", &format!("{after}:notes.txt")]);
}

#[tokio::test]
async fn a_round_leaves_a_checkpoint_after_each_of_its_steps_in_order_with_their_heads_and_passes() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-work"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, _) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let taken = checkpoints(&rig, &team);
    let causes: Vec<CheckpointCause> = taken
        .iter()
        .map(|taken| taken.cause)
        .filter(|cause| {
            !matches!(
                cause,
                CheckpointCause::Commit | CheckpointCause::RunEnded | CheckpointCause::HolderChanged
            )
        })
        .collect();
    let submitted = |pass| {
        taken
            .iter()
            .filter(|taken| taken.cause == CheckpointCause::ReviewSubmitted && taken.review_pass == Some(pass))
            .count()
    };
    // One checkpoint for each reviewer's own review, in each of the two review passes.
    assert_eq!((submitted(0), submitted(1)), (2, 2), "{taken:#?}");
    assert_eq!(
        causes,
        [
            CheckpointCause::RoundOpened,
            CheckpointCause::PullOpened,
            CheckpointCause::PullReady,
            CheckpointCause::ReviewSubmitted,
            CheckpointCause::ReviewSubmitted,
            CheckpointCause::FixPassStarted,
            CheckpointCause::PullPushed,
            CheckpointCause::ReviewSubmitted,
            CheckpointCause::ReviewSubmitted,
            CheckpointCause::PullApproved,
            CheckpointCause::PullMerged,
        ],
        "{taken:#?}"
    );
    let at = |cause| taken.iter().find(|taken| taken.cause == cause).unwrap();
    assert!(at(CheckpointCause::PullOpened).draft);
    assert!(!at(CheckpointCause::PullReady).draft);
    assert_eq!(at(CheckpointCause::FixPassStarted).fix_pass, Some(1));
    let pull = rig.github.pulls().remove(0);
    assert_eq!(
        at(CheckpointCause::PullPushed).head.as_ref().map(CommitId::as_str),
        Some(rig.remote_tip(&pull.head).as_str())
    );
    // The checkpoints of the round that ended were removed with its snapshot references.
    assert_eq!(git(&rig.repo, &["for-each-ref", "refs/agents/snapshots"]), "");
}

#[tokio::test]
async fn an_overdue_round_is_checkpointed_at_its_last_local_commit_before_the_take_over_is_recorded() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "script:commits"),
            normal("claude-2", "work"),
            investigator("inv-r1", APPROVING),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    let mut config = std::fs::read_to_string(rig.home.config_file()).unwrap();
    config.push_str("[replacement]\nagents = [\"claude-2\"]\n[limits]\nround_minutes = 120\n");
    std::fs::write(rig.home.config_file(), config).unwrap();
    let signals = commit_script(&rig, "commits");
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    rig.wait_for("two commits", || signals.join("two").exists()).await;
    let two = git(&worktree, &["rev-parse", "HEAD"]);

    rig.advance_history_days(1);
    rig.item_where("1.1", "done after the hand-over", |phase| {
        matches!(phase, ItemPhase::Done { .. })
    })
    .await;
    rig.stop().await.unwrap();

    let records = Reader::open(&rig.home.history_file())
        .unwrap()
        .team_records(&team)
        .unwrap();
    let place = |wanted: &dyn Fn(&Record) -> bool| records.iter().position(|stored| wanted(&stored.record));
    let overdue = place(&|record| {
        matches!(record, Record::Event(event)
            if matches!(&event.event, HistoryEvent::Checkpoint { cause: CheckpointCause::RoundOverdue, .. }))
    })
    .expect("a round_overdue checkpoint");
    let cut = place(&|record| matches!(record, Record::RunEnd(_))).expect("the end of the cut run");
    assert!(overdue < cut, "overdue at {overdue}, the end of the run at {cut}");
    let Record::Event(event) = &records[overdue].record else {
        panic!("not an event")
    };
    let HistoryEvent::Checkpoint { head, snapshot, .. } = &event.event else {
        panic!("not a checkpoint")
    };
    assert_eq!(head.as_ref().map(CommitId::as_str), Some(two.as_str()));
    assert!(snapshot.is_some());
}

/// Provokes `checkpoint_failed` by its real cause, a worktree that is gone when the run ends, and records it into
/// `observed`: the checkpoint is recorded with no snapshot and the daemon goes on.
pub(in crate::daemon::owner) async fn record_checkpoint_failed(observed: &mut BTreeSet<LogEvent>) {
    let mut rig = Rig::new(&committing(), 5, false).await;
    let signals = commit_script(&rig, "commits");
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    let worktree = rig.worktree(&tasks[0]);
    rig.wait_for("two commits", || signals.join("two").exists()).await;
    let branch = git(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]);
    git(
        &rig.repo,
        &["worktree", "remove", "--force", worktree.to_str().unwrap()],
    );
    git(&rig.repo, &["branch", "-D", &branch]);
    std::fs::write(signals.join("third"), "").unwrap();
    rig.wait_for("a checkpoint with no snapshot", || {
        checkpoints(&rig, &team)
            .iter()
            .any(|taken| taken.cause == CheckpointCause::RunEnded && taken.snapshot.is_none())
    })
    .await;
    let failed: Vec<_> = Reader::open(&rig.home.history_file())
        .unwrap()
        .daemon_logs()
        .unwrap()
        .into_iter()
        .filter(|entry| entry.event() == LogEvent::CheckpointFailed)
        .collect();
    assert_ne!(failed.len(), 0);
    assert_eq!(failed[0].level(), LogEvent::CheckpointFailed.level());
    let ended = checkpoints(&rig, &team)
        .into_iter()
        .find(|taken| taken.cause == CheckpointCause::RunEnded)
        .unwrap();
    assert_eq!((ended.head, ended.snapshot), (None, None));
    std::fs::write(signals.join("finish"), "").unwrap();
    rig.stop().await.unwrap();
    observed.insert(LogEvent::CheckpointFailed);
}

#[tokio::test]
async fn a_snapshot_that_fails_is_recorded_without_a_reference_and_logged_and_the_round_goes_on() {
    let mut observed = BTreeSet::new();
    record_checkpoint_failed(&mut observed).await;
    assert_eq!(observed, BTreeSet::from([LogEvent::CheckpointFailed]));
}

/// What a checkpoint says of its holder and of the pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Held {
    /// The step it follows.
    cause: CheckpointCause,
    /// The holder whose worktree it keeps.
    task: agent_core::ids::TaskId,
    /// The number of the pull request, when it had one.
    pull_request: Option<u32>,
    /// Whether the pull request was a draft.
    draft: bool,
}

/// The holder and the pull request of every checkpoint of `team`, oldest first.
fn holders(rig: &Rig, team: &TeamId) -> Vec<Held> {
    let reader = Reader::open(&rig.home.history_file()).unwrap();
    let mut held = Vec::new();
    for stored in reader.team_records(team).unwrap() {
        if let Record::Event(event) = stored.record
            && let HistoryEvent::Checkpoint {
                cause,
                task,
                pull_request,
                draft,
                ..
            } = event.event
        {
            held.push(Held {
                cause,
                task,
                pull_request: pull_request.map(|number| number.get().get()),
                draft,
            });
        }
    }
    held
}

#[tokio::test]
async fn only_the_runs_of_the_lead_end_in_a_checkpoint_and_the_pull_request_and_its_draft_are_kept_from_its_opening() {
    let mut rig = Rig::new(
        &[
            normal("claude-1", "work,fix-work"),
            investigator("inv-r1", CHANGES_ONCE),
            investigator("inv-r2", APPROVING),
        ],
        5,
        false,
    )
    .await;
    rig.start().await;
    let (team, tasks) = rig.run(&["claude-1"], ONE, MergeMode::Autopilot, None).await;
    rig.item_where("1.1", "done", |phase| matches!(phase, ItemPhase::Done { .. }))
        .await;
    rig.stop().await.unwrap();

    let held = holders(&rig, &team);

    let ended: Vec<_> = held
        .iter()
        .filter(|held| held.cause == CheckpointCause::RunEnded)
        .collect();
    assert!(!ended.is_empty(), "{held:#?}");
    // The reviewers' runs end too, and leave nothing: one checkpoint for each run of the lead.
    assert_eq!(ended.len(), usize::try_from(rig.runs(&tasks[0])).unwrap(), "{held:#?}");
    assert!(held.iter().all(|held| held.task == tasks[0]), "{held:#?}");
    let number = rig.github.pulls().remove(0).number;
    let opened = held
        .iter()
        .position(|held| held.cause == CheckpointCause::PullOpened)
        .unwrap();
    let ready = held
        .iter()
        .position(|held| held.cause == CheckpointCause::PullReady)
        .unwrap();
    for held in &held[opened..ready] {
        assert_eq!((held.pull_request, held.draft), (Some(number), true), "{held:#?}");
    }
    for held in &held[ready..] {
        assert_eq!((held.pull_request, held.draft), (Some(number), false), "{held:#?}");
    }
}
