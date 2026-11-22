//! The file of a request whose worker has ended: written stage by stage and read back.

#![cfg(test)]

use std::num::NonZeroU64;
use std::str::FromStr;

use agent_core::decide::worker_result::CheckOutcome;
use agent_core::ids::{AgentName, TaskId, TeamId};

use super::*;

fn request(number: u64) -> RequestId {
    RequestId::new(NonZeroU64::new(number).expect("a number"))
}

fn pending(number: u64, stage: Stage) -> Pending {
    Pending {
        request: request(number),
        team: TeamId::from_str("team-20261122-120000-aaaa").expect("a team"),
        requester: AgentName::from_str("claude-1").expect("a name"),
        requester_task: TaskId::from_str("task-20261122-120000-aaaa").expect("a task"),
        worker: AgentName::from_str("glm-1").expect("a name"),
        worker_task: TaskId::from_str("task-20261122-120001-bbbb").expect("a task"),
        summary: "Result: three files. Łódź.".to_owned(),
        run_seconds: 42,
        base: Some("0123456789abcdef0123456789abcdef01234567".to_owned()),
        stage,
    }
}

fn checked() -> Checked {
    Checked {
        step: Some("2.1".parse().expect("a step")),
        files: vec!["docs/count.txt".to_owned()],
        head: Some("89abcdef0123456789abcdef0123456789abcdef".to_owned()),
        committed: true,
        outside: Vec::new(),
        found: Some(Found {
            outcome: CheckOutcome::Passed,
            line: "test -s docs/count.txt".to_owned(),
            tail: "3\n".to_owned(),
            reason: None,
            duration_ms: 12,
            output: Some("checks/2.1.log".to_owned()),
        }),
    }
}

fn every_stage() -> Vec<Stage> {
    vec![
        Stage::Ended,
        Stage::Checked(Box::new(checked())),
        Stage::Settled {
            checked: Box::new(checked()),
            settlement: Settlement::Brought {
                commit: "fedcba9876543210fedcba9876543210fedcba98".to_owned(),
            },
        },
        Stage::Settled {
            checked: Box::new(checked()),
            settlement: Settlement::Conflict {
                files: vec!["docs/count.txt".to_owned()],
            },
        },
        Stage::Settled {
            checked: Box::new(checked()),
            settlement: Settlement::Returned {
                reasons: vec!["the check failed".to_owned()],
            },
        },
        Stage::Settled {
            checked: Box::new(checked()),
            settlement: Settlement::Report,
        },
    ]
}

#[tokio::test]
async fn a_request_written_in_any_stage_is_read_back_whole() {
    for stage in every_stage() {
        let dir = tempfile::tempdir().expect("a directory");
        let written = pending(3, stage);

        write_pending(dir.path(), &written).expect("the file is written");
        let read = read_pending(dir.path()).await.expect("the file is read");

        assert_eq!(read, vec![written]);
    }
}

#[tokio::test]
async fn a_request_written_again_replaces_its_file_and_leaves_no_temporary_file() {
    let dir = tempfile::tempdir().expect("a directory");
    write_pending(dir.path(), &pending(3, Stage::Ended)).expect("the first write");
    let later = pending(3, Stage::Checked(Box::new(checked())));

    write_pending(dir.path(), &later).expect("the second write");

    assert_eq!(read_pending(dir.path()).await.expect("read"), vec![later]);
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .expect("the directory")
        .map(|entry| entry.expect("an entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        vec![
            pending_path(dir.path(), request(3))
                .file_name()
                .expect("a name")
                .to_string_lossy()
                .into_owned()
        ]
    );
}

#[tokio::test]
async fn the_requests_of_a_directory_come_oldest_first_and_other_files_are_left_alone() {
    let dir = tempfile::tempdir().expect("a directory");
    write_pending(dir.path(), &pending(12, Stage::Ended)).expect("write");
    write_pending(dir.path(), &pending(4, Stage::Ended)).expect("write");
    std::fs::write(dir.path().join("worker-request-notes.txt"), "not a request").expect("another file");
    std::fs::write(dir.path().join("task.json"), "{}").expect("another file");

    let read = read_pending(dir.path()).await.expect("read");

    assert_eq!(
        read.iter().map(|pending| pending.request).collect::<Vec<_>>(),
        vec![request(4), request(12)]
    );
}

