use std::str::FromStr;

use agent_core::history::SnapshotRecord;
use agent_core::ids::{RoundId, TaskId};
use agent_core::state::{CommitId, Timestamp};
use time::UtcOffset;

use super::snapshot_lines;

fn snapshot(task: &str, number: u32, seconds: i64, bytes: u64) -> SnapshotRecord {
    SnapshotRecord {
        round: RoundId::from_str("round-20261120-100000-r1r1").unwrap(),
        task: TaskId::from_str(task).unwrap(),
        number,
        at: Timestamp::from_utc(time::UtcDateTime::from_unix_timestamp(seconds).unwrap()).unwrap(),
        commit: CommitId::from_str(&"c".repeat(40)).unwrap(),
        head: CommitId::from_str(&"d".repeat(40)).unwrap(),
        bytes,
    }
}

#[test]
fn a_round_without_snapshots_shows_no_line() {
    assert_eq!(snapshot_lines(&[], UtcOffset::UTC), "");
}

#[test]
fn each_task_shows_its_last_snapshot_with_its_number_time_and_size() {
    let snapshots = [
        snapshot("task-20261120-100000-aaaa", 1, 3_600 + 5, 10),
        snapshot("task-20261120-100000-aaaa", 2, 3_600 + 65, 20),
        snapshot("task-20261120-100000-bbbb", 1, 3_600 + 125, 30),
    ];

    assert_eq!(
        snapshot_lines(&snapshots, UtcOffset::UTC),
        "  snapshot task-20261120-100000-aaaa number 2 at 01:01:05, 20 bytes\n\
         \x20 snapshot task-20261120-100000-bbbb number 1 at 01:02:05, 30 bytes\n"
    );
}
