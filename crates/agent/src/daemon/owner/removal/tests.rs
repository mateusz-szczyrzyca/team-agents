//! `agent remove` in the owner: what a removal of an idle team takes away, what it bundles and what it keeps. The
//! team's worktrees and branches are real git worktrees of a scratch checkout; no GitHub and no CLI is involved.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command as Process;
use std::str::FromStr;

use crate::client::ClientError;
use agent_core::config::AbsolutePath;
use agent_core::ids::BranchName;
use agent_core::protocol::{Client, Command, FailureKind, RemoveRequest, Reply};
use agent_core::state::{Crew, Member, MergeMode, State, TaskListText, TaskRef, Team, TeamMembers};
use agent_core::status::TaskStatus;

use super::super::fixture::{Daemon, pair_state};
use super::super::start::work_branch;
use crate::fixtures::{at, member, normal, repo, task, team_id};

/// Runs git in `dir` with a fixed identity and returns its standard output.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Process::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// The absolute path of `path`, as the reply and the state name it.
fn abs(path: &Path) -> AbsolutePath {
    AbsolutePath::from_str(path.to_str().unwrap()).unwrap()
}

/// A checkout of the repository on `main` with one commit.
fn checkout() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("README.md"), "read me\n").unwrap();
    git(dir.path(), &["add", "README.md"]);
    git(dir.path(), &["commit", "-q", "-m", "first"]);
    dir
}

/// The branch of the member of `task_number` in `state`, as the owner names it.
fn branch_of(state: &State, task_number: u32) -> BranchName {
    let TaskRef { team, member, .. } = state.find_task(&task(task_number)).unwrap();
    work_branch(team, member).unwrap()
}

/// A worktree of `checkout` at `path` on a new branch `branch` made from `main`.
fn worktree(checkout: &Path, path: &Path, branch: &BranchName) {
    git(
        checkout,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            branch.as_str(),
            path.to_str().unwrap(),
            "main",
        ],
    );
}

/// The state of [`pair_state`] with a second team, team 1 of tasks 3 and 4, which shares the agent `claude-1` with
/// team 0 and is not removed.
fn two_teams() -> State {
    let crew = Crew::team(TeamMembers {
        coordinator: member(3, normal("claude-1")),
        members: vec![Member::new(task(4), normal("claude-2"))],
        workers: vec![],
    })
    .unwrap();
    let team = Team::new(
        team_id(1),
        repo(),
        at(0),
        MergeMode::Autopilot,
        TaskListText::new("Fix the typo in the README.\n".to_owned()).unwrap(),
        crew,
    );
    let mut state = pair_state();
    state.insert_team(team).unwrap();
    state
}

/// A team's worktrees: task `first` has a commit of its own on its branch, task `first + 1` has none and one changed
/// tracked file. Each task has a directory with a file in it.
fn make_team_files(daemon: &Daemon, checkout: &Path, state: &State, first: u32) -> (BranchName, BranchName) {
    let first_branch = branch_of(state, first);
    let second_branch = branch_of(state, first + 1);
    let first_dir = daemon.home.worktree_dir(&repo(), &task(first));
    let second_dir = daemon.home.worktree_dir(&repo(), &task(first + 1));
    std::fs::create_dir_all(first_dir.parent().unwrap()).unwrap();
    worktree(checkout, &first_dir, &first_branch);
    worktree(checkout, &second_dir, &second_branch);
    std::fs::write(first_dir.join("work.txt"), "unmerged\n").unwrap();
    git(&first_dir, &["add", "work.txt"]);
    git(&first_dir, &["commit", "-q", "-m", "unmerged work"]);
    std::fs::write(second_dir.join("README.md"), "changed and not committed\n").unwrap();
    for number in [first, first + 1] {
        let dir = daemon.home.task_dir(&task(number));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("run.log"), "a run\n").unwrap();
    }
    (first_branch, second_branch)
}

