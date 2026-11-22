//! What `agent status` is told of the rejections of an item: the count of the member that holds it, and the limit the
//! daemon knows.

use std::num::NonZeroU32;

use super::*;
use crate::state::fixtures::{at, team};
use crate::state::{ItemPhase, Timestamp};

/// When the rejections of these tests happen: `seconds` after the fixture's epoch.
fn when(seconds: i64) -> Timestamp {
    let time = at(0)
        .as_utc()
        .checked_add(time::Duration::seconds(seconds))
        .expect("the time of a test fits a timestamp");
    Timestamp::from_utc(time).expect("the time of a test fits a timestamp")
}

/// The sample team with item 1.2 in progress with `codex-1`, rejected `counted` times with the member's runs after each
/// of them, and `pending` more times without a run since.
fn team_with_returns(counted: u32, pending: u32) -> crate::state::Team {
    let mut team = team(1);
    let mut item = team.item_mut("1.2".parse().unwrap()).unwrap();
    item.advance(ItemPhase::InProgress {
        assignee: "codex-1".parse().unwrap(),
        branch: "agents/codex-1/item-1-2".parse().unwrap(),
    })
    .unwrap();
    for round in 0..counted {
        item.record_rejection(format!("reason {round}"), when(i64::from(round) * 10 + 1))
            .unwrap();
        item.next_run_started(NonZeroU32::new(round + 2).unwrap(), when(i64::from(round) * 10 + 2))
            .unwrap();
    }
    for round in 0..pending {
        item.record_rejection(format!("pending {round}"), when(1000 + i64::from(round)))
            .unwrap();
    }
    drop(item);
    team
}

/// The `returned` of the member `name` in `summary`.
fn returned_of(summary: &TeamSummary, name: &str) -> Option<Returned> {
    summary
        .members
        .iter()
        .find(|member| member.agent.name() == name)
        .and_then(|member| member.returned.clone())
}

#[test]
fn the_member_that_holds_a_rejected_item_carries_its_count() {
    let team = team_with_returns(3, 0);

    let summary = TeamSummary::from(&team);

    assert_eq!(
        returned_of(&summary, "codex-1"),
        Some(Returned {
            item: "1.2".parse().unwrap(),
            counted: NonZeroU32::new(3).unwrap(),
            limit: None,
        })
    );
    assert_eq!(returned_of(&summary, "inv-opus"), None, "the lead holds no item");
    assert_eq!(returned_of(&summary, "worker-1"), None);
}

#[test]
fn a_return_the_member_has_not_run_after_is_not_counted_yet() {
    let summary = TeamSummary::from(&team_with_returns(0, 2));

    assert_eq!(returned_of(&summary, "codex-1"), None);
}

#[test]
fn an_item_nobody_returned_has_no_count() {
    let summary = TeamSummary::from(&team(1));

    assert!(summary.members.iter().all(|member| member.returned.is_none()));
}

#[test]
fn the_limit_the_daemon_knows_is_told_to_every_count_and_zero_means_never() {
    let summary = TeamSummary::from(&team_with_returns(2, 0));

    let limited = summary.clone().with_rejection_limit(5);
    let never = summary.with_rejection_limit(0);

    assert_eq!(
        returned_of(&limited, "codex-1").map(|returned| returned.limit),
        Some(NonZeroU32::new(5))
    );
    assert_eq!(
        returned_of(&never, "codex-1").map(|returned| returned.limit),
        Some(None)
    );
}

#[test]
fn a_summary_of_an_older_daemon_has_no_count_and_one_without_a_count_writes_none() {
    let summary = TeamSummary::from(&team_with_returns(1, 0));
    let mut value = serde_json::to_value(&summary).unwrap();

    let written = value["members"][1]["returned"].clone();
    value["members"][1].as_object_mut().unwrap().remove("returned");
    let read: TeamSummary = serde_json::from_value(value).unwrap();

    assert_eq!(written["counted"], serde_json::json!(1));
    assert_eq!(written["item"], serde_json::json!("1.2"));
    assert!(read.members.iter().all(|member| member.returned.is_none()));
    let plain = serde_json::to_value(TeamSummary::from(&team(1))).unwrap();
    assert!(plain["members"][1].get("returned").is_none(), "{plain}");
}
