//! A request's workspace: made at the requester's committed end in the requester's repository, on a branch of its own,
//! isolated from the requester's uncommitted files and from any crew, and released when its result is settled. Real
//! repositories in temporary directories, isolated from the configuration of the machine.

use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use agent_core::config::{AbsolutePath, RepoName};
use agent_core::ids::TaskId;
use agent_core::state::CommitId;

use crate::agentbus::RequestId;
use crate::git::{BringOver, CommitIdentity, Git};
use crate::request_workspace::{
    RequestWorkspace, Stage, WorkspaceError, branch, path_of, prepare as prepare_workspace, read_each,
    release as release_workspace, save,
};
use crate::store::Home;

const ROUND: &str = "round-20261124-090000-aaaa";
const REQUESTER: &str = "task-20261124-090000-bbbb";
const WORKER: &str = "task-20261124-090000-cccc";
const REPO: &str = "example-repo";

fn task(text: &str) -> TaskId {
    text.parse().expect("a task id")
}

fn request(number: u64) -> RequestId {
    RequestId::new(NonZeroU64::new(number).expect("not zero"))
}

/// Runs real git for the setup of a test and returns its output without the last line break.
fn raw(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture Author")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "Fixture Author")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git prints text")
        .trim_end_matches('\n')
        .to_owned()
}

fn abs(path: &Path) -> AbsolutePath {
    AbsolutePath::from_str(path.to_str().expect("a UTF-8 path")).expect("an absolute path")
}

fn handle(dir: &Path) -> Git {
    Git::new(
        abs(dir),
        CommitIdentity::new("t".to_owned(), "t@example.com".to_owned(), None),
    )
}

