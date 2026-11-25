//! The facts a worker's result is judged by and the reasons it is returned for.

#![cfg(test)]

use agent_core::decide::worker_result::{CheckOutcome, Elsewhere, Scope, WorkerEnding, Wrong};
use agent_runtime::worker_request::{Checked, Found};

use std::path::Path;

use agent_core::ids::{AgentName, TaskId};
use agent_runtime::agentbus::{DelegationRequest, DelegationStatus};

use super::{RequestCheck, facts_of, is_running_for, reasons_of, run_request_check, scope_of};

/// Runs git in `dir` with a fixed identity and returns its standard output, so a test can build the worktree a release
/// of a workspace needs.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
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

fn found(outcome: CheckOutcome) -> Found {
    Found {
        outcome,
        line: "test -s docs/count.txt".to_owned(),
        tail: String::new(),
        reason: Some("check failed with exit code 1".to_owned()),
        duration_ms: 5,
        output: None,
    }
}

fn checked(files: &[&str], outside: &[&str], found: Option<Found>) -> Checked {
    Checked {
        step: Some("2.1".parse().expect("a step")),
        files: files.iter().map(|file| (*file).to_owned()).collect(),
        head: Some("89abcdef0123456789abcdef0123456789abcdef".to_owned()),
        committed: true,
        outside: outside.iter().map(|file| (*file).to_owned()).collect(),
        found,
    }
}

#[test]
fn a_request_is_for_facts_when_it_names_no_file_and_for_files_otherwise() {
    assert_eq!(scope_of(&[]), Scope::Facts);
    assert_eq!(scope_of(&["docs/guide.md".to_owned()]), Scope::Files);
}

#[test]
fn the_facts_of_a_result_say_what_the_check_found_and_what_was_changed_outside() {
    let table = [
        (
            "the check passed",
            checked(&["a.md"], &[], Some(found(CheckOutcome::Passed))),
            0,
            0,
            CheckOutcome::Passed,
            false,
            false,
            Scope::Files,
        ),
        (
            "the check failed and a file is outside",
            checked(&["a.md"], &["b.md"], Some(found(CheckOutcome::Failed))),
            0,
            0,
            CheckOutcome::Failed,
            true,
            false,
            Scope::Files,
        ),
        (
            "no check, facts only",
            checked(&[], &[], None),
            0,
            0,
            CheckOutcome::Absent,
            false,
            false,
            Scope::Facts,
        ),
        (
            "a run of 61 seconds against a limit of 60",
            checked(&["a.md"], &[], Some(found(CheckOutcome::Passed))),
            61,
            60,
            CheckOutcome::Passed,
            false,
            true,
            Scope::Files,
        ),
        (
            "a run of 60 seconds against a limit of 60",
            checked(&["a.md"], &[], Some(found(CheckOutcome::Passed))),
            60,
            60,
            CheckOutcome::Passed,
            false,
            false,
            Scope::Files,
        ),
        (
            "a long run and no limit",
            checked(&["a.md"], &[], Some(found(CheckOutcome::Passed))),
            86_400,
            0,
            CheckOutcome::Passed,
            false,
            false,
            Scope::Files,
        ),
    ];
    for (what, checked, run, limit, check, outside, past, scope) in table {
        let facts = facts_of(&checked, run, limit);

        assert_eq!(facts.ending, WorkerEnding::Done, "{what}");
        assert_eq!(facts.check, check, "{what}");
        assert_eq!(facts.changed_outside_files, outside, "{what}");
        assert_eq!(facts.past_seconds, past, "{what}");
        assert_eq!(facts.scope, scope, "{what}");
        assert_eq!(
            (facts.try_number, facts.tries, facts.elsewhere),
            (1, 1, Elsewhere::None),
            "{what}"
        );
    }
}

#[test]
fn every_reason_a_result_is_wrong_has_its_words() {
    let with_check = checked(&["a.md"], &["b.md", "c.md"], Some(found(CheckOutcome::Failed)));
    let without = checked(&["a.md"], &[], None);

    let reasons = reasons_of(
        &with_check,
        &[
            Wrong::EndedOtherwise(WorkerEnding::Failed),
            Wrong::CheckFailed,
            Wrong::CheckTimedOut,
            Wrong::OutsideFiles,
            Wrong::NoCheck,
            Wrong::PastSeconds,
        ],
        90,
    );

    assert_eq!(
        reasons,
        vec![
            "the worker did not end its run with STATUS: DONE".to_owned(),
            "the check `test -s docs/count.txt` failed (check failed with exit code 1)".to_owned(),
            "the check ran past its time limit".to_owned(),
            "the worker changed files the request did not name: b.md, c.md".to_owned(),
            "the request names files but no check, so nothing shows the commit right".to_owned(),
            "the worker's run lasted past 90 seconds".to_owned(),
        ]
    );
    assert_eq!(
        reasons_of(&without, &[Wrong::CheckFailed], 0),
        vec!["the check failed".to_owned()]
    );
}

