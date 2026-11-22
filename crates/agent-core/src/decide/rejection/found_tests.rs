//! What the state shows of an item a lead returned, as a table over the situations the daemon meets: the first return, the
//! member started again by the lead's message, the same hand-over returned twice, an item the member holds after another,
//! and the parts of a split item.

use std::num::NonZeroU32;

use super::*;
use crate::state::fixtures::{at, drive, round_id, run, task, team};
use crate::state::split::{PartDefinition, PartKey, SplitPlan};
use crate::state::{NewRun, PhaseKind, RunEnd, Team};
use crate::status::RunOutcome;

/// A run of the member `codex-1` of these tests: its number, the round it works for, when it started and how it ended
/// (`None` while it goes).
struct Work {
    n: u32,
    round: u32,
    started: u32,
    ended: Option<(u32, RunOutcome)>,
}

/// The sample team with item 1.2 in progress in round 1 and item 2.1 in progress in round 2, both held by `codex-1`,
/// whose runs are `runs`.
fn team_with(runs: &[Work]) -> Team {
    let mut team: Team = team(1);
    for (id, item) in [(1, "1.2"), (2, "2.1")] {
        team.open_round(round_id(id), Some(item.parse().unwrap()), at(1))
            .unwrap();
        drive(&mut team, item, PhaseKind::InProgress);
    }
    for work in runs {
        team.begin_run(
            &task(2),
            NewRun {
                id: run(work.n),
                at: at(work.started),
                session: None,
                rounds: vec![round_id(work.round)],
            },
        )
        .unwrap();
        if let Some((end, outcome)) = work.ended {
            let (_, mut member) = team.crew_mut().find_mut(&task(2)).unwrap();
            member.end_run(at(end), RunEnd::Reported { outcome }).unwrap();
        }
    }
    team
}

/// Run `n` of the member for round `round`, which started at second `started` and ended `DONE` at `ended`.
fn done(n: u32, round: u32, started: u32, ended: u32) -> Work {
    Work {
        n,
        round,
        started,
        ended: Some((ended, RunOutcome::Done)),
    }
}

/// The target `item` and `part` name.
fn target(item: &str, part: Option<usize>) -> ReturnTarget {
    ReturnTarget {
        item: item.parse().unwrap(),
        part: part.and_then(PartKey::at_position),
    }
}

/// What a return of `target` by a lead whose run began at second `lead` counts.
fn counted(team: &Team, target: ReturnTarget, lead: u32) -> Option<Rejection> {
    let found = read_return(team, target, at(lead), RunOutcome::Sleeping).expect("the item is in the team");
    classify(&ReturnLine::returned(&target.item()), found.facts())
}

#[test]
fn the_first_return_of_work_the_member_handed_over_counts() {
    let team = team_with(&[done(1, 1, 10, 11)]);

    let found = read_return(&team, target("1.2", None), at(16), RunOutcome::Sleeping).unwrap();

    assert_eq!(counted(&team, target("1.2", None), 16), Some(Rejection::Counted));
    assert_eq!(found.of_run(), Some(&run(1)));
    assert_eq!(found.started_again(), None);
    assert_eq!(found.member().map(|member| member.task().clone()), Some(task(2)));
}

#[test]
fn a_member_started_again_by_the_lead_message_is_seen_and_the_return_still_counts() {
    let team = team_with(&[
        done(1, 1, 10, 11),
        Work {
            n: 2,
            round: 1,
            started: 20,
            ended: None,
        },
    ]);

    let found = read_return(&team, target("1.2", None), at(16), RunOutcome::Sleeping).unwrap();

    assert_eq!(counted(&team, target("1.2", None), 16), Some(Rejection::Counted));
    assert_eq!(
        found.of_run(),
        Some(&run(1)),
        "the work that was read, not the run it started"
    );
    assert_eq!(found.started_again(), NonZeroU32::new(2));
}

#[test]
fn the_same_hand_over_returned_again_by_a_later_lead_run_counts_once() {
    // Done at 11, sent back at 15 and counted when the member started again at 20, which ended DONE at 21; a lead run that
    // began at 16 repeats its line at 25.
    let mut team = team_with(&[done(1, 1, 10, 11), done(2, 1, 20, 21)]);
    let mut item = team.item_mut("1.2".parse().unwrap()).unwrap();
    item.record_rejection_of("fix it".to_owned(), at(15), None, Some(run(1)))
        .unwrap();
    item.next_run_started(NonZeroU32::new(2).unwrap(), at(20)).unwrap();

    let found = read_return(&team, target("1.2", None), at(16), RunOutcome::Sleeping).unwrap();

    assert_eq!(
        found.of_run(),
        Some(&run(1)),
        "the hand-over the lead began its run after"
    );
    assert_eq!(counted(&team, target("1.2", None), 16), Some(Rejection::AlreadyCounted));
    // A lead run that began after the second DONE returns new work.
    assert_eq!(counted(&team, target("1.2", None), 30), Some(Rejection::Counted));
}

#[test]
fn an_item_the_member_holds_after_another_is_not_handed_over_before_its_own_first_run() {
    // `codex-1` ended item 1.2 DONE in round 1 and holds item 2.1 in round 2 with no run for it yet.
    let team = team_with(&[done(1, 1, 10, 11)]);

    assert_eq!(counted(&team, target("2.1", None), 30), Some(Rejection::NotHandedOver));
    assert_eq!(counted(&team, target("1.2", None), 30), Some(Rejection::Counted));
}

#[test]
fn a_line_names_the_part_that_is_open_or_counts_nothing() {
    let mut team = team_with(&[done(1, 1, 10, 11)]);
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

    let table = [
        ("the open part", target("1.2", Some(0)), Some(Rejection::Counted)),
        (
            "the item, which is its open part",
            target("1.2", None),
            Some(Rejection::Counted),
        ),
        (
            "a part that is not open",
            target("1.2", Some(1)),
            Some(Rejection::NotHandedOver),
        ),
    ];
    for (what, target, expected) in table {
        assert_eq!(counted(&team, target, 16), expected, "{what}");
    }
    let found = read_return(&team, target("1.2", None), at(16), RunOutcome::Sleeping).unwrap();
    assert_eq!(found.part(), PartKey::at_position(0));
}

#[test]
fn an_item_the_team_does_not_have_is_not_found() {
    let team = team_with(&[]);

    assert_eq!(
        read_return(&team, target("9.9", None), at(16), RunOutcome::Sleeping),
        None
    );
}
