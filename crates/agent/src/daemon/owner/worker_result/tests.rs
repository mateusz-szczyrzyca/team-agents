//! The facts a worker's result is judged by and the reasons it is returned for.

#![cfg(test)]

use agent_core::decide::worker_result::{CheckOutcome, Elsewhere, Scope, WorkerEnding, Wrong};
use agent_runtime::worker_request::{Checked, Found};

use agent_core::ids::{AgentName, TaskId};
use agent_runtime::agentbus::{DelegationRequest, DelegationStatus};

use super::{facts_of, is_running_for, reasons_of, scope_of};

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