#[tokio::test]
async fn removing_a_team_bundles_its_unmerged_branch_and_takes_its_files_away() {
    let state = pair_state();
    let daemon = Daemon::new(&state, None);
    let dir = checkout();
    let (unmerged, merged) = make_team_files(&daemon, dir.path(), &state, 1);
    let tip = git(dir.path(), &["rev-parse", unmerged.as_str()]).trim().to_owned();
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let removal = owner
        .remove_request(
            &Client::Cli,
            RemoveRequest {
                team: team_id(0),
                force: false,
            },
        )
        .await
        .unwrap()
        .unwrap();

    let bundle = daemon.home.removed_bundle(&team_id(0));
    assert_eq!(removal.bundle, Some(abs(&bundle)));
    assert_eq!(removal.bundled, vec![unmerged.clone()]);
    let deleted: BTreeSet<BranchName> = removal.branches.iter().cloned().collect();
    assert_eq!(deleted, [unmerged.clone(), merged.clone()].into_iter().collect());
    assert!(removal.freed_bytes > 0);
    assert!(
        removal
            .dirty_worktrees
            .contains(&abs(&daemon.home.worktree_dir(&repo(), &task(2))))
    );
    let heads = git(dir.path(), &["bundle", "list-heads", bundle.to_str().unwrap()]);
    assert!(heads.contains(&format!("{tip} refs/heads/{unmerged}")), "{heads}");
    for number in [1, 2] {
        assert!(!daemon.home.worktree_dir(&repo(), &task(number)).exists());
        assert!(!daemon.home.task_dir(&task(number)).exists());
    }
    assert_eq!(git(dir.path(), &["branch", "--list", unmerged.as_str()]).trim(), "");
    assert_eq!(git(dir.path(), &["branch", "--list", merged.as_str()]).trim(), "");
    assert!(daemon.state().team(&team_id(0)).is_none());
}

#[tokio::test]
async fn removing_a_team_that_is_not_there_is_refused_and_changes_nothing() {
    let state = pair_state();
    let daemon = Daemon::new(&state, None);
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let failure = owner
        .remove_request(
            &Client::Cli,
            RemoveRequest {
                team: team_id(7),
                force: false,
            },
        )
        .await
        .unwrap()
        .unwrap_err();

    assert_eq!(failure.kind, FailureKind::NotFound);
    assert!(daemon.state().team(&team_id(0)).is_some());
}

#[tokio::test]
async fn the_other_team_keeps_its_worktrees_branches_and_files() {
    let state = two_teams();
    let daemon = Daemon::new(&state, None);
    let dir = checkout();
    make_team_files(&daemon, dir.path(), &state, 1);
    let (kept_branch, _) = make_team_files(&daemon, dir.path(), &state, 3);
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner
        .remove_request(
            &Client::Cli,
            RemoveRequest {
                team: team_id(0),
                force: false,
            },
        )
        .await
        .unwrap()
        .unwrap();

    assert!(daemon.home.worktree_dir(&repo(), &task(3)).join("work.txt").is_file());
    assert!(daemon.home.task_dir(&task(3)).join("run.log").is_file());
    assert_ne!(git(dir.path(), &["branch", "--list", kept_branch.as_str()]).trim(), "");
    assert!(daemon.state().team(&team_id(1)).is_some());
    assert!(daemon.state().team(&team_id(0)).is_none());
}

#[tokio::test]
async fn a_removed_team_stays_removed_when_the_state_is_read_again() {
    let state = pair_state();
    let daemon = Daemon::new(&state, None);
    let dir = checkout();
    make_team_files(&daemon, dir.path(), &state, 1);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    owner
        .remove_request(
            &Client::Cli,
            RemoveRequest {
                team: team_id(0),
                force: false,
            },
        )
        .await
        .unwrap()
        .unwrap();
    drop(owner);

    let again = daemon.state();

    assert!(again.team(&team_id(0)).is_none());
    assert!(again.find_task(&task(1)).is_none());
}

#[tokio::test]
async fn a_branch_of_the_team_that_the_state_no_longer_names_is_bundled_and_deleted_too() {
    let state = pair_state();
    let daemon = Daemon::new(&state, None);
    let dir = checkout();
    make_team_files(&daemon, dir.path(), &state, 1);
    // A split part's branch, from before the state named it: its name carries the team's task ID, and it has a commit.
    let historical = BranchName::from_str(&format!("split/{}-part-a", task(1))).unwrap();
    let scratch = dir.path().join("scratch");
    git(
        dir.path(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            historical.as_str(),
            scratch.to_str().unwrap(),
            "main",
        ],
    );
    std::fs::write(scratch.join("part.txt"), "a part\n").unwrap();
    git(&scratch, &["add", "part.txt"]);
    git(&scratch, &["commit", "-q", "-m", "part a"]);
    let tip = git(dir.path(), &["rev-parse", historical.as_str()]).trim().to_owned();
    git(
        dir.path(),
        &["worktree", "remove", "--force", scratch.to_str().unwrap()],
    );
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let removal = owner
        .remove_request(
            &Client::Cli,
            RemoveRequest {
                team: team_id(0),
                force: false,
            },
        )
        .await
        .unwrap()
        .unwrap();

    assert!(removal.bundled.contains(&historical));
    let bundle = daemon.home.removed_bundle(&team_id(0));
    let heads = git(dir.path(), &["bundle", "list-heads", bundle.to_str().unwrap()]);
    assert!(heads.contains(&format!("{tip} refs/heads/{historical}")), "{heads}");
    assert_eq!(git(dir.path(), &["branch", "--list", historical.as_str()]).trim(), "");
}

