//! `agent remove` in the owner: what a removal of an idle team takes away, what it bundles and what it keeps. The
//! team's worktrees and branches are real git worktrees of a scratch checkout; no GitHub and no CLI is involved.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;

use agent_core::config::AbsolutePath;
use agent_core::ids::BranchName;
use agent_core::protocol::{Client, FailureKind, RemoveRequest};
use agent_core::state::{Crew, Member, MergeMode, State, TaskListText, TaskRef, Team, TeamMembers};

use super::super::fixture::{Daemon, pair_state};
use super::super::start::work_branch;
use crate::fixtures::{at, member, normal, repo, task, team_id};

/// Runs git in `dir` with a fixed identity and returns its standard output.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
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
    assert!(
        git(dir.path(), &["branch", "--list", unmerged.as_str()])
            .trim()
            .is_empty()
    );
    assert!(
        git(dir.path(), &["branch", "--list", merged.as_str()])
            .trim()
            .is_empty()
    );
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
    assert!(
        !git(dir.path(), &["branch", "--list", kept_branch.as_str()])
            .trim()
            .is_empty()
    );
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