/// A repository with one commit on `main`, in a temporary directory; the temporary directory keeps it alive.
struct Fixture {
    /// Keeps the directory alive.
    tmp: tempfile::TempDir,
    /// The repository.
    repo: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().expect("a temporary directory");
        let repo = tmp.path().canonicalize().unwrap().join("repo");
        std::fs::create_dir(&repo).unwrap();
        raw(&repo, &["init", "-q", "-b", "main"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        raw(&repo, &["add", "a.txt"]);
        raw(&repo, &["commit", "-q", "-m", "First"]);
        Self { tmp, repo }
    }

    fn head(&self) -> CommitId {
        raw(&self.repo, &["rev-parse", "HEAD"]).parse().expect("a commit id")
    }
}

fn workspace(number: u64) -> RequestWorkspace {
    use agent_core::ids::{AgentName, RoundId};
    let worker_task = task(WORKER);
    let worker: AgentName = "glm-1".parse().unwrap();
    RequestWorkspace {
        request: request(number),
        requester: "claude-1".parse().unwrap(),
        requester_task: task(REQUESTER),
        requester_round: RoundId::from_str(ROUND).unwrap(),
        requester_repo: RepoName::from_str(REPO).unwrap(),
        worker: worker.clone(),
        worker_task: worker_task.clone(),
        branch: branch(&worker, &worker_task).unwrap(),
        base: "0000000000000000000000000000000000000000".parse().unwrap(),
        stage: Stage::Working,
    }
}

#[tokio::test]
async fn a_workspace_is_made_at_the_requesters_committed_end_in_their_repository() {
    let fixture = Fixture::new();
    let home = Home::new(&abs(fixture.tmp.path()));
    let base = fixture.head();
    let workspace = workspace(1);
    let workspace = RequestWorkspace {
        base: base.to_string().parse().unwrap(),
        ..workspace
    };

    // The requester has uncommitted work in its own worktree, which the workspace must not touch.
    let requester_worktree = home.worktree_dir(&workspace.requester_repo, &workspace.requester_task);
    std::fs::create_dir_all(&requester_worktree).unwrap();
    std::fs::write(requester_worktree.join("dirty.txt"), "not committed\n").unwrap();

    let requester_git = handle(&fixture.repo);
    prepare_workspace(&home, &requester_git, &workspace).await.unwrap();

    let worktree = home.worktree_dir(&workspace.requester_repo, &workspace.worker_task);
    assert!(worktree.exists(), "the workspace's worktree is there");
    let head: CommitId = raw(&worktree, &["rev-parse", "HEAD"]).parse().unwrap();
    assert_eq!(head, base, "the workspace starts from the requester's committed end");
    assert_eq!(
        raw(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        workspace.branch.as_str(),
        "on a branch of its own"
    );
    assert!(
        !worktree.join("dirty.txt").exists(),
        "isolated from the requester's uncommitted files"
    );
    assert!(worktree.join("a.txt").exists(), "has the requester's committed files");

    let task_dir = home.task_dir(&workspace.requester_task);
    let files = read_each(&task_dir).await.unwrap();
    assert_eq!(files.len(), 1, "the record is in the requester's task directory");
    let read = files.into_iter().next().unwrap().unwrap();
    assert_eq!(read.worker_task, workspace.worker_task);
    assert_eq!(read.requester_task, workspace.requester_task);
    assert_eq!(read.base.to_string(), base.to_string());
}

#[tokio::test]
async fn releasing_removes_the_worktree_branch_and_record() {
    let fixture = Fixture::new();
    let home = Home::new(&abs(fixture.tmp.path()));
    let base = fixture.head();
    let workspace = RequestWorkspace {
        base: base.to_string().parse().unwrap(),
        ..workspace(2)
    };
    let requester_git = handle(&fixture.repo);
    prepare_workspace(&home, &requester_git, &workspace).await.unwrap();
    let worktree = home.worktree_dir(&workspace.requester_repo, &workspace.worker_task);
    assert!(worktree.exists());

    release_workspace(&home, &requester_git, &workspace).await.unwrap();

    assert!(!worktree.exists(), "the workspace's worktree is gone");
    let branches = raw(&fixture.repo, &["branch", "--format=%(refname:short)"]);
    assert!(
        !branches.lines().any(|line| line == workspace.branch.as_str()),
        "the workspace's branch is gone: {branches}"
    );
    let task_dir = home.task_dir(&workspace.requester_task);
    assert_eq!(read_each(&task_dir).await.unwrap().len(), 0);
}

#[tokio::test]
async fn a_workspace_record_saves_and_reads_back() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = workspace(3);
    let task_dir = tmp.path().join("tasks").join(workspace.requester_task.as_str());
    std::fs::create_dir_all(&task_dir).unwrap();

    save(&task_dir, &workspace).unwrap();

    let read = read_each(&task_dir).await.unwrap().into_iter().next().unwrap().unwrap();
    assert_eq!(read, workspace);
}

#[tokio::test]
async fn a_workspace_record_of_the_kept_format_loads() {
    let tmp = tempfile::tempdir().unwrap();
    let task_dir = tmp.path().join("tasks").join(task(REQUESTER).as_str());
    std::fs::create_dir_all(&task_dir).unwrap();
    std::fs::write(
        path_of(&task_dir, request(7)),
        include_str!("fixtures/request-workspace-7.json"),
    )
    .unwrap();

    let read = read_each(&task_dir).await.unwrap().into_iter().next().unwrap().unwrap();
    let expected = RequestWorkspace {
        base: "0123456789abcdef0123456789abcdef01234567".parse().unwrap(),
        ..workspace(7)
    };
    assert_eq!(read, expected);
}

#[tokio::test]
async fn a_committed_step_of_the_workspace_is_brought_over_to_the_requesters_end() {
    let fixture = Fixture::new();
    let home = Home::new(&abs(fixture.tmp.path()));
    let base = fixture.head();
    let workspace = RequestWorkspace {
        base: base.to_string().parse().unwrap(),
        ..workspace(6)
    };
    let requester_git = handle(&fixture.repo);
    prepare_workspace(&home, &requester_git, &workspace).await.unwrap();

    // The worker commits the step of its run in the workspace's worktree, on the branch of its own.
    let worktree = home.worktree_dir(&workspace.requester_repo, &workspace.worker_task);
    std::fs::write(worktree.join("b.txt"), "two\n").unwrap();
    raw(&worktree, &["add", "b.txt"]);
    raw(&worktree, &["commit", "-q", "-m", "the worker's step"]);
    let head: CommitId = raw(&worktree, &["rev-parse", "HEAD"]).parse().unwrap();
    assert_eq!(
        raw(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        workspace.branch.as_str(),
        "the step is on the workspace's branch"
    );

    // The requester's committed end is brought to the worker's step through the workspace's git, and the requester's
    // worktree holds the file the step brought.
    let outcome = requester_git
        .bring_over_since(&base, &head, &workspace.requester_round, &workspace.requester_task)
        .await
        .expect("the worker's step applies");
    let BringOver::Applied { snapshot, commit } = outcome else {
        panic!("the worker's step applies: {outcome:?}");
    };
    assert_eq!(snapshot.head.to_string(), base.to_string());
    assert_eq!(raw(&fixture.repo, &["rev-parse", "HEAD"]), commit.to_string());
    assert_eq!(raw(&fixture.repo, &["show", &format!("{commit}:b.txt")]), "two");
    assert!(
        fixture.repo.join("b.txt").exists(),
        "the requester's worktree holds the worker's file"
    );
}

// ---- failure and recovery branches -------------------------------------------------------------------------------

#[tokio::test]
async fn prepare_rolls_the_worktree_and_branch_back_when_the_record_cannot_be_written() {
    let fixture = Fixture::new();
    let home = Home::new(&abs(fixture.tmp.path()));
    let base = fixture.head();
    let workspace = RequestWorkspace {
        base: base.to_string().parse().unwrap(),
        ..workspace(8)
    };
    let requester_git = handle(&fixture.repo);

    // A file blocks the tasks directory, so the record of the workspace cannot be written after the worktree is made.
    std::fs::create_dir_all(home.root()).unwrap();
    std::fs::write(home.tasks_dir(), "not a directory\n").unwrap();

    let refused = prepare_workspace(&home, &requester_git, &workspace).await;
    assert!(
        matches!(&refused, Err(WorkspaceError::Write { request, .. }) if *request == workspace.request),
        "{refused:?}"
    );

    // The worktree and its branch are rolled back: a failed prepare leaves nothing behind.
    let worktree = home.worktree_dir(&workspace.requester_repo, &workspace.worker_task);
    assert!(!worktree.exists(), "the worktree is rolled back");
    let branches = raw(&fixture.repo, &["branch", "--format=%(refname:short)"]);
    assert!(
        !branches.lines().any(|line| line == workspace.branch.as_str()),
        "the branch is rolled back: {branches}"
    );
}

#[tokio::test]
async fn a_partial_release_can_be_retried_to_completion() {
    let fixture = Fixture::new();
    let home = Home::new(&abs(fixture.tmp.path()));
    let base = fixture.head();
    let workspace = RequestWorkspace {
        base: base.to_string().parse().unwrap(),
        ..workspace(9)
    };
    let requester_git = handle(&fixture.repo);
    prepare_workspace(&home, &requester_git, &workspace).await.unwrap();
    let worktree = home.worktree_dir(&workspace.requester_repo, &workspace.worker_task);
    assert!(worktree.exists());

    // The record's file is a directory when the release tries to remove it, so the removal fails after the worktree
    // and the branch are already gone.
    let record = path_of(&home.task_dir(&workspace.requester_task), workspace.request);
    std::fs::remove_file(&record).unwrap();
    std::fs::create_dir(&record).unwrap();

    let refused = release_workspace(&home, &requester_git, &workspace).await;
    assert!(
        matches!(&refused, Err(WorkspaceError::Write { request, .. }) if *request == workspace.request),
        "{refused:?}"
    );
    assert!(!worktree.exists(), "the worktree was removed before the failure");
    let branches = raw(&fixture.repo, &["branch", "--format=%(refname:short)"]);
    assert!(
        !branches.lines().any(|line| line == workspace.branch.as_str()),
        "the branch was removed before the failure: {branches}"
    );
    assert!(record.exists(), "the record is still there");

    // A later release retries the release and completes it: the already-removed worktree and branch and the
    // now-removed record are not failures again.
    std::fs::remove_dir(&record).unwrap();
    release_workspace(&home, &requester_git, &workspace).await.unwrap();
    assert!(!record.exists(), "the retried release removed the record");
    assert!(!worktree.exists(), "the retried release leaves the worktree gone");
    let branches = raw(&fixture.repo, &["branch", "--format=%(refname:short)"]);
    assert!(
        !branches.lines().any(|line| line == workspace.branch.as_str()),
        "the retried release leaves the branch gone: {branches}"
    );
}

#[tokio::test]
async fn read_each_of_a_missing_task_directory_holds_no_workspaces() {
    let tmp = tempfile::tempdir().unwrap();
    let task_dir = tmp.path().join("tasks").join(task(REQUESTER).as_str());
    assert!(!task_dir.exists());

    let read = read_each(&task_dir).await.unwrap();

    assert!(read.is_empty());
}

#[tokio::test]
async fn a_malformed_workspace_record_is_an_error_of_its_own() {
    let tmp = tempfile::tempdir().unwrap();
    let task_dir = tmp.path().join("tasks").join(task(REQUESTER).as_str());
    std::fs::create_dir_all(&task_dir).unwrap();
    std::fs::write(path_of(&task_dir, request(1)), "not the record of a workspace").unwrap();

    let read = read_each(&task_dir).await.unwrap();
    assert_eq!(read.len(), 1);
    assert!(
        matches!(&read[0], Err(WorkspaceError::Parse { path, .. }) if *path == path_of(&task_dir, request(1))),
        "{:?}",
        read[0]
    );
}

#[tokio::test]
async fn a_workspace_record_of_another_request_is_a_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let task_dir = tmp.path().join("tasks").join(task(REQUESTER).as_str());
    std::fs::create_dir_all(&task_dir).unwrap();
    // The file of request 1 holds the record of request 2.
    let other = workspace(2);
    std::fs::write(
        path_of(&task_dir, request(1)),
        serde_json::to_string_pretty(&other).unwrap(),
    )
    .unwrap();

    let read = read_each(&task_dir).await.unwrap();
    assert_eq!(read.len(), 1);
    assert!(
        matches!(&read[0], Err(WorkspaceError::Mismatch { path, found }) if *path == path_of(&task_dir, request(1)) && *found == other.request.number()),
        "{:?}",
        read[0]
    );
}

#[tokio::test]
async fn a_damaged_workspace_record_does_not_hide_the_others() {
    let tmp = tempfile::tempdir().unwrap();
    let task_dir = tmp.path().join("tasks").join(task(REQUESTER).as_str());
    std::fs::create_dir_all(&task_dir).unwrap();
    std::fs::write(path_of(&task_dir, request(1)), "not the record of a workspace").unwrap();
    save(&task_dir, &workspace(2)).unwrap();

    let read = read_each(&task_dir).await.unwrap();
    assert_eq!(read.len(), 2, "each file is read on its own");
    assert!(matches!(&read[0], Err(WorkspaceError::Parse { .. })), "{:?}", read[0]);
    assert_eq!(read[1].as_ref().unwrap(), &workspace(2));
}

#[tokio::test]
async fn a_workspace_record_of_the_ended_stage_reads_back() {
    let tmp = tempfile::tempdir().unwrap();
    let task_dir = tmp.path().join("tasks").join(task(REQUESTER).as_str());
    std::fs::create_dir_all(&task_dir).unwrap();
    let ended = RequestWorkspace {
        stage: Stage::Ended,
        ..workspace(3)
    };

    save(&task_dir, &ended).unwrap();

    let read = read_each(&task_dir).await.unwrap().into_iter().next().unwrap().unwrap();
    assert_eq!(read, ended);
    assert_eq!(read.stage, Stage::Ended);
}

#[tokio::test]
async fn the_requesters_dirty_worktree_stays_byte_identical_after_the_workspace_is_made() {
    let fixture = Fixture::new();
    let home = Home::new(&abs(fixture.tmp.path()));
    let base = fixture.head();
    let workspace = RequestWorkspace {
        base: base.to_string().parse().unwrap(),
        ..workspace(10)
    };
    let requester_git = handle(&fixture.repo);

    // The requester's own worktree — the repository itself — has uncommitted work: a changed file and an untracked one.
    std::fs::write(fixture.repo.join("a.txt"), "the requester's own uncommitted edit\n").unwrap();
    std::fs::write(fixture.repo.join("untracked.txt"), "untracked by the requester\n").unwrap();
    let changed_before = std::fs::read(fixture.repo.join("a.txt")).unwrap();
    let untracked_before = std::fs::read(fixture.repo.join("untracked.txt")).unwrap();

    prepare_workspace(&home, &requester_git, &workspace).await.unwrap();

    // The requester's uncommitted files are untouched, byte for byte.
    assert_eq!(std::fs::read(fixture.repo.join("a.txt")).unwrap(), changed_before);
    assert_eq!(
        std::fs::read(fixture.repo.join("untracked.txt")).unwrap(),
        untracked_before
    );

    // The workspace's worktree starts from the committed content, without the requester's uncommitted work.
    let worktree = home.worktree_dir(&workspace.requester_repo, &workspace.worker_task);
    assert_eq!(std::fs::read_to_string(worktree.join("a.txt")).unwrap(), "one\n");
    assert!(!worktree.join("untracked.txt").exists());
}