/// A daemon whose two tasks, a coordinator and a member of team 0, run CLIs that never end by themselves.
async fn running_pair() -> Daemon {
    let mut daemon = Daemon::new(&pair_state(), None);
    daemon.scenario(1, "busy");
    daemon.scenario(2, "busy");
    daemon.start().await;
    daemon.status_is(1, TaskStatus::Running).await;
    daemon.status_is(2, TaskStatus::Running).await;
    daemon
}

#[tokio::test]
async fn a_team_with_a_run_going_is_refused_without_force_and_keeps_its_runs() {
    let mut daemon = running_pair().await;

    let refused = daemon
        .ask(Command::Remove(RemoveRequest {
            team: team_id(0),
            force: false,
        }))
        .await;

    match refused {
        Err(ClientError::Refused(failure)) => assert_eq!(failure.kind, FailureKind::Conflict),
        other => panic!("a refusal was expected: {other:?}"),
    }
    daemon.status_is(1, TaskStatus::Running).await;
    assert!(daemon.state().team(&team_id(0)).is_some());
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_team_with_a_run_going_is_removed_with_force_after_its_runs_ended() {
    let mut daemon = running_pair().await;

    let reply = daemon
        .ask(Command::Remove(RemoveRequest {
            team: team_id(0),
            force: true,
        }))
        .await
        .unwrap();

    let Reply::Removed { removal } = reply else {
        panic!("a removal was expected: {reply:?}");
    };
    assert_eq!(removal.stopped.len(), 2);
    assert!(daemon.state().team(&team_id(0)).is_none());
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_removal_over_the_socket_of_an_unknown_team_is_refused() {
    let state = pair_state();
    let mut daemon = Daemon::new(&state, None);
    daemon.start().await;

    let refused = daemon
        .ask(Command::Remove(RemoveRequest {
            team: team_id(7),
            force: false,
        }))
        .await;

    match refused {
        Err(ClientError::Refused(failure)) => assert_eq!(failure.kind, FailureKind::NotFound),
        other => panic!("a refusal was expected: {other:?}"),
    }
    daemon.stop().await.unwrap();
}

#[tokio::test]
async fn a_commit_only_a_detached_worktree_holds_is_fetched_back_from_the_bundle_after_the_removal() {
    let state = pair_state();
    let daemon = Daemon::new(&state, None);
    let dir = checkout();
    make_team_files(&daemon, dir.path(), &state, 1);
    // The reviewer's detached HEAD of task 2: a commit that no branch holds and that only the worktree points at.
    let detached = daemon.home.worktree_dir(&repo(), &task(2));
    std::fs::remove_dir_all(&detached).unwrap();
    git(dir.path(), &["worktree", "prune"]);
    git(
        dir.path(),
        &["worktree", "add", "-q", "--detach", detached.to_str().unwrap(), "main"],
    );
    std::fs::write(detached.join("review.txt"), "a local-only commit\n").unwrap();
    git(&detached, &["add", "review.txt"]);
    git(&detached, &["commit", "-q", "-m", "local only"]);
    let head = git(&detached, &["rev-parse", "HEAD"]).trim().to_owned();
    let (mut owner, _sinks) = daemon.bare_owner().await;

    owner
        .remove_request(
            &Client::Cli,
            RemoveRequest {
                team: team_id(0),
                force: false,
            },
        )
        .await
        .unwrap()
        .unwrap();

    assert!(!detached.exists());
    let bundle = daemon.home.removed_bundle(&team_id(0));
    let other = tempfile::tempdir().unwrap();
    git(other.path(), &["init", "-q", "-b", "main"]);
    git(
        other.path(),
        &[
            "fetch",
            "-q",
            bundle.to_str().unwrap(),
            &format!("refs/agent-removed/{}:refs/heads/back", task(2)),
        ],
    );
    assert_eq!(git(other.path(), &["rev-parse", "back"]).trim(), head);
}

#[tokio::test]
async fn a_bundle_that_cannot_be_written_leaves_the_team_and_its_files_intact() {
    let state = pair_state();
    let daemon = Daemon::new(&state, None);
    let dir = checkout();
    let (unmerged, _) = make_team_files(&daemon, dir.path(), &state, 1);
    // A file where the directory of the bundles must be makes the bundle impossible to write.
    std::fs::write(daemon.home.root().join("removed"), "not a directory\n").unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let failure = owner
        .remove_request(
            &Client::Cli,
            RemoveRequest {
                team: team_id(0),
                force: false,
            },
        )
        .await
        .unwrap()
        .unwrap_err();

    assert_eq!(failure.kind, FailureKind::Internal);
    assert!(daemon.state().team(&team_id(0)).is_some());
    assert!(daemon.home.worktree_dir(&repo(), &task(1)).join("work.txt").is_file());
    assert!(daemon.home.task_dir(&task(1)).join("run.log").is_file());
    assert_ne!(git(dir.path(), &["branch", "--list", unmerged.as_str()]).trim(), "");
}

#[tokio::test]
async fn a_worktree_git_cannot_remove_is_kept_and_its_bytes_are_not_counted_as_freed() {
    let state = pair_state();
    let daemon = Daemon::new(&state, None);
    let dir = checkout();
    make_team_files(&daemon, dir.path(), &state, 1);
    let locked = daemon.home.worktree_dir(&repo(), &task(2));
    git(dir.path(), &["worktree", "lock", locked.to_str().unwrap()]);
    let sized = |path: &Path| agent_runtime::disk::directory_size(path).unwrap();
    let expected = sized(&daemon.home.worktree_dir(&repo(), &task(1)))
        + sized(&daemon.home.task_dir(&task(1)))
        + sized(&daemon.home.task_dir(&task(2)));
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let removal = owner
        .remove_request(
            &Client::Cli,
            RemoveRequest {
                team: team_id(0),
                force: false,
            },
        )
        .await
        .unwrap()
        .unwrap();

    assert_eq!(removal.freed_bytes, expected);
    assert!(locked.join("README.md").is_file());
    assert!(
        removal.left_behind.iter().any(|left| left.contains(task(2).as_str())),
        "{:?}",
        removal.left_behind
    );
    assert!(daemon.state().team(&team_id(0)).is_none());
}

#[tokio::test]
async fn a_dirty_worktree_git_cannot_remove_is_not_named_as_removed() {
    let state = pair_state();
    let daemon = Daemon::new(&state, None);
    let dir = checkout();
    make_team_files(&daemon, dir.path(), &state, 1);
    let locked = daemon.home.worktree_dir(&repo(), &task(2));
    git(dir.path(), &["worktree", "lock", locked.to_str().unwrap()]);
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let removal = owner
        .remove_request(
            &Client::Cli,
            RemoveRequest {
                team: team_id(0),
                force: false,
            },
        )
        .await
        .unwrap()
        .unwrap();

    assert!(
        removal.dirty_worktrees.contains(&abs(&locked)),
        "{:?}",
        removal.dirty_worktrees
    );
    assert!(locked.is_dir());
    assert!(
        removal.left_behind.iter().any(|left| left.contains(task(2).as_str())),
        "{:?}",
        removal.left_behind
    );
}

#[tokio::test]
async fn a_worktree_with_only_untracked_files_is_not_named_as_having_uncommitted_changes() {
    let state = pair_state();
    let daemon = Daemon::new(&state, None);
    let dir = checkout();
    make_team_files(&daemon, dir.path(), &state, 1);
    // make_team_files changes a tracked file of task 2; this worktree has an untracked file and nothing tracked.
    let second = daemon.home.worktree_dir(&repo(), &task(2));
    git(&second, &["checkout", "-q", "--", "README.md"]);
    std::fs::write(second.join("notes.txt"), "untracked\n").unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let removal = owner
        .remove_request(
            &Client::Cli,
            RemoveRequest {
                team: team_id(0),
                force: false,
            },
        )
        .await
        .unwrap()
        .unwrap();

    assert!(removal.dirty_worktrees.is_empty(), "{:?}", removal.dirty_worktrees);
}

#[tokio::test]
async fn removal_without_surviving_worktrees_refuses_and_preserves_state_and_task_files() {
    let state = pair_state();
    let daemon = Daemon::new(&state, None);
    let task_dir = daemon.home.task_dir(&task(1));
    std::fs::create_dir_all(&task_dir).unwrap();
    std::fs::write(task_dir.join("run.log"), "keep me").unwrap();
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let failure = owner
        .remove_request(
            &Client::Cli,
            RemoveRequest {
                team: team_id(0),
                force: false,
            },
        )
        .await
        .unwrap()
        .expect_err("no repository handle must refuse removal");
    assert_eq!(failure.kind, FailureKind::Refused);
    for number in [1, 2] {
        assert!(failure.message.contains(branch_of(&state, number).as_str()));
    }
    assert!(failure.message.contains("keep one worktree"));
    assert_eq!(std::fs::read_to_string(task_dir.join("run.log")).unwrap(), "keep me");
    assert_eq!(owner.parts.state, state);
    assert_eq!(daemon.state(), state);
    assert!(!daemon.home.removed_bundle(&team_id(0)).exists());
}

#[tokio::test]
async fn failed_removal_state_save_keeps_the_live_team_and_its_files() {
    let state = pair_state();
    let daemon = Daemon::new(&state, None);
    let dir = checkout();
    make_team_files(&daemon, dir.path(), &state, 1);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    std::fs::remove_file(daemon.home.state_file()).unwrap();
    std::fs::create_dir(daemon.home.state_file()).unwrap();

    let result = owner
        .remove_request(
            &Client::Cli,
            RemoveRequest {
                team: team_id(0),
                force: false,
            },
        )
        .await;

    assert!(matches!(result, Err(super::OwnerError::Store { .. })));
    assert_eq!(owner.parts.state, state);
    assert!(daemon.home.task_dir(&task(1)).join("run.log").is_file());
    assert!(daemon.home.worktree_dir(&repo(), &task(1)).join("work.txt").is_file());
}

#[tokio::test]
async fn removal_reports_persisted_wake_budgets_and_bus_cleanup_limit() {
    use agent_core::state::WakeDelivery;
    use std::num::NonZeroU64;

    let mut state = pair_state();
    state.record_wake_delivery(
        task(1),
        Some(WakeDelivery::Pending {
            message: NonZeroU64::new(91),
            count: 2,
        }),
    );
    state.record_wake_delivery(
        task(2),
        Some(WakeDelivery::Delivered {
            message: None,
            count: 4,
        }),
    );
    let daemon = Daemon::new(&state, None);
    let dir = checkout();
    make_team_files(&daemon, dir.path(), &state, 1);
    let (mut owner, _sinks) = daemon.bare_owner().await;

    let removed = owner
        .remove_request(
            &Client::Cli,
            RemoveRequest {
                team: team_id(0),
                force: false,
            },
        )
        .await
        .unwrap()
        .unwrap();

    for (number, message, count, phase) in [(1, "91", 2, "pending"), (2, "unknown", 4, "delivered")] {
        let expected = format!(
            "removed wake budget: task {}, message {message}, count {count}, state {phase}, source {}",
            task(number),
            daemon.home.state_file().display()
        );
        assert!(removed.left_behind.contains(&expected), "{:?}", removed.left_behind);
        assert_eq!(owner.parts.state.wake_delivery(&task(number)), None);
    }
    assert!(
        removed
            .left_behind
            .iter()
            .any(|line| line.contains("bus itself was neither read for wake-ups nor cleaned"))
    );
}

#[tokio::test]
async fn removal_refuses_a_gate_that_outlives_its_agent_run_without_force() {
    let state = pair_state();
    let daemon = Daemon::new(&state, None);
    let dir = checkout();
    make_team_files(&daemon, dir.path(), &state, 1);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let (queue, mut waiting) = crate::daemon::gate::GateQueue::new();
    owner = owner.with_gate(queue);
    let (reply, _answer) = tokio::sync::oneshot::channel();
    owner.gate_request(agent_core::protocol::GateRequest { task: task(1) }, reply);
    let job = waiting.recv().await.unwrap();

    let result = owner
        .remove_request(
            &Client::Cli,
            RemoveRequest {
                team: team_id(0),
                force: false,
            },
        )
        .await
        .unwrap();

    assert!(matches!(
        result,
        Err(agent_core::protocol::Failure {
            kind: FailureKind::Conflict,
            ..
        })
    ));
    assert!(!job.cancel.is_cancelled());
    assert_eq!(owner.parts.state, state);
    assert!(daemon.home.task_dir(&task(1)).join("run.log").is_file());
}

#[tokio::test]
async fn forced_removal_waits_for_the_gate_to_finish_before_deleting_files() {
    let state = pair_state();
    let daemon = Daemon::new(&state, None);
    let dir = checkout();
    make_team_files(&daemon, dir.path(), &state, 1);
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let (queue, mut waiting) = crate::daemon::gate::GateQueue::new();
    owner = owner.with_gate(queue);
    let (reply, answer) = tokio::sync::oneshot::channel();
    owner.gate_request(agent_core::protocol::GateRequest { task: task(1) }, reply);
    let job = waiting.recv().await.unwrap();
    let mut removal = Box::pin(owner.remove_request(
        &Client::Cli,
        RemoveRequest {
            team: team_id(0),
            force: true,
        },
    ));
    tokio::select! {
        result = &mut removal => panic!("removal completed before cancelling and joining the gate: {result:?}"),
        () = job.cancel.cancelled() => {},
    }
    assert!(daemon.home.task_dir(&task(1)).join("run.log").is_file());
    assert!(daemon.home.worktree_dir(&repo(), &task(1)).join("work.txt").is_file());
    job.reply.send(Ok(Reply::Applied { tasks: vec![task(1)] })).unwrap();
    let removed = removal.await.unwrap().unwrap();
    assert_eq!(answer.await.unwrap().unwrap(), Reply::Applied { tasks: vec![task(1)] });
    assert_eq!(removed.stopped, vec![task(1)]);
    assert!(!daemon.home.task_dir(&task(1)).exists());
}

#[tokio::test]
async fn removal_refuses_parked_checks_or_joins_them_before_forced_cleanup() {
    use agent_core::decide::outcome::{Classified, ExitStatus, RunFinish, StoppedBy};
    use agent_core::state::{NewRun, RunEnd};
    use agent_core::status::RunOutcome;
    use agent_runtime::run::{FinishedRun, Progress};

    for force in [false, true] {
        let state = pair_state();
        let daemon = Daemon::new(&state, None);
        let dir = checkout();
        make_team_files(&daemon, dir.path(), &state, 1);
        let worktree = daemon.home.worktree_dir(&repo(), &task(1));
        std::fs::create_dir_all(worktree.join(".agent")).unwrap();
        std::fs::write(
            worktree.join(".agent/plan.md"),
            "Finish it.\n\n1. Work\n[x] 1.1 Build\n    Check: /bin/true\n",
        )
        .unwrap();
        let (mut owner, _sinks) = daemon.bare_owner().await;
        let round = agent_core::ids::RoundId::derive(&team_id(0), 1);
        owner
            .parts
            .state
            .team_mut(&team_id(0))
            .unwrap()
            .open_round(round.clone(), None, at(0))
            .unwrap();
        owner
            .parts
            .state
            .begin_run(
                &task(1),
                NewRun {
                    id: "run-20261122-060000-abcd".parse().unwrap(),
                    at: at(0),
                    session: None,
                    rounds: vec![round],
                },
            )
            .unwrap();
        owner
            .parts
            .state
            .find_task_mut(&task(1))
            .unwrap()
            .1
            .transition(TaskStatus::Running)
            .unwrap();
        owner
            .finished(
                &task(1),
                FinishedRun {
                    classified: Classified {
                        end: RunEnd::Reported {
                            outcome: RunOutcome::Done,
                        },
                        pause: None,
                    },
                    finish: RunFinish {
                        exit: ExitStatus::Code(0),
                        stopped_by: StoppedBy::Nobody,
                    },
                    progress: Progress::Made,
                    session: None,
                    final_message: Some("STATUS: DONE".to_owned()),
                },
            )
            .await
            .unwrap();
        assert!(owner.parked.contains_key(&task(1)));
        let result = owner
            .remove_request(
                &Client::Cli,
                RemoveRequest {
                    team: team_id(0),
                    force,
                },
            )
            .await
            .unwrap();
        if force {
            assert_eq!(result.unwrap().stopped, vec![task(1)]);
            assert!(owner.parked.is_empty());
            assert!(
                owner.checks.is_empty(),
                "check tasks must be joined before their files go"
            );
            assert!(!worktree.exists());
        } else {
            assert!(matches!(
                result,
                Err(agent_core::protocol::Failure {
                    kind: FailureKind::Conflict,
                    ..
                })
            ));
            assert!(worktree.is_dir());
            owner.halt_parked_check(&task(1)).await.unwrap();
            while let Some(joined) = owner.checks.join_next_with_id().await {
                owner.check_batch_done(joined).await.unwrap();
            }
        }
    }
}
