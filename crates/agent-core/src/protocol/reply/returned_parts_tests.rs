//! `agent status` shows the count of the part the member works on, not the returns of the other parts of the item.

use std::num::NonZeroU32;

use super::*;
use crate::state::fixtures::{at, run, team};
use crate::state::split::{PartDefinition, PartKey, SplitPlan};
use crate::state::{ItemPhase, Timestamp};

/// `seconds` after the fixture's epoch.
fn when(seconds: i64) -> Timestamp {
    let time = at(0)
        .as_utc()
        .checked_add(time::Duration::seconds(seconds))
        .expect("the time of a test fits a timestamp");
    Timestamp::from_utc(time).expect("the time of a test fits a timestamp")
}

/// The sample team with item 1.2 in progress with `codex-1` in a round, split into two parts when `split` is set;
/// `rejections` say for which part (by position, `None` for no part) each counted return was.
fn team_with(split: bool, rejections: &[Option<usize>]) -> crate::state::Team {
    let mut team = team(1);
    team.open_round(
        "round-20261111-153001-k3x9".parse().unwrap(),
        Some("1.2".parse().unwrap()),
        at(1),
    )
    .unwrap();
    let mut item = team.item_mut("1.2".parse().unwrap()).unwrap();
    item.advance(ItemPhase::InProgress {
        assignee: "codex-1".parse().unwrap(),
        branch: "agents/codex-1/item-1-2".parse().unwrap(),
    })
    .unwrap();
    for (index, part) in rejections.iter().enumerate() {
        let n = u32::try_from(index).unwrap();
        item.record_rejection_of(
            format!("reason {n}"),
            when(i64::from(n) * 10 + 1),
            part.and_then(PartKey::at_position),
            Some(run(n + 1)),
        )
        .unwrap();
        item.next_run_started(NonZeroU32::new(n + 2).unwrap(), when(i64::from(n) * 10 + 2))
            .unwrap();
    }
    if split {
        let plan = SplitPlan::new(
            "1.2".parse().unwrap(),
            ["First", "Second"]
                .into_iter()
                .map(|title| PartDefinition {
                    title: title.into(),
                    delivers: "A document".into(),
                    checks: "Read it".into(),
                    depends_on: Vec::new(),
                })
                .collect(),
        )
        .unwrap();
        team.activate_split(plan).unwrap();
    }
    team
}

/// How many returns the summary of `team` says `codex-1` was sent back.
fn counted(team: &crate::state::Team) -> Option<u32> {
    TeamSummary::from(team)
        .members
        .iter()
        .find(|member| member.agent.name() == "codex-1")
        .and_then(|member| member.returned.as_ref())
        .map(|returned| returned.counted.get())
}

#[test]
fn the_member_of_a_part_carries_the_count_of_that_part_only() {
    // Part `a` is the open round's part: two returns of it, one of part `b` and one before the split.
    let team = team_with(true, &[Some(0), Some(0), Some(1), None]);

    assert_eq!(counted(&team), Some(2));
}

#[test]
fn a_part_nobody_returned_has_no_count_whatever_the_others_had() {
    let team = team_with(true, &[Some(1), Some(1), None]);

    assert_eq!(counted(&team), None);
}

#[test]
fn an_item_that_is_not_split_counts_the_returns_of_no_part() {
    let team = team_with(false, &[None, None, Some(0)]);

    assert_eq!(counted(&team), Some(2));
}
