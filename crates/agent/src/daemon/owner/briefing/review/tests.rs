//! What the state says of the pull request of an item and its review, as the briefing of a new session takes it.

use agent_core::local_time::Zone;
use serde_json::{Value, json};
use time::UtcOffset;

use super::*;

/// The settings of a briefing of a team with `limit` fix passes, in UTC.
fn setting(limit: u32, waits_for_human: bool) -> ReviewSetting<'static> {
    ReviewSetting {
        limit,
        zone: &Zone::Fixed(UtcOffset::UTC),
        waits_for_human,
    }
}

/// The pull request of the items of these tests.
fn pull_request() -> Value {
    json!({"number": 12, "branch": "agents/claude-1/task-20261126-171756-y3m2-1.1", "opened_at": "2026-11-26T17:18:30Z"})
}

/// A review that asked for changes.
fn changes(reviewer: &str, task: &str) -> Value {
    json!({
        "reviewer": reviewer,
        "task": task,
        "requested_at": "2026-11-26T17:19:00Z",
        "outcome": {"status": "submitted", "verdict": "REQUEST_CHANGES", "at": "2026-11-26T17:20:00Z"},
    })
}

/// An item in `phase` with `extra` fields of its progress.
fn item(phase: &Value, extra: &Value) -> ItemProgress {
    let mut progress = json!({"id": "1.1", "phase": phase});
    if let (Some(fields), Some(more)) = (progress.as_object_mut(), extra.as_object()) {
        fields.extend(more.clone());
    }
    serde_json::from_value(progress).unwrap()
}

/// An item being fixed in its `passes`-th fix pass.
fn fixing(passes: usize, extra: &Value) -> ItemProgress {
    let pass = json!({"requested_by": [changes("inv-r1", "task-20261126-171800-aaaa")]});
    let mut fields = json!({"fix_passes": vec![pass; passes]});
    if let (Some(fields), Some(more)) = (fields.as_object_mut(), extra.as_object()) {
        fields.extend(more.clone());
    }
    item(
        &json!({"phase": "fixing", "assignee": "claude-1", "branch": "agents/claude-1/task-20261126-171756-y3m2-1.1", "pr": pull_request()}),
        &fields,
    )
}

/// The ledger of the findings of these tests: #1 resolved, #2 open.
fn ledger() -> Value {
    let entry = |number: u32, status: &str| {
        json!({
            "number": number,
            "finding": {"severity": "MAJOR", "rule": "TDD-2", "place": "src/lib.rs:7", "text": "untested", "check": "read it"},
            "reporter": "inv-r1",
            "pass": 1,
            "status": status,
        })
    };
    json!({"findings": {"entries": [entry(1, "resolved"), entry(2, "open")]}})
}

#[test]
fn an_item_that_is_not_in_review_has_no_pull_request_and_no_findings() {
    let started = item(
        &json!({"phase": "in_progress", "assignee": "claude-1", "branch": "agents/claude-1/task-20261126-171756-y3m2-1.1"}),
        &json!({}),
    );

    assert_eq!(
        ReviewFacts::of(Some(&started), setting(5, false)),
        ReviewFacts::default()
    );
    assert_eq!(ReviewFacts::of(None, setting(5, false)), ReviewFacts::default());
}

#[test]
fn an_item_in_review_is_told_with_its_pull_request_and_the_review_pass_after_its_fix_passes() {
    let reviewed = item(
        &json!({"phase": "in_review", "pr": pull_request(), "reviews": []}),
        &json!({"fix_passes": [{"requested_by": [changes("inv-r1", "task-20261126-171800-aaaa")]}], "findings": {"entries": []}}),
    );

    let facts = ReviewFacts::of(Some(&reviewed), setting(5, false));

    let review = facts.as_review();
    assert_eq!(
        review.pull_request.map(|pr| (pr.number.get(), pr.branch, pr.opened)),
        Some((12, "agents/claude-1/task-20261126-171756-y3m2-1.1", "2026-11-26 17:18"))
    );
    assert_eq!(
        review.standing,
        ReviewStanding::InReview {
            pass: NonZeroU32::new(2).unwrap()
        }
    );
    assert_eq!(review.findings, BriefingFindings::Ledger(&[]));
}

#[test]
fn an_item_being_fixed_is_told_its_fix_pass_and_its_limit_and_the_ledger_in_number_order() {
    let fixed = fixing(2, &ledger());

    let facts = ReviewFacts::of(Some(&fixed), setting(5, false));

    let review = facts.as_review();
    assert_eq!(
        review.standing,
        ReviewStanding::Fixing {
            pass: NonZeroU32::new(2).unwrap(),
            limit: 5
        }
    );
    let BriefingFindings::Ledger(entries) = review.findings else {
        panic!("a ledger was expected");
    };
    assert_eq!(
        entries.iter().map(|entry| entry.number.get()).collect::<Vec<_>>(),
        [1, 2]
    );
}

#[test]
fn an_item_whose_lead_was_sent_back_to_its_limit_waits_for_the_human() {
    let fixed = fixing(3, &ledger());

    let facts = ReviewFacts::of(Some(&fixed), setting(3, true));

    assert_eq!(
        facts.as_review().standing,
        ReviewStanding::WaitingForHuman {
            pass: NonZeroU32::new(3).unwrap(),
            limit: 3
        }
    );
}

#[test]
fn an_approved_item_is_told_to_wait_for_the_merge() {
    let approved = item(
        &json!({"phase": "approved", "pr": pull_request(), "reviews": [{
            "reviewer": "inv-r1",
            "task": "task-20261126-171800-aaaa",
            "requested_at": "2026-11-26T17:19:00Z",
            "outcome": {"status": "submitted", "verdict": "APPROVE", "at": "2026-11-26T17:20:00Z"},
        }]}),
        &json!({"findings": {"entries": []}}),
    );

    let facts = ReviewFacts::of(Some(&approved), setting(5, false));

    assert_eq!(facts.as_review().standing, ReviewStanding::Approved);
}

#[test]
fn a_pull_request_of_a_review_that_kept_no_ledger_points_to_its_comments() {
    let legacy = fixing(1, &json!({}));

    let facts = ReviewFacts::of(Some(&legacy), setting(5, false));

    assert_eq!(
        facts.as_review().findings,
        BriefingFindings::Legacy {
            pull_request: NonZeroU32::new(12).unwrap()
        }
    );
}