#[tokio::test]
async fn a_directory_that_is_not_there_has_no_requests() {
    let dir = tempfile::tempdir().expect("a directory");

    let read = read_pending(&dir.path().join("task-none")).await.expect("read");

    assert_eq!(read, Vec::<Pending>::new());
}

#[tokio::test]
async fn a_file_that_is_not_a_request_is_an_error_that_names_it() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = pending_path(dir.path(), request(5));
    std::fs::write(&path, r#"{"request":5,"surprise":true}"#).expect("a file");

    let error = read_pending(dir.path()).await.expect_err("the file is refused");

    assert!(
        matches!(&error, PendingError::Parse { path: named, .. } if *named == path),
        "{error:?}"
    );
}

#[test]
fn a_file_cannot_be_written_to_a_directory_that_is_not_there() {
    let dir = tempfile::tempdir().expect("a directory");
    let missing = dir.path().join("task-none");

    let error = write_pending(&missing, &pending(3, Stage::Ended)).expect_err("no directory");

    assert!(matches!(error, PendingError::Write { .. }), "{error:?}");
}

#[test]
fn every_settlement_has_its_word_for_the_log() {
    let words: Vec<&str> = every_stage()
        .into_iter()
        .filter_map(|stage| match stage {
            Stage::Settled { settlement, .. } => Some(settlement.outcome()),
            Stage::Ended | Stage::Checked(_) => None,
        })
        .collect();

    assert_eq!(words, vec!["brought", "conflict", "returned", "report"]);
}

#[tokio::test]
async fn a_file_that_is_not_a_request_does_not_hide_the_requests_beside_it() {
    let dir = tempfile::tempdir().expect("a directory");
    write_pending(dir.path(), &pending(4, Stage::Ended)).expect("write");
    std::fs::write(pending_path(dir.path(), request(6)), "not json").expect("a damaged file");
    write_pending(dir.path(), &pending(9, Stage::Ended)).expect("write");

    let read = read_pending_each(dir.path()).await.expect("the directory is listed");

    assert_eq!(read.len(), 3);
    assert!(
        matches!(&read[0], Ok(pending) if pending.request == request(4)),
        "{read:?}"
    );
    assert!(
        matches!(&read[1], Err(PendingError::Parse { path, .. }) if *path == pending_path(dir.path(), request(6))),
        "{read:?}"
    );
    assert!(
        matches!(&read[2], Ok(pending) if pending.request == request(9)),
        "{read:?}"
    );
}

#[test]
fn the_request_of_a_file_is_told_by_its_name_alone() {
    let named = |name: &str| request_of_file(Path::new("/tasks/t").join(name).as_path());
    assert_eq!(named("worker-request-12.json"), Some(request(12)));
    assert_eq!(named("worker-request-0.json"), None);
    assert_eq!(named("worker-request-x.json"), None);
    assert_eq!(named("worker-request-3.json.damaged"), None);
    assert_eq!(named("other-3.json"), None);
}

#[tokio::test]
async fn a_file_set_aside_is_kept_under_a_name_the_reading_does_not_meet_and_never_over_another() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("worker-request-1.json");
    tokio::fs::write(&path, "first").await.expect("written");

    let first = set_aside(&path).await.expect("set aside");
    tokio::fs::write(&path, "second").await.expect("written");
    let second = set_aside(&path).await.expect("set aside");

    assert_eq!(first, dir.path().join("worker-request-1.json.damaged"));
    assert_eq!(second, dir.path().join("worker-request-1.json.damaged-2"));
    assert_eq!(tokio::fs::read_to_string(&first).await.expect("kept"), "first");
    assert_eq!(tokio::fs::read_to_string(&second).await.expect("kept"), "second");
    assert!(!path.exists());
    assert!(read_pending_each(dir.path()).await.expect("listed").is_empty());
}

#[tokio::test]
async fn a_failure_of_a_file_names_the_file_and_one_of_the_clock_names_none() {
    let dir = tempfile::tempdir().expect("a directory");
    let path = dir.path().join("worker-request-1.json");
    tokio::fs::write(&path, "{").await.expect("written");

    let mut read = read_pending_each(dir.path()).await.expect("listed");
    let error = read.remove(0).expect_err("damaged");

    assert_eq!(error.path(), Some(path.as_path()));
    assert_eq!(
        PendingError::Encode(serde_json::from_str::<u8>("x").expect_err("bad")).path(),
        None
    );
}
