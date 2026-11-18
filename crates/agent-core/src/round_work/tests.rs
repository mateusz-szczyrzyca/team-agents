use std::num::NonZeroU32;

use super::*;
use crate::state::{CommitId, PrNumber};

fn number(n: u32) -> PrNumber {
    PrNumber::new(NonZeroU32::new(n).unwrap())
}

fn head() -> CommitId {
    "0123456789abcdef0123456789abcdef01234567".parse().unwrap()
}

fn opened(n: u32) -> HistoryEvent {
    HistoryEvent::PullOpened {
        number: number(n),
        branch: "agents/codex-1/item-1-2".parse().unwrap(),
        head: head(),
        title: "Item 1.2".to_owned(),
        body: "What and why.".to_owned(),
    }
}

fn pushed(n: u32) -> HistoryEvent {
    HistoryEvent::PullPushed {
        pr: number(n),
        head: head(),
    }
}

#[test]
fn a_round_with_no_event_about_a_pull_request_has_no_draft() {
    assert_eq!(draft_pull(&[]), DraftPull::Absent);
    assert_eq!(draft_pull(&[HistoryEvent::TeamEnded {}]), DraftPull::Absent);
}

#[test]
fn a_pull_request_that_was_opened_is_a_draft_until_something_takes_it_out() {
    assert_eq!(draft_pull(&[opened(7)]), DraftPull::Open);
    // A push to the branch of a draft leaves it a draft.
    assert_eq!(draft_pull(&[opened(7), pushed(7)]), DraftPull::Open);
}

#[test]
fn a_pull_request_that_is_ready_approved_merged_handed_over_or_closed_is_no_draft() {
    let after = [
        HistoryEvent::PullReady { pr: number(7) },
        HistoryEvent::PullApproved { pr: number(7) },
        HistoryEvent::PullMerged {
            pr: number(7),
            commit: None,
        },
        HistoryEvent::PullHandedOver { pr: number(7) },
        HistoryEvent::PullClosed { pr: number(7) },
    ];

    for event in after {
        assert_eq!(draft_pull(&[opened(7), event.clone()]), DraftPull::Absent, "{event:?}");
    }
}

#[test]
fn a_pull_request_opened_after_one_was_closed_is_a_draft_again() {
    let events = [opened(7), HistoryEvent::PullClosed { pr: number(7) }, opened(8)];

    assert_eq!(draft_pull(&events), DraftPull::Open);
}

#[test]
fn each_place_of_the_work_has_the_words_the_status_shows() {
    let words: Vec<String> = [
        RoundWork::OnlyLocal,
        RoundWork::PullDraft,
        RoundWork::LocalOrDraft,
        RoundWork::PullReady,
        RoundWork::PullInReview,
        RoundWork::PullApproved,
    ]
    .iter()
    .map(ToString::to_string)
    .collect();

    assert_eq!(
        words,
        [
            "only local",
            "PR draft",
            "only local or PR draft",
            "PR ready",
            "PR in review",
            "PR approved"
        ]
    );
}