/// The request `1` of the requester `claude-1` as the bus has it: `status`, with `helper_task` and `worker` as given.
fn known(status: &str, helper_task: Option<&str>, worker: Option<&str>) -> DelegationRequest {
    serde_json::from_value(serde_json::json!({
        "id": 1, "from": "claude-1", "from_task": "task-20261122-120000-aaaa", "repo": "example",
        "branch": "agents/claude-1/task-20261122-120000-aaaa", "worktree": "/tmp/example", "request": "Do it.",
        "context": "", "status": status, "worker": worker, "helper_task": helper_task, "summary": "",
    }))
    .expect("a request of the bus")
}

#[test]
fn a_request_is_the_workers_while_the_bus_has_it_running_for_its_task_or_for_its_name_before_the_task_is_known() {
    let task: TaskId = "task-20261122-120001-bbbb".parse().expect("a task");
    let other: TaskId = "task-20261122-120002-cccc".parse().expect("a task");
    let glm1: AgentName = "glm-1".parse().expect("a name");
    let table = [
        ("running", Some(&task), Some("glm-1"), true, "running for its task"),
        ("starting", Some(&task), Some("glm-1"), true, "starting for its task"),
        (
            "running",
            Some(&other),
            Some("glm-1"),
            false,
            "running for another task",
        ),
        (
            "running",
            None,
            Some("glm-1"),
            true,
            "the update of the task has not landed: the worker's name holds",
        ),
        ("running", None, Some("glm-2"), false, "another worker's name"),
        (
            "starting",
            Some(&task),
            Some("glm-2"),
            false,
            "the bus names the second worker, the task is still the first's",
        ),
        ("running", Some(&task), Some("glm-2"), false, "the same, as running"),
        ("running", None, None, false, "nobody's"),
        ("done", Some(&task), Some("glm-1"), false, "answered"),
        ("failed", Some(&task), Some("glm-1"), false, "answered as failed"),
        ("queued", None, None, false, "queued"),
    ];
    for (status, helper_task, worker, expected, why) in table {
        let request = known(status, helper_task.map(TaskId::as_str), worker);
        assert_eq!(is_running_for(&request, &task, &glm1), expected, "{why}");
    }
    assert_eq!(known("running", None, None).status, DelegationStatus::Running);
}

#[tokio::test]
async fn a_check_whose_workspace_worktree_cannot_be_reached_tells_why_in_the_recorded_reason() {
    use agent_runtime::process::error_chain;
    use agent_runtime::request_workspace::WorkspaceError;

    use crate::fixtures::{repo, task};

    // `start_request_check` builds the fault of a request served in a workspace from `request_workspace::worktree_of`;
    // that cause is what the recorded failed check keeps, not a generic "no worktree" text. Drive the free
    // `run_request_check` with the exact cause `worktree_of` yields for a non-text path, and prove it surfaces.
    let cause = error_chain(&WorkspaceError::NotText {
        repo: repo(),
        task: task(2),
        path: "/tmp/example".into(),
    });
    let found = run_request_check(RequestCheck::new(
        Err(cause.clone()),
        "/tmp/example".into(),
        "1.1".parse().unwrap(),
        "test -s docs/count.txt".to_owned(),
        60,
        vec![],
    ))
    .await;

    assert_eq!(found.outcome, CheckOutcome::Failed);
    assert_eq!(found.line, "test -s docs/count.txt");
    assert_eq!(
        found.reason.as_deref(),
        Some(cause.as_str()),
        "the recorded failed check keeps the worktree_of cause, not a generic message"
    );
    assert_ne!(
        found.reason.as_deref(),
        Some("the worker's worktree cannot be reached"),
        "the reason is the specific not-UTF-8 worktree cause, not the generic fallback"
    );
}

