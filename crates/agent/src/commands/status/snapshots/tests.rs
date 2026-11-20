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

#[test]
fn snapshots_of_reads_the_history_or_reports_when_it_cannot_be_read() {
    use agent_core::config::AbsolutePath;
    use agent_core::history::{MaxTextBytes, Record};
    use agent_runtime::history::{HistoryDb, Store};
    use agent_runtime::store::Home;

    use super::snapshots_of;

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let home = Home::new(&AbsolutePath::from_str(root.to_str().unwrap()).unwrap());
    let round = RoundId::from_str("round-20261120-100000-r1r1").unwrap();

    let unreadable = snapshots_of(&home, &round, UtcOffset::UTC);
    assert!(
        unreadable.starts_with("  snapshots: cannot read the history ("),
        "{unreadable:?}"
    );

    let mut db = HistoryDb::open(&home.history_file(), MaxTextBytes::default()).unwrap();
    db.write(&[
        Record::Snapshot(Box::new(snapshot("task-20261120-100000-aaaa", 1, 3_600 + 5, 10))),
        Record::Snapshot(Box::new(snapshot("task-20261120-100000-aaaa", 2, 3_600 + 65, 20))),
    ])
    .unwrap();
    drop(db);

    assert_eq!(
        snapshots_of(&home, &round, UtcOffset::UTC),
        "  snapshot task-20261120-100000-aaaa number 2 at 01:01:05, 20 bytes\n"
    );
}
