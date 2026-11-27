//! The checkpoints of a worker's request, a review and an investigation: read for their own task only, and never taken for
//! the checkpoint of an item.

use agent_core::history::{CheckpointCause, CheckpointJob};
use agent_core::kinds::Tier;
use agent_runtime::history::{HistoryDb, LastCheckpoint, Reader};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use super::support::roomy;

/// The round of these tests.
const ROUND: &str = "round-20261126-100000-r0r0";
/// The task of a worker's request.
const WORKER: &str = "task-20261126-100000-w0w0";
/// The task of an item's holder.
const HOLDER: &str = "task-20261126-100000-t0t0";

/// A history of the current version with no record.
fn database() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.db");
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch(include_str!("../fixtures/history-before-checkpoints.sql"))
        .unwrap();
    drop(raw);
    drop(HistoryDb::open(&path, roomy()).unwrap());
    (dir, path)
}

/// Records a checkpoint of `task` of `job` in the round, taken at `at` after `cause`, with `extra` members of its JSON.
fn checkpoint(path: &Path, (task, job, tier): (&str, &str, &str), at: &str, cause: &str, extra: &str) {
    let payload = format!(
        r#"{{"event":"checkpoint","round":"{ROUND}","task":"{task}","job":"{job}","tier":"{tier}","cause":"{cause}","draft":false,"phase":"in_progress"{extra}}}"#
    );
    let raw = Connection::open(path).unwrap();
    raw.execute(
        "INSERT INTO history_records (kind, key, at, team, payload, cut) VALUES ('checkpoint', NULL, ?1, 'team-20261126-100000-t0t0', ?2, '')",
        rusqlite::params![at, payload],
    )
    .unwrap();
    let sequence = raw.last_insert_rowid();
    raw.execute(
        "INSERT INTO history_record_rounds (sequence, position, round) VALUES (?1, 1, ?2)",
        rusqlite::params![sequence, ROUND],
    )
    .unwrap();
}

/// A checkpoint of the worker's request.
fn worker(path: &Path, at: &str, cause: &str, extra: &str) {
    checkpoint(path, (WORKER, "worker_request", "worker"), at, cause, extra);
}

/// The newest checkpoint of the job of `task`.
fn last_of_job(path: &Path, task: &str) -> Option<LastCheckpoint> {
    Reader::open(path)
        .unwrap()
        .last_checkpoint_of_job(&ROUND.parse().unwrap(), &task.parse().unwrap())
        .unwrap()
}

#[test]
fn the_newest_checkpoint_of_a_job_is_read_with_its_job_tier_and_content() {
    let (_dir, path) = database();
    worker(&path, "2026-11-26T09:01:00Z", "request_started", "");
    worker(
        &path,
        "2026-11-26T09:02:00Z",
        "commit",
        r#","pull_request":7,"review_pass":2,"content":{"meant":"M","done":"D","left":"L"}"#,
    );

    let found = last_of_job(&path, WORKER).unwrap();

    assert_eq!(found.cause, CheckpointCause::Commit);
    assert_eq!(found.job, CheckpointJob::WorkerRequest);
    assert_eq!(found.tier, Tier::Worker);
    assert_eq!(found.review_pass, Some(2));
    assert_eq!(
        found.pull_request.map(|number| number.to_string()).as_deref(),
        Some("7")
    );
    assert_eq!(found.content.unwrap().done, "D");
}

#[test]
fn a_task_with_no_checkpoint_of_its_own_has_none_though_the_round_has_others() {
    let (_dir, path) = database();
    checkpoint(&path, (HOLDER, "item", "normal"), "2026-11-26T09:01:00Z", "commit", "");

    assert_eq!(last_of_job(&path, WORKER), None);
}

#[test]
fn a_change_of_holder_does_not_hide_what_the_job_had_done() {
    let (_dir, path) = database();
    worker(&path, "2026-11-26T09:01:00Z", "commit", "");
    worker(&path, "2026-11-26T09:02:00Z", "holder_changed", "");

    assert_eq!(last_of_job(&path, WORKER).unwrap().cause, CheckpointCause::Commit);
}

#[test]
fn the_checkpoint_of_an_item_is_not_taken_from_a_job_of_the_round() {
    let (_dir, path) = database();
    worker(&path, "2026-11-26T09:03:00Z", "commit", "");

    let found = Reader::open(&path)
        .unwrap()
        .last_checkpoint_of_round(&ROUND.parse().unwrap(), &HOLDER.parse().unwrap())
        .unwrap();

    assert_eq!(found, None);
}

#[test]
fn the_checkpoints_of_the_jobs_of_a_round_are_listed_oldest_first_without_the_items() {
    let (_dir, path) = database();
    checkpoint(&path, (HOLDER, "item", "normal"), "2026-11-26T09:00:00Z", "commit", "");
    worker(&path, "2026-11-26T09:01:00Z", "request_started", "");
    worker(&path, "2026-11-26T09:02:00Z", "commit", "");

    let found = Reader::open(&path)
        .unwrap()
        .job_checkpoints_of_round(&ROUND.parse().unwrap())
        .unwrap();

    let causes: Vec<CheckpointCause> = found.iter().map(|checkpoint| checkpoint.cause).collect();
    assert_eq!(causes, [CheckpointCause::RequestStarted, CheckpointCause::Commit]);
}

#[test]
fn a_checkpoint_written_before_the_jobs_with_no_job_is_the_checkpoint_of_an_item_and_no_job_is_listed() {
    let (_dir, path) = database();
    let payload = format!(
        r#"{{"event":"checkpoint","round":"{ROUND}","task":"{WORKER}","tier":"normal","cause":"commit","draft":false,"phase":"in_progress"}}"#
    );
    let raw = Connection::open(&path).unwrap();
    raw.execute(
        "INSERT INTO history_records (kind, key, at, team, payload, cut) VALUES ('checkpoint', NULL, '2026-11-26T10:00:00Z', 'team-20261126-100000-t0t0', ?1, '')",
        rusqlite::params![payload],
    )
    .unwrap();
    let sequence = raw.last_insert_rowid();
    raw.execute(
        "INSERT INTO history_record_rounds (sequence, position, round) VALUES (?1, 1, ?2)",
        rusqlite::params![sequence, ROUND],
    )
    .unwrap();
    drop(raw);
    let reader = Reader::open(&path).unwrap();
    let round: agent_core::ids::RoundId = ROUND.parse().unwrap();

    let jobs = reader.job_checkpoints_of_round(&round).unwrap();
    let last = reader
        .last_checkpoint_of_round(&round, &WORKER.parse().unwrap())
        .unwrap()
        .unwrap();

    assert_eq!(jobs, Vec::new());
    assert_eq!(last.job, agent_core::history::CheckpointJob::Item);
}