/// The workspace record of request 1, requester `claude-1` of task 1, working in the workspace of `claude-2`'s task 2.
fn a_workspace_record(
    stage: agent_runtime::request_workspace::Stage,
) -> agent_runtime::request_workspace::RequestWorkspace {
    use std::num::NonZeroU64;
    use std::str::FromStr;

    use crate::fixtures::{repo, task};
    use agent_core::ids::RoundId;
    use agent_runtime::agentbus::RequestId;
    use agent_runtime::request_workspace::branch;

    let worker: agent_core::ids::AgentName = "claude-2".parse().unwrap();
    let worker_task = task(2);
    agent_runtime::request_workspace::WorkspaceRecord {
        request: RequestId::new(NonZeroU64::new(1).unwrap()),
        requester: "claude-1".parse().unwrap(),
        requester_task: task(1),
        requester_round: RoundId::from_str("round-20261124-090000-aaaa").unwrap(),
        requester_repo: repo(),
        worker: worker.clone(),
        worker_task: worker_task.clone(),
        branch: branch(&worker, &worker_task).unwrap(),
        base: "0000000000000000000000000000000000000000".parse().unwrap(),
        stage,
    }
    .try_into()
    .unwrap()
}

/// The pending of request 1 in the worker `claude-2`'s task 2, matching [`a_workspace_record`].
fn a_matching_pending() -> agent_runtime::worker_request::Pending {
    use std::num::NonZeroU64;

    use crate::fixtures::{task, team_id};
    use agent_runtime::agentbus::RequestId;
    use agent_runtime::worker_request::{Pending, Stage};

    Pending {
        request: RequestId::new(NonZeroU64::new(1).unwrap()),
        team: team_id(0),
        requester: "claude-1".parse().unwrap(),
        requester_task: task(1),
        worker: "claude-2".parse().unwrap(),
        worker_task: task(2),
        summary: "finished".to_owned(),
        run_seconds: 0,
        base: None,
        stage: Stage::Ended,
    }
}

/// A daemon in a temporary home with one pair team, whose owner is not running.
async fn pair_daemon() -> (super::super::fixture::Daemon, super::super::Owner) {
    use super::super::fixture::{Daemon, pair_state};

    let daemon = Daemon::new(&pair_state(), None);
    let (owner, _sinks) = daemon.bare_owner().await;
    (daemon, owner)
}

