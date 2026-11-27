//! The newest checkpoint of a round for a holder: the holder's own, else the round's, with what it says of the work.

use agent_core::history::CheckpointCause;
use agent_runtime::history::{HistoryDb, LastCheckpoint, Reader};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

use super::support::roomy;

/// The round of these tests.
const ROUND: &str = "round-20261126-100000-r0r0";
/// The holder whose checkpoints these tests ask for.
const HOLDER: &str = "task-20261126-100000-t0t0";
/// Another task of the round.
const OTHER: &str = "task-20261126-100000-t1t1";

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

/// Records a checkpoint of `task` in the round taken at `at` after `cause`, with `extra` as more members of its JSON.
fn checkpoint(path: &Path, task: &str, at: &str, cause: &str, extra: &str) {
    let payload = format!(
        r#"{{"event":"checkpoint","round":"{ROUND}","task":"{task}","job":"item","tier":"normal","cause":"{cause}","draft":false,"phase":"in_progress"{extra}}}"#
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

/// The newest checkpoint of the round for `task`.
fn last(path: &Path, task: &str) -> Option<LastCheckpoint> {
    Reader::open(path)
        .unwrap()
        .last_checkpoint_of_round(&ROUND.parse().unwrap(), &task.parse().unwrap())
        .unwrap()
}

#[test]
fn the_newest_checkpoint_of_the_holder_is_read_with_its_head_snapshot_and_content() {
    let (_dir, path) = database();
    checkpoint(&path, HOLDER, "2026-11-26T09:01:00Z", "round_opened", "");
    checkpoint(
        &path,
        HOLDER,
        "2026-11-26T09:02:00Z",
        "commit",
        r#","head":"0123456789abcdef0123456789abcdef01234567","snapshot":"refs/agents/snapshots/round-20261126-100000-r0r0/task-20261126-100000-t0t0/c2","content":{"meant":"M","done":"D","left":"L"}"#,
    );
    checkpoint(&path, OTHER, "2026-11-26T09:03:00Z", "commit", "");

    let found = last(&path, HOLDER).unwrap();

    assert_eq!(found.cause, CheckpointCause::Commit);
    assert_eq!(found.at.to_string(), "2026-11-26T09:02:00Z");
    assert_eq!(found.task, HOLDER.parse().unwrap());
    assert_eq!(
        found.head.as_ref().map(ToString::to_string).as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
    assert_eq!(
        found.snapshot.as_deref(),
        Some("refs/agents/snapshots/round-20261126-100000-r0r0/task-20261126-100000-t0t0/c2")
    );
    let content = found.content.unwrap();
    assert_eq!(
        (content.meant.as_str(), content.done.as_str(), content.left.as_str()),
        ("M", "D", "L")
    );
}

#[test]
fn a_holder_with_no_checkpoint_of_its_own_gets_the_newest_of_the_round() {
    let (_dir, path) = database();
    checkpoint(&path, OTHER, "2026-11-26T09:01:00Z", "commit", "");
    checkpoint(&path, OTHER, "2026-11-26T09:02:00Z", "review_submitted", "");

    let found = last(&path, HOLDER).unwrap();

    assert_eq!(found.cause, CheckpointCause::ReviewSubmitted);
    assert_eq!(found.task, OTHER.parse().unwrap());
}

#[test]
fn a_checkpoint_that_kept_no_account_of_the_work_has_no_content() {
    let (_dir, path) = database();
    checkpoint(&path, HOLDER, "2026-11-26T09:01:00Z", "commit", "");

    assert_eq!(last(&path, HOLDER).unwrap().content, None);
}

#[test]
fn a_round_with_no_checkpoint_has_none() {
    let (_dir, path) = database();

    assert_eq!(last(&path, HOLDER), None);
}

/// The checkpoint of the round that tells a new holder how the work stands, for `task`.
fn told(path: &Path, task: &str) -> Option<LastCheckpoint> {
    Reader::open(path)
        .unwrap()
        .last_told_checkpoint_of_round(&ROUND.parse().unwrap(), &task.parse().unwrap())
        .unwrap()
}

#[test]
fn a_change_of_holder_is_skipped_for_the_checkpoint_that_tells_how_the_work_stands() {
    let (_dir, path) = database();
    checkpoint(&path, HOLDER, "2026-11-26T09:01:00Z", "round_overdue", "");
    checkpoint(&path, HOLDER, "2026-11-26T09:02:00Z", "holder_changed", "");

    assert_eq!(told(&path, HOLDER).unwrap().cause, CheckpointCause::RoundOverdue);
    assert_eq!(last(&path, HOLDER).unwrap().cause, CheckpointCause::HolderChanged);
}

#[test]
fn a_holder_with_only_a_change_of_holder_is_told_the_newest_other_checkpoint_of_the_round() {
    let (_dir, path) = database();
    checkpoint(&path, OTHER, "2026-11-26T09:01:00Z", "commit", "");
    checkpoint(&path, HOLDER, "2026-11-26T09:02:00Z", "holder_changed", "");

    let found = told(&path, HOLDER).unwrap();

    assert_eq!(found.cause, CheckpointCause::Commit);
    assert_eq!(found.task, OTHER.parse().unwrap());
}

#[test]
fn a_round_with_only_changes_of_holder_tells_no_checkpoint() {
    let (_dir, path) = database();
    checkpoint(&path, HOLDER, "2026-11-26T09:01:00Z", "holder_changed", "");

    assert_eq!(told(&path, HOLDER), None);
}
