//! The checkpoints of a solo run through a daemon in a temporary home: a stand-in for the CLI that commits on demand, the
//! real `agentbus`, a clock the test moves, and a repository whose worktree is linked to a main repository the test can look
//! into after the worktree is removed.

use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::{Duration, Instant};

use agent_core::history::{CheckpointCause, CheckpointJob};
use agent_core::history::{HistoryEvent, Record};
use agent_core::kinds::Tier;
use agent_core::log::LogEvent;
use agent_core::protocol::{Command, StatusRequest};
use agent_core::state::CommitId;
use agent_runtime::history::Reader;

use super::super::fixture::{Daemon, PATIENCE, agentbus_binary};
use crate::fixtures::solo_state;

/// What a checkpoint says that the tests look at.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Taken {
    /// The step it follows.
    cause: CheckpointCause,
    /// The tip of the worktree's branch.
    head: Option<CommitId>,
    /// The reference of its snapshot.
    snapshot: Option<String>,
}

/// Runs `git` in `dir` as a fixture author and returns what it prints.
fn git(dir: &Path, args: &[&str]) -> String {
    let done = Process::new("git")
        .current_dir(dir)
        .env("GIT_AUTHOR_DATE", "2026-11-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-11-01T00:00:00Z")
        .args(["-c", "user.name=Fixture", "-c", "user.email=fixture@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
    String::from_utf8(done.stdout).unwrap().trim().to_owned()
}

/// A main repository next to the home, and the worktree of task 1 linked to it, whose scenario is `scenario`. Returns the
/// main repository and the worktree.
fn linked_worktree(daemon: &Daemon, scenario: &str) -> (PathBuf, PathBuf) {
    let main = daemon.home.root().join("main-repo");
    std::fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    git(&main, &["commit", "-q", "--allow-empty", "-m", "Start"]);
    std::fs::write(main.join(".git/info/exclude"), ".fake-*\n").unwrap();
    let worktree = daemon
        .home
        .worktree_dir(&crate::fixtures::repo(), &crate::fixtures::task(1));
    std::fs::create_dir_all(worktree.parent().unwrap()).unwrap();
    git(
        &main,
        &["worktree", "add", "-q", "-b", "task-1", worktree.to_str().unwrap()],
    );
    assert_eq!(daemon.scenario(1, scenario), worktree);
    (main, worktree)
}

/// The checkpoints of the history, oldest first, once the writer has them.
fn checkpoints(daemon: &Daemon) -> Vec<Taken> {
    let reader = Reader::open(&daemon.home.history_file()).unwrap();
    let mut taken = Vec::new();
    for team in reader.teams().unwrap() {
        for record in reader.team_records(&team.value.team).unwrap() {
            if let Record::Event(event) = record.record
                && let HistoryEvent::Checkpoint {
                    cause,
                    head,
                    snapshot,
                    job,
                    tier,
                    ..
                } = event.event
            {
                assert_eq!((job, tier), (CheckpointJob::Item, Tier::Normal));
                taken.push(Taken { cause, head, snapshot });
            }
        }
    }
    taken
}

/// The causes of the checkpoints so far.
fn causes(daemon: &Daemon) -> Vec<CheckpointCause> {
    checkpoints(daemon).into_iter().map(|taken| taken.cause).collect()
}

/// Waits until `ready` holds, polling; it never sleeps for a fixed time.
async fn until(what: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while !ready() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Moves the clock past the minute between two looks at the worktree and waits for a checkpoint of `cause` to be the
/// `count`-th one of its kind.
async fn look_until(daemon: &Daemon, cause: CheckpointCause, count: usize) {
    daemon.clock.advance(time::Duration::minutes(2));
    for _ in 0..3 {
        daemon
            .ask(Command::Status(StatusRequest { target: None }))
            .await
            .unwrap();
    }
    until(&format!("checkpoint {count} of {cause}"), || {
        causes(daemon).iter().filter(|found| **found == cause).count() >= count
    })
    .await;
}

/// Lets the stand-in take step `n`.
fn step(worktree: &Path, n: u32) {
    std::fs::write(worktree.join(format!(".fake-step-{n}")), "").unwrap();
}

/// The commits of the worktree's branch, oldest first.
fn commits(worktree: &Path) -> Vec<CommitId> {
    git(worktree, &["rev-list", "--reverse", "HEAD"])
        .lines()
        .map(|line| line.parse().unwrap())
        .collect()
}

#[tokio::test]
async fn a_commit_is_a_checkpoint_pushed_or_not_and_its_snapshot_outlives_the_worktree() {
    let mut daemon = Daemon::new(&solo_state(), Some(agentbus_binary()));
    let (main, worktree) = linked_worktree(&daemon, "commit-steps");
    daemon.start().await;
    until("the run to start", || worktree.join(".fake-args").exists()).await;

    step(&worktree, 1);
    step(&worktree, 2);
    until("two commits", || commits(&worktree).len() == 3).await;
    look_until(&daemon, CheckpointCause::Commit, 1).await;
    step(&worktree, 3);
    until("the third commit", || commits(&worktree).len() == 4).await;
    look_until(&daemon, CheckpointCause::Commit, 2).await;
    std::fs::write(worktree.join(".fake-continue"), "").unwrap();
    until("the checkpoint of the run's end", || {
        causes(&daemon).contains(&CheckpointCause::RunEnded)
    })
    .await;
    daemon.stop().await.unwrap();

    let all = commits(&worktree);
    let taken: Vec<Taken> = checkpoints(&daemon)
        .into_iter()
        .filter(|taken| taken.cause == CheckpointCause::Commit)
        .collect();
    let heads: Vec<Option<CommitId>> = taken.iter().map(|taken| taken.head.clone()).collect();
    assert_eq!(heads, vec![all.get(2).cloned(), all.get(3).cloned()]);
    let last = taken.last().unwrap().snapshot.clone().unwrap();
    git(&main, &["worktree", "remove", "--force", worktree.to_str().unwrap()]);
    git(&main, &["branch", "-D", "task-1"]);
    git(&main, &["merge-base", "--is-ancestor", &all[3].to_string(), &last]);
    assert_eq!(git(&main, &["branch", "--remotes"]), "");
}

#[tokio::test]
async fn two_checkpoints_of_the_same_worktree_share_one_snapshot_reference() {
    let mut daemon = Daemon::new(&solo_state(), Some(agentbus_binary()));
    let (_main, worktree) = linked_worktree(&daemon, "commit-steps");
    daemon.start().await;
    until("the run to start", || worktree.join(".fake-args").exists()).await;
    step(&worktree, 1);
    until("the commit", || commits(&worktree).len() == 2).await;
    look_until(&daemon, CheckpointCause::Commit, 1).await;

    std::fs::write(worktree.join(".fake-continue"), "").unwrap();
    until("the checkpoint of the run's end", || {
        causes(&daemon).contains(&CheckpointCause::RunEnded)
    })
    .await;
    daemon.stop().await.unwrap();

    let taken = checkpoints(&daemon);
    let commit = taken
        .iter()
        .find(|taken| taken.cause == CheckpointCause::Commit)
        .unwrap();
    let ended = taken
        .iter()
        .find(|taken| taken.cause == CheckpointCause::RunEnded)
        .unwrap();
    assert!(commit.snapshot.is_some());
    assert_eq!(ended.snapshot, commit.snapshot);
}

#[tokio::test]
async fn a_changed_uncommitted_file_makes_a_new_snapshot_reference() {
    let mut daemon = Daemon::new(&solo_state(), Some(agentbus_binary()));
    let (_main, worktree) = linked_worktree(&daemon, "commit-steps");
    daemon.start().await;
    until("the run to start", || worktree.join(".fake-args").exists()).await;
    step(&worktree, 1);
    until("the commit", || commits(&worktree).len() == 2).await;
    look_until(&daemon, CheckpointCause::Commit, 1).await;

    std::fs::write(worktree.join("notes.txt"), "not committed\n").unwrap();
    std::fs::write(worktree.join(".fake-continue"), "").unwrap();
    until("the checkpoint of the run's end", || {
        causes(&daemon).contains(&CheckpointCause::RunEnded)
    })
    .await;
    daemon.stop().await.unwrap();

    let taken = checkpoints(&daemon);
    let commit = taken
        .iter()
        .find(|taken| taken.cause == CheckpointCause::Commit)
        .unwrap();
    let ended = taken
        .iter()
        .find(|taken| taken.cause == CheckpointCause::RunEnded)
        .unwrap();
    assert_eq!(ended.head, commit.head);
    assert_ne!(ended.snapshot, commit.snapshot);
    assert!(ended.snapshot.is_some());
}

#[tokio::test]
async fn a_snapshot_that_fails_is_recorded_without_a_reference_and_the_round_goes_on() {
    let mut daemon = Daemon::new(&solo_state(), Some(agentbus_binary()));
    let worktree = daemon.scenario(1, "done");
    daemon.start().await;
    until("the checkpoint of the run's end", || {
        causes(&daemon).contains(&CheckpointCause::RunEnded)
    })
    .await;
    daemon.stop().await.unwrap();

    let ended: Vec<Taken> = checkpoints(&daemon)
        .into_iter()
        .filter(|taken| taken.cause == CheckpointCause::RunEnded)
        .collect();
    assert_eq!(
        ended,
        vec![Taken {
            cause: CheckpointCause::RunEnded,
            head: None,
            snapshot: None,
        }]
    );
    let failed: Vec<_> = Reader::open(&daemon.home.history_file())
        .unwrap()
        .daemon_logs()
        .unwrap()
        .into_iter()
        .filter(|entry| entry.event() == LogEvent::CheckpointFailed)
        .collect();
    assert!(!failed.is_empty());
    assert!(!worktree.join(".git").exists());
}