#[tokio::test]
async fn a_completion_whose_record_cannot_be_written_leaves_the_workspace_working_and_is_tried_again() {
    use std::os::unix::fs::PermissionsExt;

    use crate::fixtures::{task, team_id};
    use agent_runtime::request_workspace::save;
    use agent_runtime::worker_request::write_pending;

    let (daemon, mut owner) = pair_daemon().await;
    let record = a_workspace_record(agent_runtime::request_workspace::Stage::Working);
    let requester_dir = daemon.home.task_dir(&task(1));
    let worker_dir = daemon.home.task_dir(&task(2));
    std::fs::create_dir_all(&requester_dir).unwrap();
    std::fs::create_dir_all(&worker_dir).unwrap();
    save(&requester_dir, &record).unwrap();
    write_pending(&worker_dir, &a_matching_pending()).unwrap();
    // The requester's task directory is read-only, so the completion's write of the Ended record fails and the record
    // stays Working to be completed again at the next look.
    std::fs::set_permissions(&requester_dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    std::fs::set_permissions(&worker_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    owner.settle_workspace_results(&team_id(0)).await.unwrap();

    let on_disk: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(requester_dir.join("request-workspace-1.json")).unwrap())
            .unwrap();
    assert_eq!(
        on_disk["stage"]["workspace_stage"], "working",
        "the record stays Working"
    );
    assert_eq!(owner.unreadable_requests.len(), 1, "the failed completion is told once");
    assert!(
        owner
            .unreadable_requests
            .iter()
            .any(|reason| reason.contains("cannot be saved as ended")),
        "the told reason names the failed completion, got {:?}",
        owner.unreadable_requests
    );

    // A second look tries the completion again: the fault is still told once.
    owner.settle_workspace_results(&team_id(0)).await.unwrap();
    assert_eq!(owner.unreadable_requests.len(), 1);
    std::fs::set_permissions(&requester_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    // A third look, once the directory is writable again, completes the record: the stage on disk moves from working
    // to ended, so the completion/settlement actually progresses once the fault is repaired, instead of only retrying
    // it. (Any release the settlement then attempts has no git here, so it may add a `release 1`; what is asserted is
    // the progress of the record, not the count of told faults.)
    owner.settle_workspace_results(&team_id(0)).await.unwrap();
    let on_disk: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(requester_dir.join("request-workspace-1.json")).unwrap())
            .unwrap();
    assert_eq!(
        on_disk["stage"]["workspace_stage"], "ended",
        "the record progresses to ended once the directory is writable again"
    );
}

#[tokio::test]
async fn a_leftover_workspace_that_cannot_be_released_is_told_once_and_kept_for_another_try() {
    use crate::fixtures::{repo, task, team_id};

    let (daemon, mut owner) = pair_daemon().await;
    let record = a_workspace_record(agent_runtime::request_workspace::Stage::Ended);
    let requester_dir = daemon.home.task_dir(&task(1));
    std::fs::create_dir_all(&requester_dir).unwrap();
    agent_runtime::request_workspace::save(&requester_dir, &record).unwrap();
    // No pending and no `*.damaged` marker: a leftover of a settled request whose workspace is to be released. The
    // requester's worktree cannot be released (there is no such git worktree), so the release fails and is tried again.
    owner.settle_workspace_results(&team_id(0)).await.unwrap();

    assert!(
        owner.unreadable_requests.iter().any(|reason| reason == "release 1"),
        "the failed release is told once, got {:?}",
        owner.unreadable_requests
    );
    assert!(
        requester_dir.join("request-workspace-1.json").exists(),
        "the record is kept for the next look to release it"
    );

    // A second look with the fault still there is told once again, not twice: the same release is not re-told.
    owner.settle_workspace_results(&team_id(0)).await.unwrap();
    assert_eq!(
        owner
            .unreadable_requests
            .iter()
            .filter(|reason| reason == &"release 1")
            .count(),
        1
    );
    assert!(
        requester_dir.join("request-workspace-1.json").exists(),
        "the record is still kept"
    );

    // Repair the git the release needs: a checkout whose worktree for task 1 is the requester's worktree (where the
    // release runs) and whose worktree for task 2 is the workspace on its branch. A third look then removes the
    // worktree, deletes the branch and drops the record.
    let checkout = tempfile::tempdir().unwrap();
    git(checkout.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(checkout.path().join("README.md"), "read me\n").unwrap();
    git(checkout.path(), &["add", "README.md"]);
    git(checkout.path(), &["commit", "-q", "-m", "first"]);
    let requester_worktree = daemon.home.worktree_dir(&repo(), &task(1));
    let worker_worktree = daemon.home.worktree_dir(&repo(), &task(2));
    let requester_branch =
        agent_runtime::request_workspace::branch(&record.record().requester, &record.record().requester_task).unwrap();
    for (path, branch) in [
        (requester_worktree, requester_branch),
        (worker_worktree.clone(), record.record().branch.clone()),
    ] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        git(
            checkout.path(),
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

    owner.settle_workspace_results(&team_id(0)).await.unwrap();

    assert!(
        !requester_dir.join("request-workspace-1.json").exists(),
        "the record is dropped once the release succeeds"
    );
    assert!(!worker_worktree.exists(), "the worker's worktree is removed");
    assert_eq!(
        git(checkout.path(), &["branch", "--list", record.record().branch.as_str()]).trim(),
        "",
        "the workspace branch is deleted"
    );
}

#[tokio::test]
async fn a_workspace_record_that_cannot_be_read_is_told_once_and_left_alone() {
    use crate::fixtures::{task, team_id};

    let (daemon, mut owner) = pair_daemon().await;
    let requester_dir = daemon.home.task_dir(&task(1));
    std::fs::create_dir_all(&requester_dir).unwrap();
    let record_path = requester_dir.join("request-workspace-1.json");
    std::fs::write(&record_path, "not json at all").unwrap();

    owner.settle_workspace_results(&team_id(0)).await.unwrap();
    assert_eq!(owner.unreadable_requests.len(), 1, "the unreadable record is told once");
    assert_eq!(
        std::fs::read_to_string(&record_path).unwrap(),
        "not json at all",
        "the file is left alone"
    );
}

#[tokio::test]
async fn removal_refuses_or_joins_a_workers_pending_check() {
    use std::num::NonZeroU64;

    use agent_core::protocol::{Client, FailureKind, RemoveRequest};
    use agent_runtime::agentbus::RequestId;
    use agent_runtime::worker_request::{Pending, Stage};

    use super::super::fixture::{Daemon, pair_state};
    use crate::fixtures::{repo, task, team_id};

    for force in [false, true] {
        let daemon = Daemon::new(&pair_state(), None);
        let (mut owner, _sinks) = daemon.bare_owner().await;
        let task_dir = daemon.home.task_dir(&task(2));
        std::fs::create_dir_all(&task_dir).unwrap();
        for number in [1, 2] {
            std::fs::create_dir_all(daemon.home.worktree_dir(&repo(), &task(number))).unwrap();
        }
        let pending = Pending {
            request: RequestId::new(NonZeroU64::new(1).unwrap()),
            team: team_id(0),
            requester: "claude-1".parse().unwrap(),
            requester_task: task(1),
            worker: "claude-2".parse().unwrap(),
            worker_task: task(2),
            summary: "finished".to_owned(),
            run_seconds: 0,
            base: None,
            stage: Stage::Ended,
        };
        owner.start_request_check(
            &task_dir,
            pending,
            checked(&[], &[], None),
            "1.1".parse().unwrap(),
            "/bin/true".to_owned(),
            None,
        );

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
            assert_eq!(owner.request_checking.len(), 0);
            assert!(owner.request_checks.is_empty());
            assert_eq!(result.unwrap().stopped, vec![task(2)]);
            assert!(!task_dir.exists());
        } else {
            assert_eq!(result.unwrap_err().kind, FailureKind::Conflict);
            assert!(owner.parts.state.team(&team_id(0)).is_some());
            assert!(task_dir.exists());
            let joined = owner.request_checks.join_next_with_id().await.unwrap();
            owner.request_check_done(joined).await.unwrap();
        }
    }
}

#[tokio::test]
async fn starting_a_workspace_check_preserves_the_joined_path_length_error() {
    use std::sync::Arc;

    use agent_core::config::{AbsolutePath, ValueRule};
    use agent_runtime::process::error_chain;
    use agent_runtime::request_workspace::{Stage, WorkspaceError, worktree_of};
    use agent_runtime::store::{Home, StateStore};
    use agent_runtime::worker_request::{Stage as PendingStage, read_pending_each};

    let (daemon, mut owner) = pair_daemon().await;
    let root = tempfile::tempdir().unwrap();
    let mut text = root.path().to_str().unwrap().to_owned();
    // Keep the home and its lock below the path bound, while the joined worker path exceeds it.
    while text.len() < 4070 {
        text.push('/');
        let component = (4070 - text.len()).min(200);
        text.push_str(&"h".repeat(component));
    }
    let home = Home::new(&text.parse::<AbsolutePath>().unwrap());
    owner.parts.store = Arc::new(StateStore::open(home.clone()).await.unwrap());
    let record = a_workspace_record(Stage::Ended);
    let error = worktree_of(&home, &record.record().requester_repo, &record.record().worker_task).unwrap_err();
    assert!(matches!(
        &error,
        WorkspaceError::NotAbsolute { source, .. } if source.rule == ValueRule::TooLong { max: 4096 }
    ));
    let reason = error_chain(&error);
    let task_dir = daemon.home.task_dir(&record.record().worker_task);
    std::fs::create_dir_all(&task_dir).unwrap();

    owner.start_request_check(
        &task_dir,
        a_matching_pending(),
        checked(&[], &[], None),
        "1.1".parse().unwrap(),
        "/bin/true".to_owned(),
        Some(&record),
    );
    let joined = owner.request_checks.join_next_with_id().await.unwrap();
    owner.request_check_done(joined).await.unwrap();

    let mut pending = read_pending_each(&task_dir).await.unwrap();
    assert_eq!(pending.len(), 1);
    let pending = pending.pop().unwrap().unwrap();
    let PendingStage::Checked(checked) = pending.stage else {
        panic!("the starter's check must be recorded");
    };
    let found = checked.found.unwrap();
    assert_eq!(found.outcome, CheckOutcome::Failed);
    assert_eq!(found.reason.as_deref(), Some(reason.as_str()));
}

#[tokio::test]
async fn mismatched_pending_worker_or_team_quarantines_the_workspace() {
    use crate::fixtures::{task, team_id};
    use agent_runtime::worker_request::{is_set_aside, write_pending};

    for wrong_worker in [true, false] {
        let (daemon, mut owner) = pair_daemon().await;
        let record = a_workspace_record(agent_runtime::request_workspace::Stage::Ended);
        let mut pending = a_matching_pending();
        if wrong_worker {
            pending.worker = "claude-3".parse().unwrap();
        } else {
            pending.team = team_id(1);
        }
        let worker_dir = daemon.home.task_dir(&task(2));
        std::fs::create_dir_all(&worker_dir).unwrap();
        write_pending(&worker_dir, &pending).unwrap();

        owner.settle_workspace_one(&team_id(0), &record).await.unwrap();

        assert!(
            is_set_aside(&worker_dir, pending.request).await.unwrap(),
            "a mismatched worker ({wrong_worker}) or team must be quarantined"
        );
        assert_eq!(owner.answered_requests.len(), 0);
    }
}

#[tokio::test]
async fn releasing_without_requester_git_records_one_warning_per_request() {
    use crate::fixtures::team_id;

    let (_daemon, mut owner) = pair_daemon().await;
    let record = a_workspace_record(agent_runtime::request_workspace::Stage::Ended);

    for _ in 0..2 {
        owner.release_workspace(&team_id(9), &record).await.unwrap();
    }

    assert_eq!(
        owner.unreadable_requests,
        ["release 1".to_owned()].into_iter().collect()
    );
}

#[tokio::test]
async fn elsewhere_of_defers_conservatively_when_helper_request_is_missing() {
    use super::super::fixture::{Daemon, fake_cli, pair_state};
    use agent_runtime::agentbus::RequestId;
    use std::num::NonZeroU64;

    let daemon = Daemon::new(&pair_state(), None);
    let custom_config = format!(
        "[agents.claude-1]\ncli = \"claude\"\n[agents.claude-2]\ncli = \"claude\"\n\
         [agents.w-1]\ncli = \"claude\"\n\
         [agents.w-2]\ncli = \"opencode\"\nmodel = \"openrouter/free\"\n\
         [tiers]\nnormal = [\"claude-1\", \"claude-2\"]\nworkers = [\"w-1\", \"w-2\"]\n\
         [binaries]\nclaude = \"{}\"\nopencode = \"{}\"\n\
         [repos.team-agents]\nbase_branch = \"main\"\ngate = [\"true\"]\n",
        fake_cli(),
        fake_cli(),
    );
    std::fs::write(daemon.home.config_file(), custom_config).unwrap();
    let (owner, _sinks) = daemon.bare_owner().await;

    let mut pending = a_matching_pending();
    pending.worker = "w-1".parse().unwrap();
    pending.request = RequestId::new(NonZeroU64::new(999).unwrap());

    let elsewhere = owner.elsewhere_of(&pending).await;
    assert_eq!(elsewhere, Elsewhere::None);
}

#[tokio::test]
async fn dispatch_pool_request_preserves_cleanup_obligation_on_release_failure_and_retries() {
    use std::os::unix::fs::PermissionsExt;

    use crate::daemon::owner::worker_result::Again;
    use crate::fixtures::team_id;
    use agent_runtime::agentbus::RequestId;
    use std::num::NonZeroU64;

    let (daemon, mut owner) = pair_daemon().await;
    let record = a_workspace_record(agent_runtime::request_workspace::Stage::Working);
    let worker_name: AgentName = "w-nonexistent".parse().unwrap();
    let request_id = RequestId::new(NonZeroU64::new(1).unwrap());

    let again = Again {
        team: team_id(0),
        worker: "w-old".parse().unwrap(),
        failed_cli: agent_core::kinds::CliKind::Claude,
        failed_provider: "claude".to_owned(),
        tried: agent_runtime::worker_request::Try {
            number: 2,
            earlier: vec!["first try failed".to_owned()],
        },
        reason: "wrong result".to_owned(),
    };
    owner.again.insert(request_id, again);

    let req = DelegationRequest {
        id: request_id,
        from: "claude-1".parse().unwrap(),
        from_task: record.record().requester_task.clone(),
        repo: record.record().requester_repo.clone(),
        branch: "main".parse().unwrap(),
        worktree: daemon
            .home
            .worktree_dir(&record.record().requester_repo, &record.record().requester_task),
        request: "do work".to_owned(),
        context: String::new(),
        status: DelegationStatus::Queued,
        worker: None,
        helper_task: None,
        summary: String::new(),
    };

    let repo_dir = daemon
        .home
        .worktree_dir(&record.record().requester_repo, &record.record().requester_task);
    std::fs::create_dir_all(&repo_dir).unwrap();
    git(&repo_dir, &["init", "-b", "main"]);
    git(&repo_dir, &["commit", "--allow-empty", "-m", "Initial"]);
    // Git runs this hook in the newly created worktree. Locking it makes rollback
    // fail after allocation, without changing the requester's working files.
    let hook = repo_dir.join(".git/hooks/post-checkout");
    std::fs::write(&hook, "#!/bin/sh\ngit worktree lock \"$PWD\"\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let res = owner.try_dispatch_pool_request(&worker_name, &req).await;
    assert!(
        matches!(res, Err(super::super::teams::PoolDispatchError::WorkerConfigNotFound(ref name)) if name == &worker_name)
    );

    assert_eq!(
        owner.cleanup_obligations.len(),
        1,
        "cleanup obligation must be retained on release failure"
    );
    assert!(
        owner.again.contains_key(&request_id),
        "again retry history must be preserved"
    );
    assert!(
        owner.worker_availability(&worker_name).await.unsettled,
        "worker must be marked unsettled while obligation exists"
    );

    let worker_task = owner.cleanup_obligations[0].workspace.record().worker_task.clone();
    let worker_path = daemon.home.worktree_dir(&req.repo, &worker_task);
    git(&repo_dir, &["worktree", "unlock", worker_path.to_str().unwrap()]);
    owner.retry_cleanup_obligations().await;

    assert!(
        owner.cleanup_obligations.is_empty(),
        "cleanup obligation must be cleared after successful retry"
    );
    assert!(
        !owner.worker_availability(&worker_name).await.unsettled,
        "worker must no longer be unsettled"
    );
    assert!(
        owner.again.contains_key(&request_id),
        "again retry history must remain preserved"
    );
}

#[tokio::test]
async fn answered_request_keeps_worker_reserved_until_workspace_release() {
    use crate::fixtures::{repo, task, team_id};

    let (daemon, mut owner) = pair_daemon().await;
    let record = a_workspace_record(agent_runtime::request_workspace::Stage::Ended);
    let requester_dir = daemon.home.task_dir(&task(1));
    std::fs::create_dir_all(&requester_dir).unwrap();
    agent_runtime::request_workspace::save(&requester_dir, &record).unwrap();
    owner.answered_requests.insert(record.record().request);

    owner.settle_workspace_results(&team_id(0)).await.unwrap();

    assert!(owner.unreadable_requests.contains("release 1"));
    assert!(requester_dir.join("request-workspace-1.json").exists());
    assert!(
        owner.worker_availability(&record.record().worker).await.unsettled,
        "an answered request still owns its worker while release fails"
    );

    let checkout = tempfile::tempdir().unwrap();
    git(checkout.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(checkout.path().join("README.md"), "read me\n").unwrap();
    git(checkout.path(), &["add", "README.md"]);
    git(checkout.path(), &["commit", "-q", "-m", "first"]);
    let requester_worktree = daemon.home.worktree_dir(&repo(), &task(1));
    let worker_worktree = daemon.home.worktree_dir(&repo(), &task(2));
    let requester_branch =
        agent_runtime::request_workspace::branch(&record.record().requester, &record.record().requester_task).unwrap();
    for (path, branch) in [
        (requester_worktree, requester_branch),
        (worker_worktree.clone(), record.record().branch.clone()),
    ] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        git(
            checkout.path(),
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

    owner.settle_workspace_results(&team_id(0)).await.unwrap();

    assert!(!requester_dir.join("request-workspace-1.json").exists());
    assert!(!worker_worktree.exists());
    assert!(
        !owner.worker_availability(&record.record().worker).await.unsettled,
        "successful release makes the worker eligible again"
    );
}

#[test]
fn retry_journal_retains_check_and_ending_causes_instead_of_a_done_failure() {
    for (ending, outcome, committed, want) in [
        (WorkerEnding::Done, CheckOutcome::Failed, true, vec![Wrong::CheckFailed]),
        (
            WorkerEnding::Failed,
            CheckOutcome::Failed,
            true,
            vec![Wrong::EndedOtherwise(WorkerEnding::Failed), Wrong::CheckFailed],
        ),
        (
            WorkerEnding::Cannot,
            CheckOutcome::Passed,
            true,
            vec![Wrong::EndedOtherwise(WorkerEnding::Cannot)],
        ),
        (WorkerEnding::Done, CheckOutcome::Passed, false, vec![Wrong::NoCommit]),
    ] {
        let mut value = checked(&["guide.md"], &[], Some(found(outcome)));
        value.committed = committed;
        assert_eq!(super::rejection_causes(&value, ending, 0, 0), want);
    }
}

#[tokio::test]
async fn durable_final_settlement_removes_its_try_before_publishing_the_answer() {
    use std::os::unix::fs::PermissionsExt;

    use super::super::fixture::{Daemon, pair_state};
    use agent_runtime::worker_request::{Settlement, Stage, Try, try_path, write_pending, write_try};

    let scratch = tempfile::tempdir().unwrap();
    let program = scratch.path().join("observe-answer");
    let observations = scratch.path().join("observations");
    let daemon = Daemon::new(&pair_state(), Some(&program));
    let (mut owner, _sinks) = daemon.bare_owner().await;
    let mut pending = a_matching_pending();
    let task_dir = daemon.home.task_dir(&pending.worker_task);
    std::fs::create_dir_all(&task_dir).unwrap();
    let retry = try_path(&task_dir, pending.request);
    pending.stage = Stage::Settled {
        checked: Box::new(checked(&[], &[], None)),
        settlement: Settlement::Returned {
            reasons: vec!["try 1: empty result; try 2: worker failed".to_owned()],
        },
    };
    write_pending(&task_dir, &pending).unwrap();
    write_try(&task_dir, pending.request, &Try::first()).unwrap();
    std::fs::write(
        &program,
        format!(
            "#!/bin/sh\nif test -e '{}'; then echo present; else echo absent; fi >> '{}'\nexit 0\n",
            retry.display(),
            observations.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();

    owner.settle_one(&task_dir, pending, None).await.unwrap();

    assert_eq!(std::fs::read_to_string(observations).unwrap(), "absent\nabsent\n");
}

/// A reporter of the history that says nothing: the tests read the rows.
#[derive(Debug)]
struct MuteReporter;

impl agent_runtime::history::Reporter for MuteReporter {
    fn report(&self, _problem: &agent_runtime::history::Problem) {}
}

/// Settles a request of the pair team that has nothing to bring over, in an owner that writes to a history, checks the one
/// record of its end and records `worker_request_ended` into `observed`.
pub(in crate::daemon::owner) async fn record_worker_request_ended(
    observed: &mut std::collections::BTreeSet<agent_core::log::LogEvent>,
) {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Arc;

    use super::super::fixture::{Daemon, pair_state};
    use agent_core::history::MaxTextBytes;
    use agent_core::log::LogEvent;
    use agent_runtime::history::{DEFAULT_QUEUE, History, HistoryDb, Reader, Setup};
    use agent_runtime::worker_request::Stage;

    let scratch = tempfile::tempdir().unwrap();
    let program = scratch.path().join("bus");
    std::fs::write(&program, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = scratch.path().join("history.db");
    let started = History::start(
        HistoryDb::open(&path, MaxTextBytes::default()).unwrap(),
        Setup {
            reporter: Arc::new(MuteReporter),
            capacity: DEFAULT_QUEUE,
        },
    )
    .unwrap();
    let daemon = Daemon::new(&pair_state(), Some(&program));
    let (owner, _sinks) = daemon.bare_owner().await;
    let mut owner = owner.with_history(started.history.clone());
    let mut pending = a_matching_pending();
    pending.stage = Stage::Checked(Box::new(checked(&[], &[], None)));
    let task_dir = daemon.home.task_dir(&pending.worker_task);
    std::fs::create_dir_all(&task_dir).unwrap();
    agent_runtime::worker_request::write_pending(&task_dir, &pending).unwrap();

    owner.settle_one(&task_dir, pending, None).await.unwrap();
    started.writer.close().unwrap();

    let rows: Vec<_> = Reader::open(&path)
        .unwrap()
        .daemon_logs()
        .unwrap()
        .into_iter()
        .filter(|row| row.event() == LogEvent::WorkerRequestEnded)
        .collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].fields()["outcome"], "report");
    assert_eq!(rows[0].fields()["request"], "H1");
    assert_eq!(rows[0].fields()["step"], "2.1");
    assert_eq!(
        rows[0].team().map(ToString::to_string),
        Some(crate::fixtures::team_id(0).to_string())
    );
    observed.insert(LogEvent::WorkerRequestEnded);
}

#[tokio::test]
async fn a_request_settled_by_the_owner_is_one_record_of_the_history() {
    let mut observed = std::collections::BTreeSet::new();

    record_worker_request_ended(&mut observed).await;

    assert!(observed.contains(&agent_core::log::LogEvent::WorkerRequestEnded));
}
