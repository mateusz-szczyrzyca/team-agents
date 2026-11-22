//! The rejections of an item that is split are the rejections of one part each, and each one keeps the run of the member
//! that was sent back, so that the same hand-over is counted once.

use std::num::NonZeroU32;

use crate::state::fixtures::{at, run};
use crate::state::split::PartKey;
use crate::state::{ItemPhase, ItemProgress, Timestamp};
use crate::task_list::ItemMark;

/// An item a member works on.
fn working() -> ItemProgress {
    let mut progress = ItemProgress::new("1.1".parse().unwrap(), ItemMark::Open);
    progress
        .advance(ItemPhase::InProgress {
            assignee: "codex-1".parse().unwrap(),
            branch: "agents/codex-1/item-1-1".parse().unwrap(),
        })
        .unwrap();
    progress
}

/// `seconds` after the fixture's epoch.
fn when(seconds: i64) -> Timestamp {
    let time = at(0)
        .as_utc()
        .checked_add(time::Duration::seconds(seconds))
        .expect("the time of a test fits a timestamp");
    Timestamp::from_utc(time).expect("the time of a test fits a timestamp")
}

/// The key of the part at `position`.
fn part(position: usize) -> Option<PartKey> {
    PartKey::at_position(position)
}

/// An item with one counted rejection of part `a` (the member's run 1) and one of part `b` (its run 3).
fn rejected_in_two_parts() -> ItemProgress {
    let mut progress = working();
    progress
        .record_rejection_of("a: fix the reader".to_owned(), when(1), part(0), Some(run(1)))
        .unwrap();
    progress.next_run_started(NonZeroU32::new(2).unwrap(), when(2)).unwrap();
    progress
        .record_rejection_of("b: add the tests".to_owned(), when(3), part(1), Some(run(3)))
        .unwrap();
    progress.next_run_started(NonZeroU32::new(4).unwrap(), when(4)).unwrap();
    progress
}

#[test]
fn the_count_of_a_part_leaves_out_the_rejections_of_the_others() {
    let progress = rejected_in_two_parts();

    assert_eq!(progress.counted_rejections_of(part(0), 5).get(), 1);
    assert_eq!(progress.counted_rejections_of(part(1), 5).get(), 1);
    assert_eq!(progress.counted_rejections_of(part(2), 5).get(), 0);
    assert_eq!(progress.counted_rejections_of(None, 5).get(), 0);
    assert_eq!(
        progress.counted_rejections(5).get(),
        2,
        "the whole item counts every part"
    );
}

#[test]
fn the_count_of_an_item_that_is_not_split_counts_the_rejections_of_no_part() {
    let mut progress = working();
    progress.record_rejection("no part".to_owned(), when(1)).unwrap();
    progress.next_run_started(NonZeroU32::new(2).unwrap(), when(2)).unwrap();

    assert_eq!(progress.counted_rejections_of(None, 5).get(), 1);
    assert_eq!(progress.counted_rejections_of(part(0), 5).get(), 0);
}

#[test]
fn a_hand_over_that_was_sent_back_is_remembered_before_and_after_the_member_runs_again() {
    let mut progress = working();
    progress
        .record_rejection_of("fix it".to_owned(), when(1), None, Some(run(1)))
        .unwrap();
    let before = progress.was_rejected(&run(1));
    progress.next_run_started(NonZeroU32::new(2).unwrap(), when(2)).unwrap();

    assert!(before, "while the rejection waits for the member's next run");
    assert!(progress.was_rejected(&run(1)), "once it counts");
    assert!(!progress.was_rejected(&run(2)), "another run is another hand-over");
}

#[test]
fn the_part_and_the_run_are_written_and_a_rejection_written_before_them_reads_without() {
    let progress = rejected_in_two_parts();

    let written = serde_json::to_value(&progress).unwrap();
    let first = &written["rejections"][0];
    let mut old = written.clone();
    for entry in old["rejections"].as_array_mut().unwrap() {
        let entry = entry.as_object_mut().unwrap();
        entry.remove("part");
        entry.remove("of_run");
    }
    let read_old: ItemProgress = serde_json::from_value(old).unwrap();
    let read_back: ItemProgress = serde_json::from_value(written.clone()).unwrap();

    assert_eq!(first["part"], serde_json::json!("a"));
    assert_eq!(first["of_run"], serde_json::json!(run(1)));
    assert_eq!(read_back, progress);
    assert_eq!(read_old.counted_rejections(5).get(), 2);
    assert!(!read_old.was_rejected(&run(1)));
}
