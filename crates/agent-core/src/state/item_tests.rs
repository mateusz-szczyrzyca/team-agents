//! The rejections of an item kept in its progress: the count, why each one came, and the fact the next run of the item
//! was started after it. A state written before this count loads with none.

use std::num::NonZeroU32;

use crate::decide::rejection::Rejections;
use crate::ids::AgentName;
use crate::ids::{BranchName, ItemId, TaskId};
use crate::state::fixtures::{assert_refused, at};
use crate::state::{ItemPhase, ItemProgress, RejectionError, State, Timestamp};
use crate::task_list::ItemMark;

/// The item these tests count the rejections of.
fn item() -> ItemId {
    "1.1".parse().unwrap()
}

/// The member's task, which the lead returns the work to.
fn member_task() -> TaskId {
    "task-20261120-144101-r7q2".parse().unwrap()
}

/// An item a member works on, in progress on `branch`.
fn working() -> ItemProgress {
    let mut progress = ItemProgress::new(item(), ItemMark::Open);
    progress
        .advance(ItemPhase::InProgress {
            assignee: "codex-1".parse::<AgentName>().unwrap(),
            branch: "agents/codex-1/item-1-1".parse::<BranchName>().unwrap(),
        })
        .unwrap();
    progress
}

/// When the rejections of these tests happen: `seconds` after the fixture's epoch.
fn when(seconds: i64) -> Timestamp {
    let time = at(0)
        .as_utc()
        .checked_add(time::Duration::seconds(seconds))
        .expect("the time of a test fits a timestamp");
    Timestamp::from_utc(time).expect("the time of a test fits a timestamp")
}

#[test]
fn a_new_item_has_been_returned_no_time() {
    let progress = working();

    assert_eq!(progress.counted_rejections(5), Rejections::none(5));
}

#[test]
fn a_rejected_item_keeps_the_count_its_reasons_and_the_start_of_its_next_run_and_writes_them() {
    let mut progress = working();

    progress
        .record_rejection("the tests of the reader are missing".to_owned(), when(1))
        .unwrap();
    let next = progress
        .record_rejection("the count is not in the state".to_owned(), when(2))
        .unwrap();
    // The run that closes one rejection stays open until the next rejection or a new run of the item.
    assert_eq!(next, None);
    let started = progress
        .record_rejection("the status line is not shown".to_owned(), when(3))
        .unwrap();

    assert_eq!(started, Some(when(2)));
    let returned = progress.rejections().unwrap();
    assert_eq!(returned.len(), 3);
    assert_eq!(
        returned.first().map(|rejection| rejection.reason()),
        Some("the tests of the reader are missing")
    );
    assert_eq!(
        returned.last().map(|rejection| rejection.reason()),
        Some("the status line is not shown")
    );
    let written: serde_json::Value = serde_json::to_value(&progress).unwrap();
    let entries = written["rejections"].as_array().expect("the rejections are written");
    assert_eq!(entries.len(), 3, "{written}");
    assert_eq!(
        entries.first().map(|entry| entry["reason"].clone()),
        Some(serde_json::json!("the tests of the reader are missing"))
    );
}

#[test]
fn a_rejection_of_an_item_that_left_its_phase_or_has_no_reason_is_refused() {
    let mut pending = ItemProgress::new(item(), ItemMark::Open);
    assert_eq!(
        pending.record_rejection("x".to_owned(), when(1)),
        Err(RejectionError::NotWorkedOn { item: item() })
    );

    let mut progress = working();
    assert_eq!(
        progress.record_rejection("   ".to_owned(), when(1)),
        Err(RejectionError::NoReason { item: item() })
    );
}

#[test]
fn a_rejection_that_names_the_same_time_twice_is_refused() {
    let mut progress = working();
    progress
        .record_rejection("the branch is not gated".to_owned(), when(1))
        .unwrap();

    assert_eq!(
        progress.record_rejection("the branch is not gated".to_owned(), when(1)),
        Err(RejectionError::AlreadyCounted {
            item: item(),
            at: when(1)
        })
    );
}

#[test]
fn a_rejection_whose_next_run_started_before_it_is_refused() {
    let mut progress = working();
    progress.record_rejection("one".to_owned(), when(2)).unwrap();
    progress.record_rejection("two".to_owned(), when(3)).unwrap();

    assert_eq!(
        progress.next_run_started(NonZeroU32::new(2).unwrap(), when(1)),
        Err(RejectionError::RunBeforeRejection {
            item: item(),
            run_started_at: when(1),
            counted: when(2),
        })
    );
}

#[test]
fn a_rejection_is_counted_when_the_next_run_of_the_item_starts_and_only_once() {
    let mut progress = working();
    progress
        .record_rejection("the count is not saved".to_owned(), when(1))
        .unwrap();
    progress
        .record_rejection("the count is not shown".to_owned(), when(2))
        .unwrap();

    assert_eq!(progress.counted_rejections(5), Rejections::none(5));
    assert_eq!(progress.next_run_started(NonZeroU32::new(37).unwrap(), when(2)), Ok(2));
    assert_eq!(progress.counted_rejections(5).get(), 2);
    // The same run does not close a rejection twice.
    assert_eq!(progress.next_run_started(NonZeroU32::new(37).unwrap(), when(3)), Ok(0));
}

#[test]
fn a_fresh_start_clears_the_reasons_of_the_earlier_branch_and_keeps_the_count() {
    let mut progress = working();
    progress
        .record_rejection("the reader is wrong".to_owned(), when(1))
        .unwrap();
    progress.next_run_started(NonZeroU32::new(2).unwrap(), when(1)).unwrap();

    progress.clear_rejections();

    assert_eq!(progress.rejections(), Some(&[][..]));
    assert_eq!(progress.counted_rejections(5).get(), 1);
}

#[test]
fn an_item_whose_first_reason_is_counted_is_refused_when_read() {
    let mut written = serde_json::to_value(working()).unwrap();
    written["rejections"] = serde_json::json!([{"at": "2026-11-22T00:00:01Z", "reason": "x", "counted_in": 37}]);

    assert_refused::<ItemProgress>(&written);
}

#[test]
fn a_state_written_before_the_count_loads_and_counts_from_zero() {
    let state = State::from_json(include_str!("../../tests/fixtures/state-before-renewals.json"))
        .expect("a state of main loads");
    let found = state
        .find_task(&member_task())
        .expect("the task of the fixture is in the state");
    let team_id = found.team.id().clone();
    let team = state.team(&team_id).expect("the team of the fixture is in the state");

    assert!(!team.items().is_empty(), "the fixture has an item in progress");
    for progress in team.items() {
        assert_eq!(progress.counted_rejections(5), Rejections::none(5));
        assert_eq!(progress.rejections(), None);
    }
}
